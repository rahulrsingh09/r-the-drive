import * as React from 'react';

export function useChartWidth() {
  const [element, setElement] = React.useState<HTMLDivElement | null>(null);
  const [width, setWidth] = React.useState(960);
  React.useLayoutEffect(() => {
    if (!element) return;
    const update = (value: number) => {
      if (value > 0) setWidth(Math.max(1, Math.round(value)));
    };
    const style = getComputedStyle(element);
    update(element.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight));
    const observer = new ResizeObserver(entries => {
      const entry = entries[0];
      if (entry) update(entry.contentRect.width);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);
  return { width, ref: setElement };
}
