import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import {
  AlertTriangleIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
  CloudDownloadIcon,
  CloudUploadIcon,
  EllipsisIcon,
  ListPlusIcon,
  PlugIcon,
  SparklesIcon,
} from "lucide-react"
import { toast } from "sonner"

import { AddTask } from "@/components/add-task"
import { BillableSummary } from "@/components/billable-summary"
import { DaySidebar, type GitlabState } from "@/components/day-sidebar"
import {
  ConnectDialog,
  KeywordsDialog,
  SettingsDialog,
} from "@/components/dialogs"
import {
  TimesheetGrid,
  type Column,
  type Selection,
} from "@/components/timesheet-grid"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Separator } from "@/components/ui/separator"
import {
  SidebarInset,
  SidebarProvider,
  SidebarTrigger,
} from "@/components/ui/sidebar"
import { Skeleton } from "@/components/ui/skeleton"
import { Spinner } from "@/components/ui/spinner"
import { Toaster } from "@/components/ui/sonner"
import {
  ApiError,
  createEntry,
  fetchEntries,
  fetchGitlab,
  fetchTasks,
  fetchTodone,
  getSession,
  removeEntry,
  setReauth,
  updateEntry,
} from "@/lib/api"
import { draftDay } from "@/lib/draft"
import { mergeDay } from "@/lib/sync"
import { load, monthKey, save, useStored } from "@/lib/storage"
import {
  fmt,
  isWeekend,
  monthDates,
  monthLabel,
  shiftMonth,
  thisMonth,
  today,
  weekday,
} from "@/lib/time"
import {
  isDirty,
  isPending,
  newRow,
  snap,
  type Row,
  type Session,
  type Settings,
  type Task,
  type TodoneDay,
} from "@/lib/types"

type Rows = Record<string, Row[]>

function useQueryParam(name: string, fallback: string) {
  const [value, setValue] = useState(
    () => new URLSearchParams(location.search).get(name) ?? fallback
  )
  useEffect(() => {
    const url = new URL(location.href)
    url.searchParams.set(name, value)
    history.replaceState(null, "", url)
  }, [name, value])
  return [value, setValue] as const
}

const loadRows = (month: string): Rows => {
  const rows = load<{ rows: Rows }>(monthKey(month), { rows: {} }).rows
  return Object.fromEntries(
    Object.entries(rows).map(([d, list]) => [d, list.filter((r) => r.taskId)])
  )
}

/** True once `on` has held for `delay` ms, so quick loads never flash a skeleton. */
function useDelayed(on: boolean, delay = 200) {
  const [shown, setShown] = useState(false)
  useEffect(() => {
    if (!on) return
    const t = setTimeout(() => setShown(true), delay)
    return () => {
      clearTimeout(t)
      setShown(false)
    }
  }, [on, delay])
  return on && shown
}

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e))

export default function App() {
  const [month, setMonth] = useQueryParam("month", thisMonth())
  const dates = useMemo(() => monthDates(month), [month])

  const [tasks, setTasks] = useStored<Task[]>("timesheet:tasks", [])
  const [shown, setShown] = useStored<string[]>("timesheet:shown", [])
  const [keywords, setKeywords] = useStored<Record<string, string>>(
    "timesheet:keywords",
    {}
  )
  // Unset tasks count as billable unless Xero names them non-billable.
  const [billable, setBillable] = useStored<Record<string, boolean>>(
    "timesheet:billable",
    {}
  )
  const isBillable = useCallback(
    (t: Task) => billable[t.id] ?? !/non-?billable/i.test(t.name),
    [billable]
  )
  const [settings, setSettings] = useStored<Settings>("timesheet:settings", {
    hoursPerDay: 8,
    ignoreEvents: "private appointment",
  })

  // Rows are per month; switching months swaps them during render so a save never lands on the wrong key.
  const [monthRows, setMonthRows] = useState(() => ({
    month,
    rows: loadRows(month),
  }))
  const current =
    monthRows.month === month ? monthRows : { month, rows: loadRows(month) }
  if (current !== monthRows) setMonthRows(current)
  const rows = current.rows
  useEffect(
    () => save(monthKey(monthRows.month), { rows: monthRows.rows }),
    [monthRows]
  )
  const setRows = useCallback(
    (f: (r: Rows) => Rows) => setMonthRows((s) => ({ ...s, rows: f(s.rows) })),
    []
  )

  const [reload, setReload] = useState(0)
  const todoneKey = `${month}#${reload}`
  const [fetched, setFetched] = useState<{
    key: string
    data?: Record<string, TodoneDay>
    error?: string
  }>({
    key: "",
  })
  const todone: { data?: Record<string, TodoneDay>; error?: string } =
    fetched.key === todoneKey ? fetched : {}
  useEffect(() => {
    let live = true
    fetchTodone(dates[0], dates[dates.length - 1]).then(
      (data) => live && setFetched({ key: todoneKey, data }),
      (e) => live && setFetched({ key: todoneKey, error: errorText(e) })
    )
    return () => {
      live = false
    }
  }, [todoneKey, dates])

  const [session, setSessionState] = useState<Session | null>(null)
  const sessionRef = useRef<Session | null>(null)
  const setSession = useCallback((s: Session) => {
    sessionRef.current = s
    setSessionState(s)
  }, [])
  const [now, setNow] = useState(() => Date.now())
  const live = !!session && session.exp * 1000 > now
  const assigned = useRef<string[]>([])
  const [booting, setBooting] = useState(true)

  const addTasks = useCallback(
    (list: Task[]) =>
      setTasks((prev) => {
        const byId = new Map(prev.map((t) => [t.id, t]))
        for (const t of list) byId.set(t.id, t)
        return [...byId.values()]
      }),
    [setTasks]
  )

  const loadAssigned = useCallback(async () => {
    const { tasks, projectIds } = await fetchTasks()
    assigned.current = projectIds
    addTasks(tasks)
  }, [addTasks])

  useEffect(() => {
    getSession()
      .then(async (s) => {
        setSession(s)
        await loadAssigned()
      })
      .catch((e) => {
        if (!(e instanceof ApiError && e.status === 401))
          toast.error("Couldn't load Xero tasks.", {
            description: errorText(e),
          })
      })
      .finally(() => setBooting(false))
  }, [loadAssigned, setSession])

  // A 401 from Xero opens the connect dialog; the request retries once you paste a fresh cURL.
  const [connectOpen, setConnectOpen] = useState(false)
  const waiter = useRef<{
    resolve: () => void
    reject: (e: Error) => void
  } | null>(null)
  const requestAuth = useCallback(
    () =>
      new Promise<void>((resolve, reject) => {
        waiter.current = { resolve, reject }
        setConnectOpen(true)
      }),
    []
  )
  useEffect(() => setReauth(requestAuth), [requestAuth])
  const ensureSession = async () => {
    const s = sessionRef.current
    if (s && s.exp * 1000 > Date.now()) return s
    await requestAuth()
    return sessionRef.current!
  }
  const connected = useCallback(
    (s: Session) => {
      save("timesheet:shortcode", s.shortcode)
      setSession(s)
      setConnectOpen(false)
      toast.success("Connected to Xero.")
      if (waiter.current) {
        waiter.current.resolve()
        waiter.current = null
      } else if (!assigned.current.length) {
        loadAssigned().catch((e) =>
          toast.error("Couldn't load Xero tasks.", {
            description: errorText(e),
          })
        )
      }
    },
    [setSession, loadAssigned]
  )

  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 15000)
    return () => clearInterval(t)
  }, [])
  const onConnectOpenChange = (open: boolean) => {
    setConnectOpen(open)
    if (!open && waiter.current) {
      waiter.current.reject(
        new Error("Xero isn't connected. Connect, then try again.")
      )
      waiter.current = null
    }
  }

  const defaultDay = dates.includes(today())
    ? today()
    : (dates.find((d) => !isWeekend(d)) ?? dates[0])
  const [dayParam, setDayParam] = useQueryParam("day", defaultDay)
  const [sel, setSel] = useState<Selection>({
    date: dates.includes(dayParam) ? dayParam : defaultDay,
    taskId: null,
    rowId: null,
  })
  if (!dates.includes(sel.date))
    setSel({ date: defaultDay, taskId: null, rowId: null })
  useEffect(() => setDayParam(sel.date), [sel.date, setDayParam])

  // GitLab activity loads per selected day and is kept for the session; no entry means loading.
  const [gitlab, setGitlab] = useState<Record<string, GitlabState>>({})
  const needsGitlab = !gitlab[sel.date]
  useEffect(() => {
    if (!needsGitlab) return
    const date = sel.date
    let live = true
    const done = (state: GitlabState) =>
      live && setGitlab((g) => ({ ...g, [date]: state }))
    fetchGitlab(date).then(
      (data) => done({ status: "ready", data }),
      (e) =>
        done({
          status: "error",
          message: `Couldn't load GitLab activity. ${errorText(e)}`,
        })
    )
    return () => {
      live = false
    }
  }, [needsGitlab, sel.date])

  const hasTime = useCallback(
    (id: string) =>
      Object.values(rows).some((list) =>
        list.some((r) => r.taskId === id && (r.minutes || r.xeroId))
      ),
    [rows]
  )
  const visible = useMemo(
    () =>
      tasks
        .filter((t) => shown.includes(t.id) || hasTime(t.id))
        .sort(
          (a, b) =>
            a.projectName.localeCompare(b.projectName) ||
            a.name.localeCompare(b.name)
        ),
    [tasks, shown, hasTime]
  )
  const hidden = useMemo(
    () => tasks.filter((t) => !visible.includes(t)),
    [tasks, visible]
  )

  const columns = useMemo(() => {
    const cols: Column[] = []
    let week: string[] = []
    dates.forEach((d, i) => {
      if (!isWeekend(d) || todone.data?.[d]?.did.length || rows[d]?.length) {
        cols.push({ date: d })
        week.push(d)
      }
      if (weekday(d) === 0 || i === dates.length - 1) {
        if (week.length) cols.push({ week })
        week = []
      }
    })
    return cols
  }, [dates, todone.data, rows])

  const target = (d: string) =>
    isWeekend(d) ? 0 : Math.round(settings.hoursPerDay * 60)
  const gap = (d: string) => {
    const total = (rows[d] ?? []).reduce((a, r) => a + r.minutes, 0)
    return { total, gap: target(d) - total }
  }
  const monthTotal = dates.reduce((a, d) => a + gap(d).total, 0)
  const pending = Object.values(rows).flat().filter(isPending).length
  const taskById = new Map(tasks.map((t) => [t.id, t]))
  const billableMinutes = dates
    .flatMap((d) => rows[d] ?? [])
    .reduce((a, r) => {
      const t = taskById.get(r.taskId)
      return a + (!t || isBillable(t) ? r.minutes : 0)
    }, 0)

  useEffect(() => {
    document.title = `Timesheet · ${monthLabel(month)}`
  }, [month])

  const onMinutes = useCallback(
    (date: string, taskId: string, rowId: string | null, minutes: number) => {
      if (rowId) {
        setRows((r) => ({
          ...r,
          [date]: (r[date] ?? []).map((x) =>
            x.id === rowId ? { ...x, minutes } : x
          ),
        }))
        return
      }
      if (!minutes) return
      const row = { ...newRow(taskId), minutes }
      setRows((r) => ({ ...r, [date]: [...(r[date] ?? []), row] }))
      setSel((s) =>
        s.date === date && s.taskId === taskId && !s.rowId
          ? { ...s, rowId: row.id }
          : s
      )
    },
    [setRows]
  )

  const selRow = sel.rowId
    ? (rows[sel.date] ?? []).find((r) => r.id === sel.rowId)
    : undefined
  const onDesc = (desc: string) => {
    if (!sel.taskId) return
    if (sel.rowId) {
      setRows((r) => ({
        ...r,
        [sel.date]: (r[sel.date] ?? []).map((x) =>
          x.id === sel.rowId ? { ...x, desc } : x
        ),
      }))
      return
    }
    const row = { ...newRow(sel.taskId), desc }
    setRows((r) => ({ ...r, [sel.date]: [...(r[sel.date] ?? []), row] }))
    setSel((s) => ({ ...s, rowId: row.id }))
  }

  const [confirm, setConfirm] = useState<{
    date: string
    draft: Row[]
    count: number
  } | null>(null)
  const applyDraft = (date: string, draft: Row[]) => {
    setRows((r) => ({
      ...r,
      [date]: (r[date] ?? []).filter((x) => x.xeroId).concat(draft),
    }))
    setSel((s) => ({ ...s, rowId: null }))
    toast.success(
      `Drafted ${draft.length} ${draft.length === 1 ? "task" : "tasks"}.`
    )
  }
  const noMatch = () =>
    toast.info("Nothing matched a task's draft keywords.", {
      description: "Add keywords from a task's menu, then draft again.",
    })
  const onDraftDay = () => {
    const draft = draftDay(
      todone.data?.[sel.date],
      target(sel.date),
      settings.ignoreEvents,
      tasks,
      keywords
    )
    if (!draft.length) return noMatch()
    const count = (rows[sel.date] ?? []).filter(
      (r) => !r.xeroId && (r.minutes || r.desc)
    ).length
    if (count) setConfirm({ date: sel.date, draft, count })
    else applyDraft(sel.date, draft)
  }
  const onDraftEmpty = () => {
    const drafted: Rows = {}
    for (const d of dates) {
      if ((rows[d] ?? []).some((r) => r.minutes || r.desc)) continue
      const draft = draftDay(
        todone.data?.[d],
        target(d),
        settings.ignoreEvents,
        tasks,
        keywords
      )
      if (draft.length) drafted[d] = draft
    }
    const n = Object.keys(drafted).length
    if (!n) return noMatch()
    setRows((r) => ({ ...r, ...drafted }))
    toast.success(`Drafted ${n} ${n === 1 ? "day" : "days"}.`)
  }

  const [busy, setBusy] = useState<"pull" | "push" | "fetch" | null>(null)
  const run = async (
    kind: "pull" | "push" | "fetch",
    fn: () => Promise<string>,
    failure: string
  ) => {
    setBusy(kind)
    try {
      toast.success(await fn())
    } catch (e) {
      toast.error(failure, { description: errorText(e) })
    } finally {
      setBusy(null)
    }
  }

  const pull = () =>
    run(
      "pull",
      async () => {
        const s = await ensureSession()
        if (!assigned.current.length) await loadAssigned()
        const picked = tasks
          .filter((t) => shown.includes(t.id))
          .map((t) => t.projectId)
        const projectIds = [...new Set([...assigned.current, ...picked])]
        const { entries, tasks: seen } = await fetchEntries(
          projectIds,
          s.userId,
          dates[0],
          dates[dates.length - 1]
        )
        addTasks(seen.filter((t) => !tasks.some((x) => x.id === t.id)))
        setRows((prev) => {
          const next: Rows = {}
          for (const d of dates) {
            const fresh = entries
              .filter((e) => e.date === d)
              .map(({ date, ...e }) => e)
            next[d] = mergeDay(prev[d] ?? [], fresh)
          }
          return next
        })
        return `Pulled ${entries.length} ${entries.length === 1 ? "entry" : "entries"} from Xero.`
      },
      "Couldn't pull from Xero."
    )

  // Clearing a synced cell's time deletes the entry in Xero. Each change is saved as it lands,
  // so a retry after a failure only redoes what didn't finish.
  const push = () =>
    run(
      "push",
      async () => {
        const s = await ensureSession()
        let done = 0
        let skipped = 0
        const known = new Set(
          Object.values(rows)
            .flat()
            .flatMap((r) => (r.xeroId ? [r.xeroId] : []))
        )
        for (const [date, list] of Object.entries(rows)) {
          for (const r of list) {
            if (!r.minutes) {
              if (r.desc.trim() && !r.xeroId) skipped++
              if (!r.xeroId) continue
              await removeEntry(r)
              done++
              setRows((p) => ({
                ...p,
                [date]: (p[date] ?? []).filter((x) => x.id !== r.id),
              }))
              continue
            }
            if (!isDirty(r)) continue
            const project =
              r.xeroProject ?? tasks.find((t) => t.id === r.taskId)?.projectId
            if (!project)
              throw new Error(
                `No Xero project is known for a ${date} entry. Pull from Xero, then push again.`
              )
            const xeroId =
              r.xeroId ?? (await createEntry(r, date, s.userId, project, known))
            if (r.xeroId) await updateEntry(r, date)
            known.add(xeroId)
            done++
            const synced = snap(r)
            setRows((p) => ({
              ...p,
              [date]: (p[date] ?? []).map((x) =>
                x.id === r.id
                  ? { ...x, xeroId, xeroProject: project, synced }
                  : x
              ),
            }))
          }
        }
        if (skipped)
          throw new Error(
            `Pushed ${done}. ${skipped} ${skipped === 1 ? "entry has" : "entries have"} a description but no time; add time to push them.`
          )
        return done
          ? `Pushed ${done} ${done === 1 ? "change" : "changes"} to Xero.`
          : "Everything is already in Xero."
      },
      "Push stopped partway."
    )

  const fetchAll = () =>
    run(
      "fetch",
      async () => {
        await ensureSession()
        const { tasks: all } = await fetchTasks(true)
        addTasks(all)
        return `Found ${all.length} tasks across all projects. Add them from Add Task.`
      },
      "Couldn't fetch projects."
    )

  const hide = (ids: string[]) => {
    setShown((s) => s.filter((id) => !ids.includes(id)))
    const kept = visible
      .filter((t) => ids.includes(t.id) && hasTime(t.id))
      .map((t) => t.name)
    if (kept.length)
      toast.info(
        `${kept.join(", ")} ${kept.length === 1 ? "has" : "have"} time this month, so the row stays.`
      )
  }

  const [kwTask, setKwTask] = useState<Task | null>(null)
  const [settingsOpen, setSettingsOpen] = useState(false)

  const loading = !todone.data && !todone.error
  const showSkeleton = useDelayed(loading || booting)
  const addTask = (
    <AddTask
      tasks={hidden}
      onAdd={(id) => setShown((s) => [...s, id])}
      onFetchAll={fetchAll}
    />
  )

  const body = todone.error ? (
    <Empty className="m-auto">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <AlertTriangleIcon />
        </EmptyMedia>
        <EmptyTitle>Couldn't Load todone</EmptyTitle>
        <EmptyDescription>{todone.error}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button onClick={() => setReload((n) => n + 1)}>Try Again</Button>
      </EmptyContent>
    </Empty>
  ) : loading || booting ? (
    showSkeleton && (
      <div aria-busy className="flex flex-col gap-2 p-4">
        <span className="sr-only">Loading timesheet…</span>
        {Array.from({ length: 6 }, (_, i) => (
          <div key={i} className="flex gap-2">
            <Skeleton className="h-10 w-56" />
            {Array.from({ length: 10 }, (_, j) => (
              <Skeleton key={j} className="h-10 w-24" />
            ))}
          </div>
        ))}
      </div>
    )
  ) : !visible.length ? (
    <Empty className="m-auto">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <ListPlusIcon />
        </EmptyMedia>
        <EmptyTitle>
          {tasks.length ? "Add Your First Task" : "Connect Xero to Load Tasks"}
        </EmptyTitle>
        <EmptyDescription>
          {tasks.length
            ? "Only tasks you add, or tasks with time this month, get a row."
            : "Paste a cURL copied from Xero to load the tasks in your projects."}
        </EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        {tasks.length ? (
          addTask
        ) : (
          <Button onClick={() => setConnectOpen(true)}>Connect Xero</Button>
        )}
      </EmptyContent>
    </Empty>
  ) : (
    <TimesheetGrid
      columns={columns}
      tasks={visible}
      rows={rows}
      keywords={keywords}
      selection={sel}
      gap={gap}
      onSelect={setSel}
      onMinutes={onMinutes}
      onHide={hide}
      onEditKeywords={setKwTask}
      isBillable={isBillable}
      onBillable={(id, value) => setBillable((b) => ({ ...b, [id]: value }))}
    />
  )

  const expires = session
    ? new Date(session.exp * 1000).toLocaleTimeString(undefined, {
        timeStyle: "short",
      })
    : ""

  return (
    <SidebarProvider
      style={{ "--sidebar-width": "22rem" } as React.CSSProperties}
      className="h-svh"
    >
      <DaySidebar
        date={sel.date}
        day={todone.data?.[sel.date]}
        gitlab={gitlab[sel.date]}
        onRetryGitlab={() =>
          setGitlab((g) =>
            Object.fromEntries(
              Object.entries(g).filter(([d]) => d !== sel.date)
            )
          )
        }
        gap={gap(sel.date)}
        taskName={
          sel.taskId
            ? (tasks.find((t) => t.id === sel.taskId)?.name ?? "Task")
            : null
        }
        desc={selRow?.desc ?? ""}
        onDesc={onDesc}
        onDraftDay={onDraftDay}
      />
      <SidebarInset className="min-w-0 overflow-hidden">
        <header className="flex flex-wrap items-center gap-2 border-b px-3 py-2">
          <SidebarTrigger aria-label="Toggle day panel" />
          <Separator orientation="vertical" className="h-5" />
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Previous month"
            onClick={() => setMonth(shiftMonth(month, -1))}
          >
            <ChevronLeftIcon />
          </Button>
          <h1 className="min-w-36 text-center text-sm font-semibold">
            {monthLabel(month)}
          </h1>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Next month"
            onClick={() => setMonth(shiftMonth(month, 1))}
          >
            <ChevronRightIcon />
          </Button>
          {month !== thisMonth() && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setMonth(thisMonth())}
            >
              This Month
            </Button>
          )}
          <span className="ml-2 text-sm text-muted-foreground tabular-nums">
            {fmt(monthTotal)} logged
          </span>
          <div className="ml-auto flex flex-wrap items-center gap-2">
            {addTask}
            <Button
              variant="outline"
              size="sm"
              onClick={onDraftEmpty}
              disabled={!todone.data}
            >
              <SparklesIcon data-icon="inline-start" />
              Draft Empty Days
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={pull}
              disabled={!!busy}
            >
              {busy === "pull" ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <CloudDownloadIcon data-icon="inline-start" />
              )}
              Pull
            </Button>
            <Button size="sm" onClick={push} disabled={!!busy}>
              {busy === "push" ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <CloudUploadIcon data-icon="inline-start" />
              )}
              Push
              {pending > 0 && (
                <Badge
                  variant="secondary"
                  className="ml-0.5 h-4 min-w-4 px-1 tabular-nums"
                >
                  {pending}
                  <span className="sr-only"> unpushed</span>
                </Badge>
              )}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setConnectOpen(true)}
              className={
                live
                  ? ""
                  : "border-amber-500/60 bg-amber-500/10 text-amber-700 hover:bg-amber-500/20 dark:text-amber-300"
              }
            >
              {live ? (
                <span
                  aria-hidden
                  className="size-2 rounded-full bg-emerald-500"
                />
              ) : (
                <PlugIcon data-icon="inline-start" />
              )}
              {live
                ? `Xero Until ${expires}`
                : session
                  ? "Reconnect Xero"
                  : "Connect Xero"}
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label="More actions"
                >
                  {busy === "fetch" ? <Spinner /> : <EllipsisIcon />}
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={fetchAll}>
                  Fetch Tasks From All Projects…
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={() => setSettingsOpen(true)}>
                  Settings…
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </header>
        {visible.length > 0 && !loading && !booting && !todone.error && (
          <BillableSummary
            period={monthLabel(month)}
            billable={billableMinutes}
            nonBillable={monthTotal - billableMinutes}
          />
        )}
        <div className="flex min-h-0 flex-1 overflow-auto overscroll-contain">
          {body}
        </div>
      </SidebarInset>

      <ConnectDialog
        open={connectOpen}
        onOpenChange={onConnectOpenChange}
        onConnected={connected}
      />
      <KeywordsDialog
        task={kwTask}
        value={kwTask ? (keywords[kwTask.id] ?? "") : ""}
        onOpenChange={(o) => !o && setKwTask(null)}
        onSave={(v) => {
          if (kwTask) setKeywords((k) => ({ ...k, [kwTask.id]: v }))
          setKwTask(null)
        }}
      />
      <SettingsDialog
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        settings={settings}
        onSave={(s) => {
          setSettings(s)
          setSettingsOpen(false)
        }}
      />
      <AlertDialog
        open={!!confirm}
        onOpenChange={(o) => !o && setConfirm(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Replace Unpushed Entries?</AlertDialogTitle>
            <AlertDialogDescription>
              Drafting replaces {confirm?.count} unpushed{" "}
              {confirm?.count === 1 ? "entry" : "entries"} on this day. Entries
              already in Xero stay.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep Entries</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (confirm) applyDraft(confirm.date, confirm.draft)
                setConfirm(null)
              }}
            >
              Replace & Draft
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <Toaster position="bottom-right" />
    </SidebarProvider>
  )
}
