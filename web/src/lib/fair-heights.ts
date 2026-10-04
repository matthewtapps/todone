import { useLayoutEffect, useRef, useState } from "react"

/**
 * Splits `available` between items wanting `wants`: items smaller than an even
 * share get all they want, and the rest divide what's left equally.
 */
export function fairShares(wants: number[], available: number): number[] {
  const order = wants.map((w, i) => [w, i] as const).sort((a, b) => a[0] - b[0])
  const out = new Array<number>(wants.length)
  let left = Math.max(0, available)
  order.forEach(([want, i], k) => {
    out[i] = Math.floor(Math.min(want, left / (order.length - k)))
    left -= out[i]
  })
  return out
}

/**
 * Caps each `[data-scroll]` region inside the container so every section fits.
 * Sections mark themselves with `data-segment={id}` and wrap content in `[data-content]`.
 * `key` must change whenever a section opens or closes.
 */
export function useFairHeights(key: string) {
  const container = useRef<HTMLDivElement>(null)
  const [heights, setHeights] = useState<Record<string, number>>({})

  useLayoutEffect(() => {
    const el = container.current
    if (!el) return
    const measure = () => {
      const sections = [...el.querySelectorAll<HTMLElement>("[data-segment]")]
      let chrome = 0
      const open: { id: string; want: number }[] = []
      for (const s of sections) {
        const scroll = s.querySelector<HTMLElement>("[data-scroll]")
        const content = s.querySelector<HTMLElement>("[data-content]")
        chrome += s.offsetHeight - (scroll?.offsetHeight ?? 0)
        if (scroll && content) {
          const pad =
            scroll.offsetHeight -
            scroll.clientHeight +
            parseFloat(getComputedStyle(scroll).paddingBottom)
          open.push({
            id: s.dataset.segment!,
            want: content.offsetHeight + pad,
          })
        }
      }
      const shares = fairShares(
        open.map((o) => o.want),
        el.clientHeight - chrome
      )
      const next = Object.fromEntries(open.map((o, i) => [o.id, shares[i]]))
      setHeights((prev) =>
        JSON.stringify(prev) === JSON.stringify(next) ? prev : next
      )
    }
    // Observing fires once straight away, then on any size change.
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    el.querySelectorAll("[data-content]").forEach((c) => ro.observe(c))
    return () => ro.disconnect()
  }, [key])

  return { container, heights }
}
