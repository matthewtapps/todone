import { useId, type ReactNode } from "react"
import {
  CalendarClockIcon,
  CheckIcon,
  ChevronRightIcon,
  CircleDotIcon,
  GitCommitHorizontalIcon,
  GitMergeIcon,
  GitPullRequestArrowIcon,
  MessageSquareIcon,
  PlusIcon,
  SparklesIcon,
  XIcon,
  type LucideIcon,
} from "lucide-react"

import { GapLabel } from "@/components/timesheet-grid"
import { Button } from "@/components/ui/button"
import {
  Sidebar,
  SidebarFooter,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
} from "@/components/ui/sidebar"
import { Textarea } from "@/components/ui/textarea"
import { useFairHeights } from "@/lib/fair-heights"
import { useStored } from "@/lib/storage"
import { cn } from "@/lib/utils"
import type { GitlabDay } from "@/lib/api"
import { dayLabel, fmt, minutesBetween } from "@/lib/time"
import type { TodoneDay } from "@/lib/types"

export type GitlabState =
  { status: "error"; message: string } | { status: "ready"; data: GitlabDay }

type Props = {
  date: string
  day: TodoneDay | undefined
  gitlab: GitlabState | undefined
  gap: { total: number; gap: number }
  /** Name of the selected cell's task, or null when only a day is selected. */
  taskName: string | null
  desc: string
  onDesc: (desc: string) => void
  onDraftDay: () => void
  onRetryGitlab: () => void
}

const GITLAB_ICONS: Record<string, LucideIcon> = {
  push: GitCommitHorizontalIcon,
  comment: MessageSquareIcon,
  approve: CheckIcon,
  open: GitPullRequestArrowIcon,
  merge: GitMergeIcon,
  close: XIcon,
}

const time = (iso: string) =>
  new Date(iso).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  })

function Item({
  icon,
  text,
  meta,
  sub = false,
  taskName,
  onAdd,
}: {
  icon: ReactNode
  text: string
  meta?: string
  sub?: boolean
  taskName: string | null
  onAdd: (text: string) => void
}) {
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        disabled={!taskName}
        onClick={() => onAdd(text)}
        title={
          taskName
            ? `Add to ${taskName} description`
            : "Select a time cell first"
        }
        className="h-auto items-start py-1.5 whitespace-normal data-[sub=true]:pl-6"
        data-sub={sub}
      >
        {icon}
        <span className="flex-1 overflow-visible! whitespace-normal!">
          {text}
        </span>
        {meta && (
          <span className="text-xs whitespace-nowrap text-muted-foreground tabular-nums">
            {meta}
          </span>
        )}
      </SidebarMenuButton>
    </SidebarMenuItem>
  )
}

/** One resizable source list; it scrolls on its own so the others stay put. */
/**
 * One source list, sized to its content. When they don't all fit, the longest
 * are capped and scroll on their own (see useFairHeights); the heading collapses it.
 */
function Segment({
  id,
  title,
  open,
  onToggle,
  maxHeight,
  busy,
  children,
}: {
  id: string
  title: string
  open: boolean
  onToggle: () => void
  maxHeight: number | undefined
  busy?: boolean
  children: ReactNode
}) {
  return (
    <section
      data-segment={id}
      className="flex shrink-0 flex-col border-t border-sidebar-border"
    >
      <h3>
        <button
          type="button"
          aria-expanded={open}
          onClick={onToggle}
          className="flex w-full items-center gap-1 px-4 py-2 text-xs font-medium text-sidebar-foreground/70 outline-none hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring"
        >
          <ChevronRightIcon
            aria-hidden
            className={cn("size-3.5 transition-transform", open && "rotate-90")}
          />
          {title}
        </button>
      </h3>
      {open && (
        <div
          data-scroll
          aria-busy={busy}
          style={{ maxHeight }}
          className="overflow-y-auto overscroll-contain px-2 pb-2"
        >
          <div data-content>{children}</div>
        </div>
      )}
    </section>
  )
}

const empty = (text: string) => (
  <p className="px-2 text-sm text-muted-foreground">{text}</p>
)

export function DaySidebar({
  date,
  day,
  gitlab,
  gap,
  taskName,
  desc,
  onDesc,
  onDraftDay,
  onRetryGitlab,
}: Props) {
  const descId = useId()
  const add = (text: string) => onDesc(desc ? `${desc}\n${text}` : text)
  const events = day?.events ?? []
  const did = day?.did ?? []
  // Branch deletions are noise for a timesheet.
  const groups =
    gitlab?.status === "ready"
      ? (gitlab.data.groups ?? [])
          .map((g) => ({
            ...g,
            lines: g.lines.filter(
              (l) => !(l.kind === "other" && l.text.startsWith("deleted"))
            ),
          }))
          .filter((g) => g.lines.length)
      : []
  const gitlabEnabled = !(gitlab?.status === "ready" && !gitlab.data.enabled)
  const [collapsed, setCollapsed] = useStored<string[]>(
    "timesheet:collapsed",
    []
  )
  const openKey = ["todone", "calendar", ...(gitlabEnabled ? ["gitlab"] : [])]
    .filter((id) => !collapsed.includes(id))
    .join()
  const { container, heights } = useFairHeights(openKey)
  const segment = (id: string) => ({
    id,
    maxHeight: heights[id],
    open: !collapsed.includes(id),
    onToggle: () =>
      setCollapsed((c) =>
        c.includes(id) ? c.filter((x) => x !== id) : [...c, id]
      ),
  })

  return (
    <Sidebar>
      <SidebarHeader className="gap-1 px-4 pt-4">
        <h2 className="text-base font-semibold text-balance">
          {dayLabel(date, { weekday: "long", day: "numeric", month: "long" })}
        </h2>
        <div className="text-sm">
          <GapLabel {...gap} />
        </div>
      </SidebarHeader>
      <div className="flex shrink-0 flex-col gap-1.5 px-4 pb-3">
        <label
          htmlFor={descId}
          className="text-xs font-medium text-sidebar-foreground/70"
        >
          {taskName ? `${taskName} Description` : "Description"}
        </label>
        <Textarea
          id={descId}
          value={desc}
          disabled={!taskName}
          rows={6}
          maxLength={1000}
          placeholder={taskName ? "What you worked on…" : ""}
          onChange={(e) => onDesc(e.target.value)}
          className="min-h-32 resize-y bg-background text-base md:text-sm"
        />
        <p className="text-xs text-muted-foreground">
          {taskName
            ? "Saved as you type. Click an item below to add it."
            : "Select a time cell to edit its description."}
        </p>
      </div>
      <div ref={container} className="flex min-h-0 flex-1 flex-col">
        <Segment title="todone" {...segment("todone")}>
          {did.length ? (
            <SidebarMenu>
              {did.map((line, i) => (
                <Item
                  key={i}
                  icon={<PlusIcon aria-hidden className="mt-0.5 opacity-50" />}
                  text={line.replace(/^\s*-\s*/, "")}
                  sub={/^\s*-/.test(line)}
                  taskName={taskName}
                  onAdd={add}
                />
              ))}
            </SidebarMenu>
          ) : (
            empty("Nothing in todone for this day.")
          )}
        </Segment>
        <Segment title="Calendar" {...segment("calendar")}>
          {events.length ? (
            <SidebarMenu>
              {events.map((e) => (
                <Item
                  key={`${e.start}-${e.summary}`}
                  icon={
                    <CalendarClockIcon
                      aria-hidden
                      className="mt-0.5 text-emerald-600 dark:text-emerald-400"
                    />
                  }
                  text={e.summary}
                  meta={
                    e.all_day
                      ? "All day"
                      : `${time(e.start)}–${time(e.end)} · ${fmt(minutesBetween(e.start, e.end))}`
                  }
                  taskName={taskName}
                  onAdd={add}
                />
              ))}
            </SidebarMenu>
          ) : (
            empty("No meetings.")
          )}
        </Segment>
        {gitlabEnabled && (
          <Segment title="GitLab" busy={!gitlab} {...segment("gitlab")}>
            {!gitlab ? (
              <SidebarMenu>
                <span className="sr-only">Loading GitLab activity…</span>
                {Array.from({ length: 3 }, (_, i) => (
                  <SidebarMenuItem key={i}>
                    <SidebarMenuSkeleton showIcon />
                  </SidebarMenuItem>
                ))}
              </SidebarMenu>
            ) : gitlab.status === "error" ? (
              <div className="flex flex-col items-start gap-2 px-2 text-sm">
                <p className="text-muted-foreground">{gitlab.message}</p>
                <Button variant="outline" size="sm" onClick={onRetryGitlab}>
                  Try Again
                </Button>
              </div>
            ) : groups.length ? (
              <SidebarMenu>
                {groups.map((g) => (
                  <li key={g.project} className="flex flex-col">
                    <div className="px-2 pt-2 pb-0.5 text-xs font-medium text-muted-foreground">
                      {g.project}
                    </div>
                    <SidebarMenu>
                      {g.lines.map((l, i) => {
                        const Icon = GITLAB_ICONS[l.kind] ?? CircleDotIcon
                        return (
                          <Item
                            key={i}
                            icon={
                              <Icon aria-hidden className="mt-0.5 opacity-70" />
                            }
                            text={l.text}
                            taskName={taskName}
                            onAdd={(text) => add(`${g.project}: ${text}`)}
                          />
                        )
                      })}
                    </SidebarMenu>
                  </li>
                ))}
              </SidebarMenu>
            ) : (
              empty("No GitLab activity.")
            )}
          </Segment>
        )}
      </div>
      <SidebarFooter className="p-4">
        <Button
          variant="outline"
          onClick={onDraftDay}
          disabled={!events.length && !did.length}
        >
          <SparklesIcon data-icon="inline-start" />
          Draft This Day
        </Button>
      </SidebarFooter>
    </Sidebar>
  )
}
