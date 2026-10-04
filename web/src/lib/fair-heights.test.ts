import assert from "node:assert/strict"
import { test } from "node:test"

import { fairShares } from "./fair-heights.ts"

test("everything fits: each gets what it wants", () => {
  assert.deepEqual(fairShares([50, 100, 80], 400), [50, 100, 80])
})

test("short sections keep their height; long ones split the rest", () => {
  assert.deepEqual(fairShares([60, 2000, 900], 600), [60, 270, 270])
})

test("no space means nothing", () => {
  assert.deepEqual(fairShares([60, 100], -10), [0, 0])
})
