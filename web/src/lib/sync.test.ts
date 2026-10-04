import assert from "node:assert/strict"
import { test } from "node:test"

import { mergeDay } from "./sync.ts"
import { snap, type Row } from "./types.ts"

const row = (
  id: string,
  xeroId: string | null,
  minutes: number,
  desc = "work"
): Row => ({
  id,
  xeroId,
  taskId: "t",
  desc,
  minutes,
  synced: null,
})

test("an unpushed row matching a Xero entry is replaced by it", () => {
  const merged = mergeDay([row("local", null, 60)], [row("x", "X1", 60)])
  assert.deepEqual(
    merged.map((r) => r.xeroId),
    ["X1"]
  )
})

test("different unpushed rows are kept", () => {
  const merged = mergeDay([row("local", null, 30)], [row("x", "X1", 60)])
  assert.deepEqual(
    merged.map((r) => r.xeroId),
    ["X1", null]
  )
})

test("each Xero entry absorbs at most one unpushed row", () => {
  const merged = mergeDay(
    [row("a", null, 60), row("b", null, 60)],
    [row("x", "X1", 60)]
  )
  assert.deepEqual(
    merged.map((r) => r.id),
    ["x", "b"]
  )
})

test("local edits to synced rows win over the pulled copy", () => {
  const edited = { ...row("e", "X1", 60), synced: snap(row("e", "X1", 30)) }
  const merged = mergeDay([edited], [row("x", "X1", 30)])
  assert.deepEqual(
    merged.map((r) => r.id),
    ["e"]
  )
})

test("matching ignores whitespace differences", () => {
  const merged = mergeDay(
    [row("local", null, 60, "a\nb ")],
    [row("x", "X1", 60, "a\r\nb")]
  )
  assert.deepEqual(
    merged.map((r) => r.xeroId),
    ["X1"]
  )
})
