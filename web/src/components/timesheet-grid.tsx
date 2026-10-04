import { memo, useEffect, useRef, useState } from "react"
import { CheckIcon, EllipsisIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { cn } from "@/lib/utils"
import { dayLabel, fmt, parseDur, weekday } from "@/lib/time"
import { isDirty, isPending, type Row, type Task } from "@/lib/types"

export type Column = { date: string } | { week: string[] }
export type Selection = {
  date: string
  taskId: string | null
  rowId: string | null
}

type Props = {
  columns: Column[]
  tasks: Task[]
  rows: Record<string, Row[]>
  keywords: Record<string, string>
  selection: Selection
  gap: (date: string) => { total: number; gap: number }
  onSelect: (s: Selection) => void
  onMinutes: (
    date: string,
    taskId: string,
    rowId: string | null,
    minutes: number
  ) => void
  onHide: (taskIds: string[]) => void
  onEditKeywords: (task: Task) => void
  isBillable: (task: Task) => boolean
  onBillable: (taskId: string, billable: boolean) => void
}

const sumFor = (
  rows: Record<string, Row[]>,
  dates: string[],
  taskId?: string
) =>
  dates.reduce(
    (a, d) =>
      a +
      (rows[d] ?? [])
        .filter((r) => !taskId || r.taskId === taskId)
        .reduce((b, r) => b + r.minutes, 0),
    0
  )

export function GapLabel({
  total,
  gap,
  stacked = false,
}: {
  total: number
  gap: number
  stacked?: boolean
}) {
  return (
    <span
      className={cn(
        "inline-flex gap-x-1 whitespace-nowrap tabular-nums",
        stacked ? "flex-col items-start" : "items-center",
        gap > 0 && "text-amber-600 dark:text-amber-400",
        gap < 0 && "text-destructive",
        gap === 0 && "text-emerald-600 dark:text-emerald-400"
      )}
    >
      {gap === 0 && <CheckIcon aria-hidden className="size-3" />}
      {fmt(total)}
      {gap > 0 && (
        <span className="text-muted-foreground">
          {!stacked && "· "}
          {fmt(gap)}&nbsp;left
        </span>
      )}
      {gap < 0 && (
        <span>
          {!stacked && "· "}
          {fmt(-gap)}&nbsp;over
        </span>
      )}
    </span>
  )
}

/** Arrow keys move between cells; left and right only once the caret reaches the edge. */
function moveFocus(e: React.KeyboardEvent<HTMLInputElement>) {
  const el = e.currentTarget
  const [r, c] = [Number(el.dataset.r), Number(el.dataset.c)]
  const atStart = el.selectionStart === 0 && el.selectionEnd === 0
  const atEnd = el.selectionStart === el.value.length
  const target =
    e.key === "ArrowUp"
      ? [r - 1, c]
      : e.key === "ArrowDown" || e.key === "Enter"
        ? [r + 1, c]
        : e.key === "ArrowLeft" && atStart
          ? [r, c - 1]
          : e.key === "ArrowRight" && atEnd
            ? [r, c + 1]
            : null
  if (!target) return
  const next = document.querySelector<HTMLInputElement>(
    `input[data-r="${target[0]}"][data-c="${target[1]}"]`
  )
  if (!next) return
  e.preventDefault()
  el.blur()
  next.focus()
  next.select()
}

type CellProps = {
  date: string
  task: Task
  row: Row | null
  r: number
  c: number
  selected: boolean
  onSelect: Props["onSelect"]
  onMinutes: Props["onMinutes"]
}

const TimeCell = memo(function TimeCell({
  date,
  task,
  row,
  r,
  c,
  selected,
  onSelect,
  onMinutes,
}: CellProps) {
  const shown = row?.minutes ? fmt(row.minutes) : ""
  const [text, setText] = useState(shown)
  const focused = useRef(false)
  useEffect(() => {
    if (!focused.current) setText(shown)
  }, [shown])

  const commit = () => {
    const m = parseDur(text)
    if (m === null) return setText(shown)
    if (m !== (row?.minutes ?? 0)) onMinutes(date, task.id, row?.id ?? null, m)
    setText(m ? fmt(m) : "")
  }

  const pending = row && isPending(row)
  const synced = row?.xeroId && !isDirty(row)
  const noDesc = row && row.minutes > 0 && !row.desc.trim()
  const state = pending ? "Not pushed" : synced ? "In Xero" : ""

  return (
    <div className="relative">
      <input
        aria-label={`${task.name}, ${dayLabel(date, { weekday: "long", day: "numeric", month: "long" })}${state ? `, ${state}` : ""}${noDesc ? ", no description" : ""}`}
        data-r={r}
        data-c={c}
        value={text}
        inputMode="decimal"
        autoComplete="off"
        spellCheck={false}
        placeholder="–"
        title={row?.desc || undefined}
        onChange={(e) => setText(e.target.value)}
        onFocus={(e) => {
          focused.current = true
          e.currentTarget.select()
          onSelect({ date, taskId: task.id, rowId: row?.id ?? null })
        }}
        onBlur={() => {
          focused.current = false
          commit()
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit()
          if (e.key === "Escape") setText(shown)
          moveFocus(e)
        }}
        className={cn(
          "h-8 w-16 rounded-md border border-input bg-transparent px-2 text-right text-sm tabular-nums transition-[border-color,box-shadow,background-color] outline-none",
          "placeholder:text-muted-foreground/50 hover:border-ring/60 focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/40",
          noDesc && "bg-amber-500/10",
          selected && "border-ring ring-2 ring-ring/30"
        )}
      />
      {(pending || synced) && (
        <span
          aria-hidden
          className={cn(
            "absolute top-1 right-1 size-1.5 rounded-full",
            pending ? "bg-amber-500" : "bg-emerald-500"
          )}
        />
      )}
    </div>
  )
})

function TaskMenu({
  label,
  items,
}: {
  label: string
  items: [string, () => void][]
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={label}
          className="opacity-60 hover:opacity-100"
        >
          <EllipsisIcon />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        {items.map(([text, fn]) => (
          <DropdownMenuItem key={text} onSelect={fn}>
            {text}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

export function TimesheetGrid(p: Props) {
  const dates = p.columns.flatMap((c) => ("date" in c ? [c.date] : []))
  const cell = "border-b border-r border-border px-1.5 py-1 align-top"
  const sticky = "sticky left-0 z-10 bg-background"
  const colIndex = new Map(dates.map((d, i) => [d, i]))
  const projects = [...new Set(p.tasks.map((t) => t.projectId))].map((id) =>
    p.tasks.filter((t) => t.projectId === id)
  )

  return (
    <table className="h-fit border-separate border-spacing-0 self-start text-sm">
      <thead>
        <tr>
          <th
            className={cn(
              cell,
              sticky,
              "top-0 z-30 min-w-56 bg-muted text-left font-medium"
            )}
          >
            Task
          </th>
          {p.columns.map((c, i) =>
            "date" in c ? (
              <th
                key={c.date}
                className={cn(
                  cell,
                  "sticky top-0 z-20 bg-muted p-0 text-left font-normal",
                  weekday(c.date) === 1 && "border-l-2 border-l-foreground/15",
                  p.selection.date === c.date && "bg-accent"
                )}
              >
                <button
                  type="button"
                  onClick={() =>
                    p.onSelect({ date: c.date, taskId: null, rowId: null })
                  }
                  className="flex w-full flex-col items-start gap-0.5 px-2 py-1.5 text-xs outline-none focus-visible:ring-3 focus-visible:ring-ring/40"
                >
                  <span className="font-medium text-foreground">
                    {dayLabel(c.date)}
                  </span>
                  <GapLabel {...p.gap(c.date)} stacked />
                </button>
              </th>
            ) : (
              <th
                key={`w${i}`}
                className={cn(
                  cell,
                  "sticky top-0 z-20 bg-muted text-right text-xs font-medium tabular-nums"
                )}
              >
                <div className="text-muted-foreground">Week</div>
                {fmt(sumFor(p.rows, c.week))}
              </th>
            )
          )}
          <th
            className={cn(
              cell,
              "sticky top-0 z-20 bg-muted text-right text-xs font-medium tabular-nums"
            )}
          >
            <div className="text-muted-foreground">Month</div>
            {fmt(sumFor(p.rows, dates))}
          </th>
        </tr>
      </thead>
      <tbody>
        {projects.flatMap((group) => {
          const t = group[0]
          const ids = group.map((x) => x.id)
          return [
            <tr key={`p-${t.projectId}`}>
              <td
                colSpan={p.columns.length + 2}
                className={cn(cell, "bg-muted/60 py-1")}
              >
                <div className="sticky left-1.5 flex w-fit items-center gap-1 font-medium">
                  {t.projectName}
                  <TaskMenu
                    label={`${t.projectName} options`}
                    items={[["Hide Project", () => p.onHide(ids)]]}
                  />
                </div>
              </td>
            </tr>,
            ...group.map((t) => {
              const rowIndex = p.tasks.indexOf(t)
              return (
                <tr key={t.id}>
                  <th className={cn(cell, sticky, "text-left font-normal")}>
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <div className="truncate">{t.name}</div>
                        <label className="my-0.5 flex w-fit cursor-pointer items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground">
                          <Checkbox
                            checked={p.isBillable(t)}
                            onCheckedChange={(v) =>
                              p.onBillable(t.id, v === true)
                            }
                          />
                          Billable
                        </label>
                        <button
                          type="button"
                          onClick={() => p.onEditKeywords(t)}
                          className="max-w-48 truncate text-left text-xs text-muted-foreground hover:text-foreground focus-visible:underline focus-visible:outline-none"
                        >
                          {p.keywords[t.id]?.trim() || "Add draft keywords…"}
                        </button>
                      </div>
                      <TaskMenu
                        label={`${t.name} options`}
                        items={[
                          ["Edit Draft Keywords…", () => p.onEditKeywords(t)],
                          ["Hide Task", () => p.onHide([t.id])],
                        ]}
                      />
                    </div>
                  </th>
                  {p.columns.map((col, i) => {
                    if (!("date" in col))
                      return (
                        <td
                          key={`w${i}`}
                          className={cn(
                            cell,
                            "bg-muted/40 text-right font-medium tabular-nums"
                          )}
                        >
                          {sumFor(p.rows, col.week, t.id)
                            ? fmt(sumFor(p.rows, col.week, t.id))
                            : ""}
                        </td>
                      )
                    const rows = (p.rows[col.date] ?? []).filter(
                      (x) => x.taskId === t.id
                    )
                    return (
                      <td
                        key={col.date}
                        className={cn(
                          cell,
                          weekday(col.date) === 1 &&
                            "border-l-2 border-l-foreground/15",
                          p.selection.date === col.date && "bg-accent/50"
                        )}
                      >
                        <div className="flex flex-col gap-1">
                          {(rows.length ? rows : [null]).map((row, k) => (
                            <TimeCell
                              key={row?.id ?? "new"}
                              date={col.date}
                              task={t}
                              row={row}
                              r={rowIndex}
                              c={k === 0 ? colIndex.get(col.date)! : -1}
                              selected={
                                p.selection.date === col.date &&
                                p.selection.taskId === t.id &&
                                p.selection.rowId === (row?.id ?? null)
                              }
                              onSelect={p.onSelect}
                              onMinutes={p.onMinutes}
                            />
                          ))}
                        </div>
                      </td>
                    )
                  })}
                  <td
                    className={cn(
                      cell,
                      "bg-muted/40 text-right font-medium tabular-nums"
                    )}
                  >
                    {sumFor(p.rows, dates, t.id)
                      ? fmt(sumFor(p.rows, dates, t.id))
                      : ""}
                  </td>
                </tr>
              )
            }),
          ]
        })}
      </tbody>
    </table>
  )
}
