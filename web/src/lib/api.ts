import { localDate, toUtc } from "./time.ts"
import type { Row, Session, Task, TodoneDay } from "./types.ts"

export class ApiError extends Error {
  status: number
  constructor(message: string, status: number) {
    super(message)
    this.status = status
  }
}

async function request<T>(path: string, opts: RequestInit = {}) {
  const r = await fetch(path, {
    ...opts,
    headers: {
      "X-Timesheet": "1",
      "Content-Type": "application/json",
      ...opts.headers,
    },
  })
  const body = await r.json().catch(() => null)
  if (!r.ok)
    throw new ApiError(body?.error || `${r.status} ${r.statusText}`, r.status)
  return { body: body as T, headers: r.headers }
}

export const api = async <T>(path: string, opts: RequestInit = {}) =>
  (await request<T>(path, opts)).body

export const fetchTodone = (from: string, to: string) =>
  api<Record<string, TodoneDay>>(`/api/todone?from=${from}&to=${to}`)
export type GitlabDay = {
  enabled: boolean
  groups?: { project: string; lines: { kind: string; text: string }[] }[]
}
export const fetchGitlab = (date: string) =>
  api<GitlabDay>(`/api/gitlab?date=${date}`)
export const getSession = () => api<Session>("/api/session")
export const postSession = (text: string) =>
  api<Session>("/api/session", {
    method: "POST",
    body: JSON.stringify({ text }),
  })

export const startLogin = (shortcode: string) =>
  api<{ url: string }>("/api/xero-login", {
    method: "POST",
    body: JSON.stringify({ shortcode }),
  })
export const finishLogin = (text: string) =>
  api<Session>("/api/xero-login/finish", {
    method: "POST",
    body: JSON.stringify({ text }),
  })

// Called on a 401 so the UI can ask for a fresh paste; rejects if you cancel.
let reauth: (() => Promise<void>) | null = null
export const setReauth = (f: typeof reauth) => {
  reauth = f
}

async function xrequest<T>(path: string, opts: RequestInit = {}) {
  try {
    return await request<T>("/api/xero/" + path, opts)
  } catch (e) {
    if (!(e instanceof ApiError) || e.status !== 401 || !reauth) throw e
    await reauth()
    return request<T>("/api/xero/" + path, opts)
  }
}

const xapi = async <T>(path: string, opts: RequestInit = {}) =>
  (await xrequest<T>(path, opts)).body

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms))

type Items<T> = { items: T[] }
type XProject = { projectId: string; name: string }
type XTask = { taskId: string; projectId: string; name: string; status: string }
type XEntry = {
  timeEntryId: string
  userId: string
  projectId: string
  projectName: string
  taskId: string
  taskName: string
  dateUtc: string
  timeZone?: string
  duration: number
  description: string | null
  dateEnteredUtc?: string
}

/** Active tasks in your assigned projects, or every in-progress project when `all`. */
export async function fetchTasks(
  all = false
): Promise<{ tasks: Task[]; projectIds: string[] }> {
  const projects = (
    await xapi<Items<XProject>>(
      `projects?status=INPROGRESS&assigned=${all ? "any" : "mine"}&itemsPerPage=100`
    )
  ).items
  const lists = await Promise.all(
    projects.map((p) => xapi<Items<XTask>>(`projects/${p.projectId}/tasks`))
  )
  const tasks = lists.flatMap((l, i) =>
    l.items
      .filter((t) => t.status === "ACTIVE")
      .map((t) => ({
        id: t.taskId,
        name: t.name,
        projectId: t.projectId,
        projectName: projects[i].name,
      }))
  )
  return { tasks, projectIds: projects.map((p) => p.projectId) }
}

export type Entry = Row & { date: string }

// The time endpoint ignores filters and returns every user's entries for the project.
export async function fetchEntries(
  projectIds: string[],
  userId: string,
  from: string,
  to: string
) {
  const lists = await Promise.all(
    projectIds.map((id) => xapi<Items<XEntry>>(`projects/${id}/time`))
  )
  const mine = lists.flatMap((l) => l.items).filter((e) => e.userId === userId)
  const tasks: Task[] = mine.map((e) => ({
    id: e.taskId,
    name: e.taskName,
    projectId: e.projectId,
    projectName: e.projectName,
  }))
  const entries: Entry[] = mine
    .map((e) => ({
      id: crypto.randomUUID(),
      xeroId: e.timeEntryId,
      xeroProject: e.projectId,
      date: localDate(e.dateUtc, e.timeZone),
      taskId: e.taskId,
      desc: e.description ?? "",
      minutes: e.duration,
      synced: null,
    }))
    .filter((e) => e.date >= from && e.date <= to)
  return { entries, tasks }
}

/** Creates the entry and returns its ID; `known` holds IDs already linked to other rows. */
export async function createEntry(
  row: Row,
  date: string,
  userId: string,
  projectId: string,
  known: Set<string>
) {
  const { body, headers } = await xrequest<XEntry | null>(
    `projects/${projectId}/time`,
    {
      method: "POST",
      body: JSON.stringify({
        taskId: row.taskId,
        userId,
        duration: row.minutes,
        dateUtc: toUtc(date),
        description: row.desc,
        timeEntryType: "duration",
      }),
    }
  )
  const id = body?.timeEntryId ?? headers.get("location")?.split("/").pop()
  if (id) return id

  // Last resort: find the newest matching entry. Xero's list lags a moment behind
  // writes, so retry; descriptions aren't compared because Xero normalises whitespace.
  for (let attempt = 0; attempt < 4; attempt++) {
    if (attempt) await sleep(1500)
    const { items } = await xapi<Items<XEntry>>(`projects/${projectId}/time`)
    const match = items
      .filter(
        (x) =>
          x.userId === userId &&
          x.taskId === row.taskId &&
          x.duration === row.minutes &&
          !known.has(x.timeEntryId) &&
          localDate(x.dateUtc, x.timeZone) === date
      )
      .sort((a, b) =>
        (b.dateEnteredUtc ?? "").localeCompare(a.dateEnteredUtc ?? "")
      )[0]
    if (match) return match.timeEntryId
  }
  throw new Error(
    "Xero took the entry but didn't say which one it was. Pull from Xero before pushing again, so it isn't added twice."
  )
}

export const updateEntry = (row: Row, date: string) =>
  xapi(`projects/${row.xeroProject}/time/${row.xeroId}`, {
    method: "PUT",
    body: JSON.stringify({
      projectId: row.xeroProject,
      taskId: row.taskId,
      duration: row.minutes,
      dateUtc: toUtc(date),
      description: row.desc,
    }),
  })

export const removeEntry = (row: Row) =>
  xapi(`projects/${row.xeroProject}/time/${row.xeroId}`, { method: "DELETE" })
