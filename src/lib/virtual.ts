// Windowing maths for fixed-height rows. Pure, so it can be tested without a DOM.

export interface Range {
  /** First row to render (inclusive). */
  start: number;
  /** One past the last row to render. */
  end: number;
}

/**
 * Which rows to render for a scroll position. `overscan` extra rows are kept
 * above and below so fast scrolling does not show blanks.
 */
export function visibleRange(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  count: number,
  overscan = 4,
): Range {
  if (count <= 0 || rowHeight <= 0) return { start: 0, end: 0 };
  const viewport = Math.max(0, viewportHeight);
  // The list can shrink under a scrolled viewport (a filter, a deletion);
  // without this clamp `start` would pass `end` and nothing would render.
  const maxTop = Math.max(0, count * rowHeight - viewport);
  const top = Math.min(Math.max(0, scrollTop), maxTop);
  const first = Math.floor(top / rowHeight);
  const last = Math.ceil((top + viewport) / rowHeight);
  return {
    start: Math.max(0, first - overscan),
    end: Math.min(count, last + overscan),
  };
}

/** True when the viewport is within `threshold` rows of the end of the list. */
export function nearEnd(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  count: number,
  threshold = 10,
): boolean {
  if (count <= 0) return false;
  const bottomRow = (scrollTop + viewportHeight) / rowHeight;
  return bottomRow >= count - threshold;
}
