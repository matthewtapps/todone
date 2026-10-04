export const fmt = (m: number) =>
  `${m < 0 ? "-" : ""}${Math.floor(Math.abs(m) / 60)}:${String(Math.abs(m) % 60).padStart(2, "0")}`

/** Accepts `1:30`, `:45`, `1.5`, `1.5h`, `90m`; empty means zero. Returns null when unparseable. */
export function parseDur(s: string): number | null {
  s = s.trim()
  let m: RegExpMatchArray | null
  if (!s) return 0
  if ((m = s.match(/^(\d*):(\d{1,2})$/))) return +m[1] * 60 + +m[2]
  if ((m = s.match(/^(\d+)m$/))) return +m[1]
  if ((m = s.match(/^(\d*\.?\d+)h?$/))) return Math.round(parseFloat(m[1]) * 60)
  return null
}

export function monthDates(month: string): string[] {
  const [y, mo] = month.split("-").map(Number)
  const n = new Date(y, mo, 0).getDate()
  return Array.from(
    { length: n },
    (_, i) => `${month}-${String(i + 1).padStart(2, "0")}`
  )
}

export function shiftMonth(month: string, by: number): string {
  const [y, mo] = month.split("-").map(Number)
  const d = new Date(y, mo - 1 + by, 1)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`
}

export const weekday = (d: string) => new Date(d + "T00:00").getDay()
export const isWeekend = (d: string) => [0, 6].includes(weekday(d))
export const today = () => new Date().toLocaleDateString("en-CA")
export const thisMonth = () => today().slice(0, 7)

export const dayLabel = (
  d: string,
  opts: Intl.DateTimeFormatOptions = { weekday: "short", day: "numeric" }
) => new Date(d + "T00:00").toLocaleDateString(undefined, opts)

export const monthLabel = (month: string) =>
  new Date(month + "-01T00:00").toLocaleDateString(undefined, {
    month: "long",
    year: "numeric",
  })

export const localDate = (utc: string, timeZone?: string) =>
  new Date(utc).toLocaleDateString("en-CA", { timeZone })
export const toUtc = (date: string) =>
  new Date(date + "T00:00").toISOString().replace(".000", "")
export const minutesBetween = (start: string, end: string) =>
  (new Date(end).getTime() - new Date(start).getTime()) / 60000
