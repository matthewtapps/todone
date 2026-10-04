import { useEffect, useState } from "react"

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
