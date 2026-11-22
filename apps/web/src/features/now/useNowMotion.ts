import React from 'react';

export function prefersReducedMotion() {
  return typeof window !== 'undefined' && Boolean(window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
}

/** True when the owner allows motion. Read once per mount, so a view does not change style mid-animation. */
export function useMotionAllowed() {
  const [allowed] = React.useState(() => !prefersReducedMotion());
  return allowed;
}

/** Counts from 0 to the first known value when `animate` is true; afterwards it follows the value directly. */
export function useCountUp(value: number | null, animate: boolean, duration = 900) {
  const [shown, setShown] = React.useState<number | null>(animate ? (value == null ? null : 0) : value);
  const counted = React.useRef(!animate);
  React.useEffect(() => {
    if (value == null) { setShown(null); return; }
    if (counted.current) { setShown(value); return; }
    counted.current = true;
    const began = performance.now();
    let frame = 0;
    const tick = (time: number) => {
      const progress = Math.min(1, (time - began) / duration);
      setShown(value * (1 - Math.pow(1 - progress, 3)));
      if (progress < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => { cancelAnimationFrame(frame); setShown(value); };
  }, [value, duration]);
  return shown;
}

/** Becomes true once, two frames after mount (the trigger for CSS transitions to their final state). */
export function useArrived(animate: boolean) {
  const [arrived, setArrived] = React.useState(!animate);
  React.useEffect(() => {
    if (arrived) return;
    let second = 0;
    const first = requestAnimationFrame(() => { second = requestAnimationFrame(() => setArrived(true)); });
    return () => { cancelAnimationFrame(first); cancelAnimationFrame(second); };
  }, [arrived]);
  return arrived;
}
