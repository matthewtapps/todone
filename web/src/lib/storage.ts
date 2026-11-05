import { useCallback, useEffect, useRef, useState } from "react"
import { toast } from "sonner"
import { getDoc, putDoc } from "./api.ts"
import { localMonths, shouldReplace } from "./disk.ts"

// Keys match the earlier standalone page on the same origin, so saved state carries over.
export const monthKey = (month: string) => `timesheet:${month}`

export function load<T>(key: string, fallback: T): T {
  try {
    return JSON.parse(localStorage.getItem(key) ?? "null") ?? fallback
  } catch {
    return fallback
  }
}

export const save = (key: string, value: unknown) =>
  localStorage.setItem(key, JSON.stringify(value))

export function useStored<T>(key: string, fallback: T) {
  const [value, setValue] = useState<T>(() => load(key, fallback))
  useEffect(() => save(key, value), [key, value])
  return [value, setValue] as const
}

type DocState<T> = {
  path: string
  version: number
  value: T
  unsaved: boolean
  loaded: boolean
  gen: number
  error?: string
}
type Job<T> = { path: string; version: number; value: T; gen: number }

const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e))
// A save may only update the state it was sent from, not one loaded since.
let generation = 0
const fresh = <T>(path: string, value: T): DocState<T> => ({
  path,
  version: 0,
  value,
  unsaved: false,
  loaded: false,
  gen: ++generation,
})

/** A server-backed document; `fallback` must be a stable module constant. */
export function useDiskDoc<T extends object>(
  path: string,
  fallback: T,
  reload: number
) {
  const [state, setState] = useState(() => fresh(path, fallback))
  const s = state.path === path ? state : fresh(path, fallback)
  if (s !== state) setState(s)
  const current = useRef(s)
  useEffect(() => {
    current.current = s
  })

  const replace = useCallback(
    (p: string, version: number, doc: Record<string, unknown>) =>
      setState((x) =>
        x.path === p
          ? {
              path: p,
              version,
              value: { ...fallback, ...doc } as T,
              unsaved: false,
              loaded: true,
              gen: ++generation,
            }
          : x
      ),
    [fallback]
  )

  useEffect(() => {
    let live = true
    const load = (first: boolean) =>
      getDoc(path).then(
        ({ version, ...doc }) => {
          if (!live || (!first && !shouldReplace(current.current, version)))
            return
          replace(path, version, doc)
          if (!first) toast.info("Timesheet updated on disk.")
        },
        (e) => {
          if (live && first)
            setState((x) =>
              x.path === path ? { ...x, loaded: true, error: errorText(e) } : x
            )
        }
      )
    load(true)
    const poll = () => {
      if (document.visibilityState === "visible") load(false)
    }
    const timer = setInterval(poll, 5000)
    window.addEventListener("focus", poll)
    return () => {
      live = false
      clearInterval(timer)
      window.removeEventListener("focus", poll)
    }
  }, [path, reload, replace])

  const pending = useRef<Job<T> | null>(null)
  const save = useCallback(
    (job: Job<T>, keepalive = false) =>
      putDoc(job.path, job.version, job.value, keepalive).then(
        (r) => {
          if (r.ok)
            setState((x) =>
              x.path === job.path && x.gen === job.gen
                ? { ...x, version: r.version, unsaved: x.value !== job.value }
                : x
            )
          else {
            replace(job.path, r.version, r.doc)
            toast.warning(
              "Timesheet changed on disk; your last edit wasn't saved."
            )
          }
        },
        (e) =>
          toast.error("Couldn't save the timesheet.", {
            description: errorText(e),
          })
      ),
    [replace]
  )
  const flush = useCallback(
    (keepalive = false) => {
      const job = pending.current
      pending.current = null
      if (job) save(job, keepalive)
    },
    [save]
  )

  useEffect(() => {
    if (!s.unsaved) {
      pending.current = null
      return
    }
    pending.current = {
      path: s.path,
      version: s.version,
      value: s.value,
      gen: s.gen,
    }
    const timer = setTimeout(() => flush(), 500)
    return () => clearTimeout(timer)
  }, [s, flush])

  // Switching month or closing the tab must not drop an edit still waiting on the timer.
  useEffect(() => () => flush(), [path, flush])
  useEffect(() => {
    const onHide = () => flush(true)
    window.addEventListener("pagehide", onHide)
    return () => window.removeEventListener("pagehide", onHide)
  }, [flush])

  const set = useCallback(
    (f: (v: T) => T) =>
      setState((x) =>
        x.path === path ? { ...x, value: f(x.value), unsaved: true } : x
      ),
    [path]
  )
  return { value: s.value, loaded: s.loaded, error: s.error, set }
}

const SETTINGS_KEYS = [
  "tasks",
  "shown",
  "keywords",
  "billable",
  "settings",
] as const

/** Copies localStorage timesheet data to disk where the disk has none, then drops the local copy. */
export async function migrateLocal() {
  const settings = await getDoc("/api/timesheet/settings")
  const local = SETTINGS_KEYS.flatMap((k) => {
    const v = localStorage.getItem(`timesheet:${k}`)
    return v === null ? [] : [[k, JSON.parse(v)] as const]
  })
  if (settings.version === 0 && local.length) {
    const r = await putDoc(
      "/api/timesheet/settings",
      0,
      Object.fromEntries(local)
    )
    if (r.ok)
      SETTINGS_KEYS.forEach((k) => localStorage.removeItem(`timesheet:${k}`))
  }
  for (const month of localMonths(Object.keys(localStorage))) {
    const path = `/api/timesheet/${month}`
    if ((await getDoc(path)).version !== 0) continue
    const rows = load<{ rows: Record<string, { taskId?: string }[]> }>(
      monthKey(month),
      { rows: {} }
    ).rows
    const kept = Object.fromEntries(
      Object.entries(rows).map(([d, list]) => [d, list.filter((r) => r.taskId)])
    )
    const r = await putDoc(path, 0, { rows: kept })
    if (r.ok) localStorage.removeItem(monthKey(month))
  }
}
