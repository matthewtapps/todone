//! `todone timesheet export|import`: read and fill the timesheet from the CLI.

use std::{
    collections::BTreeMap,
    fs, io,
    path::Path,
};

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::timesheet;

const USAGE: &str = "usage: todone timesheet export YYYY-MM\n       todone timesheet import FILE|- [--dry-run] [--force]";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    #[serde(default)]
    pub xero_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xero_project: Option<String>,
    pub task_id: String,
    pub desc: String,
    pub minutes: u32,
    #[serde(default)]
    pub synced: Option<String>,
    #[serde(default)]
    pub imported: Option<String>,
    // Fields the web app adds later must survive a CLI write.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub name: String,
    pub project_name: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub date: NaiveDate,
    pub task: String,
    #[serde(default)]
    pub project: Option<String>,
    pub minutes: i64,
    #[serde(default)]
    pub desc: String,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Added,
    Updated,
    Removed,
    Unchanged,
    Skipped(&'static str),
}

/// Must match `snap` in web/src/lib/types.ts byte for byte.
pub fn snap(r: &Row) -> String {
    serde_json::to_string(&(&r.task_id, &r.desc, r.minutes)).expect("strings and ints serialise")
}

fn new_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS randomness");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

pub fn resolve<'a>(tasks: &'a [Task], shown: &[String], i: &Input) -> Result<&'a Task> {
    let hits: Vec<&Task> = tasks
        .iter()
        .filter(|t| shown.contains(&t.id) && t.name == i.task)
        .filter(|t| i.project.as_deref().is_none_or(|p| t.project_name.starts_with(p)))
        .collect();
    match hits.as_slice() {
        [t] => Ok(t),
        [] => bail!("{}: no task named {:?} is shown in the grid", i.date, i.task),
        many => bail!(
            "{}: task {:?} is in more than one project ({}); add \"project\"",
            i.date,
            i.task,
            many.iter().map(|t| t.project_name.as_str()).collect::<Vec<_>>().join(", ")
        ),
    }
}

pub fn apply(day: &mut Vec<Row>, task_id: &str, minutes: u32, desc: &str, force: bool) -> Outcome {
    let found: Vec<usize> = day
        .iter()
        .enumerate()
        .filter(|(_, r)| r.task_id == task_id && r.minutes > 0)
        .map(|(i, _)| i)
        .collect();
    let i = match found.as_slice() {
        [] if minutes == 0 => return Outcome::Unchanged,
        [] => {
            let mut row = Row {
                id: new_id(),
                xero_id: None,
                xero_project: None,
                task_id: task_id.into(),
                desc: desc.into(),
                minutes,
                synced: None,
                imported: None,
                extra: Map::new(),
            };
            row.imported = Some(snap(&row));
            day.push(row);
            return Outcome::Added;
        }
        [i] => *i,
        _ => return Outcome::Skipped("more than one row for this task"),
    };
    let row = &mut day[i];
    let untouched = row.imported.as_deref() == Some(snap(row).as_str());
    if !untouched && !force {
        return Outcome::Skipped(if row.imported.is_some() { "edited since import" } else { "made by you" });
    }
    if minutes == 0 {
        // A pushed row at zero minutes is how the web app asks Push to delete it.
        if row.xero_id.is_some() {
            row.minutes = 0;
            row.imported = None;
        } else {
            day.remove(i);
        }
        return Outcome::Removed;
    }
    row.minutes = minutes;
    row.desc = desc.into();
    row.imported = Some(snap(row));
    Outcome::Updated
}

fn load_tasks(dir: &Path) -> Result<(Vec<Task>, Vec<String>)> {
    let (_, mut doc) = timesheet::read(&timesheet::settings_path(dir))?;
    let tasks: Vec<Task> = serde_json::from_value(doc.remove("tasks").unwrap_or(json!([])))?;
    let shown: Vec<String> = serde_json::from_value(doc.remove("shown").unwrap_or(json!([])))?;
    Ok((tasks, shown))
}

type Rows = BTreeMap<String, Vec<Row>>;

fn take_rows(doc: &mut Map<String, Value>) -> Result<Rows> {
    Ok(serde_json::from_value(doc.remove("rows").unwrap_or(json!({})))?)
}

pub fn import(dir: &Path, inputs: &[Input], dry_run: bool, force: bool) -> Result<Vec<String>> {
    let (tasks, shown) = load_tasks(dir)?;
    if tasks.is_empty() {
        bail!("open the timesheet page once so todone knows your Xero tasks");
    }
    let mut months: BTreeMap<String, Vec<(&Input, &Task, u32)>> = BTreeMap::new();
    for i in inputs {
        let minutes = u32::try_from(i.minutes)
            .with_context(|| format!("{} {}: minutes must be 0 or more", i.date, i.task))?;
        let task = resolve(&tasks, &shown, i)?;
        months.entry(i.date.format("%Y-%m").to_string()).or_default().push((i, task, minutes));
    }
    let mut report = vec![];
    for (month, items) in months {
        let path = timesheet::month_path(dir, &month)?;
        let mut merge = |doc: &mut Map<String, Value>| -> Result<()> {
            let mut rows = take_rows(doc)?;
            for (i, task, minutes) in &items {
                let day = rows.entry(i.date.to_string()).or_default();
                let outcome = apply(day, &task.id, *minutes, &i.desc, force);
                report.push(format!("{} {} / {}: {outcome:?}", i.date, task.project_name, task.name));
            }
            rows.retain(|_, day| !day.is_empty());
            doc.insert("rows".into(), serde_json::to_value(rows)?);
            Ok(())
        };
        if dry_run {
            merge(&mut timesheet::read(&path)?.1)?;
        } else {
            timesheet::modify(&path, merge)?;
        }
    }
    Ok(report)
}

pub fn export(dir: &Path, month: &str) -> Result<Value> {
    let (tasks, shown) = load_tasks(dir)?;
    let name = |id: &str| {
        tasks.iter().find(|t| t.id == id).map_or(("?".into(), id.to_string()), |t| (t.project_name.clone(), t.name.clone()))
    };
    let (_, mut doc) = timesheet::read(&timesheet::month_path(dir, month)?)?;
    let rows: Vec<Value> = take_rows(&mut doc)?
        .into_iter()
        .flat_map(|(date, day)| day.into_iter().map(move |r| (date.clone(), r)))
        .map(|(date, r)| {
            let (project, task) = name(&r.task_id);
            json!({
                "date": date, "project": project, "task": task, "minutes": r.minutes,
                "desc": r.desc, "pushed": r.xero_id.is_some(), "imported": r.imported.is_some(),
                "edited": r.imported.as_deref().is_some_and(|s| s != snap(&r)),
            })
        })
        .collect();
    let shown_tasks: Vec<Value> = tasks
        .iter()
        .filter(|t| shown.contains(&t.id))
        .map(|t| json!({ "project": t.project_name, "task": t.name }))
        .collect();
    Ok(json!({ "tasks": shown_tasks, "rows": rows }))
}

pub fn run(args: &[String]) -> Result<()> {
    let dir = timesheet::dir()?;
    match args {
        [cmd, month] if cmd == "export" => {
            println!("{}", serde_json::to_string_pretty(&export(&dir, month)?)?);
        }
        [cmd, rest @ ..] if cmd == "import" => {
            let (mut file, mut dry_run, mut force) = (None, false, false);
            for a in rest {
                match a.as_str() {
                    "--dry-run" => dry_run = true,
                    "--force" => force = true,
                    f if file.is_none() => file = Some(f),
                    _ => bail!(USAGE),
                }
            }
            let text = match file {
                Some("-") => io::read_to_string(io::stdin())?,
                Some(p) => fs::read_to_string(p).with_context(|| format!("reading {p}"))?,
                None => bail!(USAGE),
            };
            let inputs: Vec<Input> = serde_json::from_str(&text).context("parsing import rows")?;
            for line in import(&dir, &inputs, dry_run, force)? {
                println!("{line}");
            }
        }
        _ => bail!(USAGE),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "todone-timesheet-cli-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn row(task: &str, minutes: u32, desc: &str) -> Row {
        Row {
            id: "r".into(),
            xero_id: None,
            xero_project: None,
            task_id: task.into(),
            desc: desc.into(),
            minutes,
            synced: None,
            imported: None,
            extra: Map::new(),
        }
    }

    fn imported(mut r: Row) -> Row {
        r.imported = Some(snap(&r));
        r
    }

    fn task(id: &str, name: &str, project: &str) -> Task {
        Task { id: id.into(), name: name.into(), project_name: project.into() }
    }

    fn input(task: &str, project: Option<&str>) -> Input {
        Input {
            date: "2026-10-30".parse().unwrap(),
            task: task.into(),
            project: project.map(Into::into),
            minutes: 30,
            desc: String::new(),
        }
    }

    #[test]
    fn snap_matches_javascript_json_stringify() {
        // JSON.stringify(["t", "a \"b\"\nc é/", 30]) in Node 24
        let r = row("t", 30, "a \"b\"\nc é/");
        assert_eq!(snap(&r), r#"["t","a \"b\"\nc é/",30]"#);
    }

    #[test]
    fn rows_without_newer_fields_round_trip_with_extras() {
        let v = json!({"id": "1", "xeroId": null, "taskId": "t", "desc": "d",
                       "minutes": 15, "synced": null, "futureField": [1]});
        let r: Row = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(r.imported, None);
        let back = serde_json::to_value(&r).unwrap();
        assert_eq!(back["futureField"], json!([1]));
        assert_eq!(back["imported"], Value::Null);
    }

    #[test]
    fn adds_a_row_marked_as_imported() {
        let mut day = vec![];
        assert_eq!(apply(&mut day, "t", 60, "work", false), Outcome::Added);
        assert_eq!(day[0].imported.as_deref(), Some(r#"["t","work",60]"#));
        assert_eq!(day[0].xero_id, None);
    }

    #[test]
    fn zero_minutes_with_no_row_does_nothing() {
        let mut day = vec![];
        assert_eq!(apply(&mut day, "t", 0, "", false), Outcome::Unchanged);
        assert!(day.is_empty());
    }

    #[test]
    fn updates_an_untouched_import_in_place() {
        let mut pushed = imported(row("t", 30, "old"));
        pushed.xero_id = Some("X1".into());
        pushed.synced = Some(snap(&pushed));
        let mut day = vec![pushed];
        assert_eq!(apply(&mut day, "t", 45, "new", false), Outcome::Updated);
        assert_eq!((day[0].minutes, day[0].desc.as_str()), (45, "new"));
        assert_eq!(day[0].xero_id.as_deref(), Some("X1"));
        assert_eq!(day[0].synced.as_deref(), Some(r#"["t","old",30]"#));
    }

    #[test]
    fn skips_rows_the_user_made_or_edited() {
        let mut day = vec![row("t", 30, "mine")];
        assert_eq!(apply(&mut day, "t", 45, "new", false), Outcome::Skipped("made by you"));
        let mut edited = imported(row("t", 30, "old"));
        edited.minutes = 60;
        let mut day = vec![edited];
        assert_eq!(apply(&mut day, "t", 45, "new", false), Outcome::Skipped("edited since import"));
        assert_eq!(day[0].minutes, 60);
    }

    #[test]
    fn force_updates_a_user_row() {
        let mut day = vec![row("t", 30, "mine")];
        assert_eq!(apply(&mut day, "t", 45, "new", true), Outcome::Updated);
        assert_eq!(day[0].minutes, 45);
    }

    #[test]
    fn zero_minutes_zeroes_pushed_rows_and_drops_unpushed_ones() {
        let mut pushed = imported(row("t", 30, "d"));
        pushed.xero_id = Some("X1".into());
        let mut day = vec![pushed, imported(row("u", 30, "d"))];
        assert_eq!(apply(&mut day, "t", 0, "", false), Outcome::Removed);
        assert_eq!(apply(&mut day, "u", 0, "", false), Outcome::Removed);
        assert_eq!(day.len(), 1);
        assert_eq!((day[0].task_id.as_str(), day[0].minutes), ("t", 0));
    }

    #[test]
    fn two_rows_for_one_task_are_left_alone() {
        let mut day = vec![imported(row("t", 30, "a")), imported(row("t", 15, "b"))];
        assert_eq!(apply(&mut day, "t", 45, "new", false), Outcome::Skipped("more than one row for this task"));
    }

    #[test]
    fn resolve_needs_a_unique_shown_task() {
        let tasks = vec![
            task("lw-m", "LW", "FY27 - MARVEL Project (WO number TBD)"),
            task("lw-h", "LW", "H20001190"),
            task("sa", "Systems Administration", "GENERAL"),
            task("hidden", "Non-billable", "GENERAL"),
        ];
        let shown: Vec<String> = ["lw-m", "lw-h", "sa"].map(String::from).to_vec();
        assert_eq!(resolve(&tasks, &shown, &input("Systems Administration", None)).unwrap().id, "sa");
        assert!(resolve(&tasks, &shown, &input("LW", None)).unwrap_err().to_string().contains("more than one project"));
        assert_eq!(resolve(&tasks, &shown, &input("LW", Some("FY27"))).unwrap().id, "lw-m");
        assert!(resolve(&tasks, &shown, &input("Nope", None)).is_err());
        assert!(resolve(&tasks, &shown, &input("Non-billable", None)).is_err());
    }

    fn setup() -> PathBuf {
        let dir = tempdir();
        let settings = json!({"tasks": [{"id": "sa", "name": "Systems Administration",
            "projectId": "g", "projectName": "GENERAL"}], "shown": ["sa"]});
        timesheet::modify(&timesheet::settings_path(&dir), |d| {
            *d = settings.as_object().unwrap().clone();
            Ok(())
        })
        .unwrap();
        dir
    }

    fn inputs(v: Value) -> Vec<Input> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn import_writes_each_month_and_dry_run_writes_nothing() {
        let dir = setup();
        let rows = inputs(json!([
            {"date": "2026-10-30", "task": "Systems Administration", "minutes": 60, "desc": "a"},
            {"date": "2026-11-02", "task": "Systems Administration", "minutes": 30, "desc": "b"}
        ]));
        let plan = import(&dir, &rows, true, false).unwrap();
        assert_eq!(plan.len(), 2);
        assert!(!timesheet::month_path(&dir, "2026-10").unwrap().exists());

        import(&dir, &rows, false, false).unwrap();
        let (v, oct) = timesheet::read(&timesheet::month_path(&dir, "2026-10").unwrap()).unwrap();
        assert_eq!(v, 1);
        assert_eq!(oct["rows"]["2026-10-30"][0]["minutes"], json!(60));
        let (_, nov) = timesheet::read(&timesheet::month_path(&dir, "2026-11").unwrap()).unwrap();
        assert_eq!(nov["rows"]["2026-11-02"][0]["desc"], json!("b"));
    }

    #[test]
    fn invalid_input_aborts_before_any_write() {
        let dir = setup();
        let rows = inputs(json!([
            {"date": "2026-10-30", "task": "Systems Administration", "minutes": 60, "desc": "a"},
            {"date": "2026-10-31", "task": "Systems Administration", "minutes": -5, "desc": "b"}
        ]));
        assert!(import(&dir, &rows, false, false).is_err());
        assert!(!timesheet::month_path(&dir, "2026-10").unwrap().exists());
    }

    #[test]
    fn import_without_tasks_says_to_open_the_page() {
        let err = import(&tempdir(), &[], false, false).unwrap_err();
        assert!(err.to_string().contains("open the timesheet page once so todone knows your Xero tasks"));
    }
}
