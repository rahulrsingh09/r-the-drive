import React from 'react';

export function useMeasuredWidgetHeight(fallback = 260, minimum = 140) {
  const ref = React.useRef<HTMLDivElement | null>(null);
  const [pixels, measure] = React.useReducer((previous: number, next: number) => next > 0 ? Math.floor(next) : previous, fallback);
  React.useEffect(() => {
    const target = ref.current;
    if (typeof ResizeObserver !== 'function' || target === null) return;
    const sample = () => measure(target.getBoundingClientRect().height);
    const observer = new ResizeObserver(sample);
    sample();
    observer.observe(target);
    return () => { observer.disconnect(); };
  }, []);
  return { ref, height: Math.max(minimum, pixels || fallback) };
}
