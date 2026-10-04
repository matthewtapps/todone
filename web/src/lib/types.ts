export type Task = {
  id: string
  name: string
  projectId: string
  projectName: string
}

export type Row = {
  id: string
  xeroId: string | null
  xeroProject?: string
  taskId: string
  desc: string
  minutes: number
  /** Snapshot of the row as last seen in Xero; null until pushed. */
  synced: string | null
}

export type MonthState = { rows: Record<string, Row[]> }

export type TodoneEvent = {
  summary: string
  start: string
  end: string
  all_day: boolean
}
export type TodoneDay = { did: string[]; events: TodoneEvent[] }

export type Settings = { hoursPerDay: number; ignoreEvents: string }

export type Session = {
  userId: string
  exp: number
  shortcode: string
}

export const snap = (r: Row) => JSON.stringify([r.taskId, r.desc, r.minutes])
export const isDirty = (r: Row) => r.synced !== snap(r)
export const isPending = (r: Row) => (r.xeroId ? isDirty(r) : r.minutes > 0)
export const newRow = (taskId: string): Row => ({
  id: crypto.randomUUID(),
  xeroId: null,
  taskId,
  desc: "",
  minutes: 0,
  synced: null,
})
