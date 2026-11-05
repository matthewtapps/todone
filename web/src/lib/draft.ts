import { minutesBetween } from "./time.ts"
import { newRow, type Row, type Task, type TodoneDay } from "./types.ts"

export const keywordList = (s: string | undefined) =>
  (s ?? "")
    .split(",")
    .map((k) => k.trim())
    .filter(Boolean)

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")

/** First task with a keyword appearing in `text` as a whole word, case-insensitive. */
export function taskFor(
  text: string,
  tasks: Task[],
  keywords: Record<string, string>
): string {
  const hit = tasks.find((t) =>
    keywordList(keywords[t.id]).some((k) =>
      new RegExp(`(^|\\W)${escapeRe(k)}($|\\W)`, "i").test(text)
    )
  )
  return hit?.id ?? ""
}

/** Folds `- sub` bullets into the bullet above them. */
export function mergeBullets(lines: string[]): string[] {
  const items: string[] = []
  for (const line of lines) {
    const sub = line.match(/^\s*-\s*(.*)/)
    if (sub && items.length) {
      const last = items.pop()!
      items.push(last + (last.endsWith(":") ? " " : "; ") + sub[1])
    } else items.push(line)
  }
  return items
}

/**
 * Meetings keep their calendar length; bullets share what is left of the day.
 * Only items matching a task's keywords become rows; one row per task.
 */
export function draftDay(
  day: TodoneDay | undefined,
  targetMinutes: number,
  ignoreEvents: string,
  tasks: Task[],
  keywords: Record<string, string>
): Row[] {
  if (!day) return []
  const ignore = ignoreEvents.trim() ? new RegExp(ignoreEvents, "i") : null
  const meetings = day.events
    .filter((e) => !e.all_day && !ignore?.test(e.summary))
    .map(
      (e) => [e.summary, Math.round(minutesBetween(e.start, e.end))] as const
    )
  const items = mergeBullets(day.did)
  const left = Math.max(
    0,
    targetMinutes - meetings.reduce((a, m) => a + m[1], 0)
  )
  const each = items.length ? Math.round(left / items.length / 15) * 15 : 0
  const byTask: Record<string, Row> = {}
  for (const [text, minutes] of [
    ...meetings,
    ...items.map((d) => [d, each] as const),
  ]) {
    const taskId = taskFor(text, tasks, keywords)
    if (!taskId) continue
    const r = (byTask[taskId] ??= newRow(taskId))
    r.minutes += minutes
    r.desc = r.desc ? `${r.desc}\n${text}` : text
  }
  return Object.values(byTask)
}
