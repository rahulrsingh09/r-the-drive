import type uPlot from 'uplot';

interface CursorSelection {
  controlled: boolean;
  showTooltip: boolean;
  selectedValue(): number | null | undefined;
  onIndex(index: number | null): void;
  render(chart: uPlot, index: number): void;
  clear(): void;
}

/** Keep external selection writes separate from cursor notifications and tooltip visibility. */
export function createRichCursorController(selection: CursorSelection) {
  let writes = 0;
  return {
    read(chart: uPlot) {
      if (!selection.showTooltip) { selection.clear(); return; }
      const index = chart.cursor.idx;
      if (index == null || index < 0) {
        if (selection.controlled && writes === 0) return;
        if (writes === 0) selection.onIndex(null);
        selection.clear();
        return;
      }
      if (!selection.controlled && writes === 0) selection.onIndex(index);
      selection.render(chart, index);
    },
    apply(chart: uPlot) {
      if (!selection.controlled) return;
      const value = selection.selectedValue();
      const position = value == null ? { left: -10, top: -10 } : { left: chart.valToPos(value, 'x'), top: 12 };
      writes += 1;
      try { chart.setCursor(position); } finally { writes -= 1; }
    },
  };
}

export function observeChartFrame(element: Element, refresh: () => void) {
  let frame = 0;
  const observer = new ResizeObserver(() => {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(refresh);
  });
  observer.observe(element);
  return () => { observer.disconnect(); cancelAnimationFrame(frame); };
}
