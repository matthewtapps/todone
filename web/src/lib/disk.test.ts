import assert from "node:assert/strict"
import { test } from "node:test"

import { localMonths, shouldReplace } from "./disk.ts"

test("a newer disk version replaces local state when nothing is unsaved", () => {
  assert.equal(shouldReplace({ version: 3, unsaved: false }, 4), true)
})

test("unsaved edits are never replaced by a poll", () => {
  assert.equal(shouldReplace({ version: 3, unsaved: true }, 4), false)
})

test("the same or an older version is ignored", () => {
  assert.equal(shouldReplace({ version: 3, unsaved: false }, 3), false)
  assert.equal(shouldReplace({ version: 3, unsaved: false }, 2), false)
})

test("only month keys count as local months", () => {
  assert.deepEqual(
    localMonths([
      "timesheet:2026-10",
      "timesheet:tasks",
      "timesheet:shortcode",
      "timesheet:2026-09",
      "other:2026-08",
    ]),
    ["2026-09", "2026-10"]
  )
})
