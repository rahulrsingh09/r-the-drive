import * as React from 'react';

export function useChartWidth() {
  const [element, ref] = React.useState<HTMLDivElement | null>(null);
  const [width, measure] = React.useReducer((current: number, measurement: number) =>
    measurement > 0 ? Math.max(1, Math.round(measurement)) : current, 960);
  React.useLayoutEffect(() => {
    if (element == null) return;
    const padding = getComputedStyle(element);
    measure(element.clientWidth - parseFloat(padding.paddingLeft) - parseFloat(padding.paddingRight));
    const observer = new ResizeObserver(entries => {
      const first = entries.at(0);
      if (first) measure(first.contentRect.width);
    });
    observer.observe(element);
    return observer.disconnect.bind(observer);
  }, [element]);
  return { width, ref };
}
