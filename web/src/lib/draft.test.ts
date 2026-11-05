import assert from "node:assert/strict"
import { test } from "node:test"

import { draftDay, mergeBullets, taskFor } from "./draft.ts"
import { parseDur } from "./time.ts"

const tasks = [
  { id: "wtk", name: "WTK", projectId: "p", projectName: "P" },
  { id: "lw", name: "LW", projectId: "p", projectName: "P" },
  { id: "admin", name: "Admin", projectId: "p", projectName: "P" },
]
const keywords = {
  wtk: "wtk, warren",
  lw: "lw, lightwire",
  admin: "standup, 1:1",
}

test("parseDur", () => {
  assert.equal(parseDur("1:30"), 90)
  assert.equal(parseDur(":45"), 45)
  assert.equal(parseDur("1.5"), 90)
  assert.equal(parseDur("90m"), 90)
  assert.equal(parseDur(""), 0)
  assert.equal(parseDur("soon"), null)
})

test("taskFor matches whole words only", () => {
  assert.equal(taskFor("Fix wtk-web N+1", tasks, keywords), "wtk")
  assert.equal(taskFor("LW effects tests", tasks, keywords), "lw")
  assert.equal(taskFor("Always testing", tasks, keywords), "")
  assert.equal(taskFor("1:1 with Harry", tasks, keywords), "admin")
})

test("mergeBullets folds sub-bullets", () => {
  assert.deepEqual(mergeBullets(["Reviews:", "- a", "- b", "Other"]), [
    "Reviews: a; b",
    "Other",
  ])
})

test("draftDay splits the rest of the day and groups by task", () => {
  const rows = draftDay(
    {
      did: ["Warren sim work", "Lightwire tests", "Misc"],
      events: [
        {
          summary: "Team standup",
          start: "2026-09-01T10:00:00+10:00",
          end: "2026-09-01T10:30:00+10:00",
          all_day: false,
        },
        {
          summary: "Private Appointment",
          start: "2026-09-01T12:00:00+10:00",
          end: "2026-09-01T13:00:00+10:00",
          all_day: false,
        },
      ],
    },
    480,
    "private appointment",
    tasks,
    keywords
  )
  const byTask = Object.fromEntries(rows.map((r) => [r.taskId, r.minutes]))
  assert.deepEqual(byTask, { admin: 30, wtk: 150, lw: 150 })
})

test("draftDay stores whole minutes for events with seconds", () => {
  const rows = draftDay(
    {
      did: [],
      events: [
        {
          summary: "Team standup",
          start: "2026-09-01T10:00:00+10:00",
          end: "2026-09-01T10:29:40+10:00",
          all_day: false,
        },
      ],
    },
    480,
    "",
    tasks,
    keywords
  )
  assert.deepEqual(
    rows.map((r) => r.minutes),
    [30]
  )
})
