/** A poll replaces local state only when the disk is newer and nothing local is waiting to save. */
export const shouldReplace = (
  local: { version: number; unsaved: boolean },
  remote: number
) => remote > local.version && !local.unsaved

/** Months still in localStorage under keys like `timesheet:2026-10`. */
export const localMonths = (keys: string[]) =>
  keys.flatMap((k) => /^timesheet:(\d{4}-\d{2})$/.exec(k)?.[1] ?? []).sort()
