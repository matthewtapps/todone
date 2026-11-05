import { isDirty, snap, type Row } from "./types.ts"

// Xero normalises whitespace in descriptions, so compare them with it collapsed.
const text = (s: string) => s.replace(/\s+/g, " ").trim()
const same = (a: Row, b: Row) =>
  a.taskId === b.taskId &&
  a.minutes === b.minutes &&
  text(a.desc) === text(b.desc)

/**
 * One day's rows after a pull: Xero's entries, then local edits to them, then unpushed rows.
 * An unpushed row identical to a Xero entry is taken to be that entry, so a push whose
 * reply went missing isn't pushed a second time.
 */
export function mergeDay(local: Row[], fresh: Row[]): Row[] {
  const edited = local.filter((r) => r.xeroId && isDirty(r))
  const kept = new Set(edited.map((r) => r.xeroId))
  const byXero = new Map(local.map((r) => [r.xeroId, r]))
  const incoming = fresh
    .filter((e) => !kept.has(e.xeroId))
    .map((e) => {
      const old = byXero.get(e.xeroId)
      const untouched = !!old && old.imported === snap(old) && same(old, e)
      return {
        ...e,
        synced: snap(e),
        imported: untouched ? snap(e) : (old?.imported ?? null),
      }
    })
  const claimed = new Set<string>()
  const unpushed = local
    .filter((r) => !r.xeroId)
    .filter((r) => {
      const twin = incoming.find((e) => !claimed.has(e.id) && same(e, r))
      if (twin) claimed.add(twin.id)
      return !twin
    })
  return [...incoming, ...edited, ...unpushed]
}
