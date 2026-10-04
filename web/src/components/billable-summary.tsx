import { Label, Pie, PieChart } from "recharts"

import { ChartContainer, type ChartConfig } from "@/components/ui/chart"
import { fmt } from "@/lib/time"

const config = {
  billable: { label: "Billable", color: "oklch(0.62 0.17 250)" },
  nonBillable: { label: "Non-billable", color: "oklch(0.78 0.02 250)" },
} satisfies ChartConfig

export function BillableSummary({
  period,
  billable,
  nonBillable,
}: {
  period: string
  billable: number
  nonBillable: number
}) {
  const total = billable + nonBillable
  const pct = (m: number) => (total ? Math.round((m / total) * 100) : 0)
  const data = total
    ? [
        { key: "billable", minutes: billable, fill: "var(--color-billable)" },
        {
          key: "nonBillable",
          minutes: nonBillable,
          fill: "var(--color-nonBillable)",
        },
      ]
    : [{ key: "none", minutes: 1, fill: "var(--muted)" }]
  const rows = [
    ["billable", billable],
    ["nonBillable", nonBillable],
  ] as const

  return (
    <section
      aria-label={`${period} billable split`}
      className="flex shrink-0 items-center gap-4 border-b px-4 py-2"
    >
      <ChartContainer config={config} className="aspect-square h-20 shrink-0">
        <PieChart>
          <Pie
            data={data}
            dataKey="minutes"
            nameKey="key"
            innerRadius={26}
            outerRadius={38}
            strokeWidth={2}
            isAnimationActive={false}
          >
            <Label
              content={({ viewBox }) =>
                viewBox && "cx" in viewBox ? (
                  <text
                    x={viewBox.cx}
                    y={viewBox.cy}
                    textAnchor="middle"
                    dominantBaseline="middle"
                    className="fill-foreground text-sm font-semibold tabular-nums"
                  >
                    {total ? `${pct(billable)}%` : "–"}
                  </text>
                ) : null
              }
            />
          </Pie>
        </PieChart>
      </ChartContainer>
      <div className="flex flex-col gap-1">
        <h2 className="text-xs font-medium text-muted-foreground">
          {period} Billable Split
        </h2>
        {total ? (
          <dl className="grid grid-cols-[auto_auto_auto] items-center gap-x-3 gap-y-0.5 text-sm tabular-nums">
            {rows.map(([key, minutes]) => (
              <div key={key} className="contents">
                <dt className="flex items-center gap-1.5">
                  <span
                    aria-hidden
                    className="size-2.5 rounded-sm"
                    style={{ background: config[key].color }}
                  />
                  {config[key].label}
                </dt>
                <dd className="text-right">{fmt(minutes)}</dd>
                <dd className="text-right text-muted-foreground">
                  {pct(minutes)}%
                </dd>
              </div>
            ))}
          </dl>
        ) : (
          <p className="text-sm text-muted-foreground">No time logged yet.</p>
        )}
      </div>
    </section>
  )
}
