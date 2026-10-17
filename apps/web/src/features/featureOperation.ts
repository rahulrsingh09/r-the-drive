import { useCallback, useEffect, useRef, useState } from 'react';

export function useFeatureOperation() {
  const generation = useRef(0);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; generation.current += 1; };
  }, []);
  const begin = useCallback(() => {
    const ticket = ++generation.current;
    return () => mounted.current && generation.current === ticket;
  }, []);
  const cancel = useCallback(() => { generation.current += 1; }, []);
  return { begin, cancel };
}

export function downloadFeatureFile(blob: Blob, fileName: string) {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  let clicked = false;
  try {
    Object.assign(anchor, { href: url, download: fileName, rel: 'noopener' });
    document.body.appendChild(anchor);
    anchor.click();
    clicked = true;
  } finally {
    anchor.remove();
    if (clicked) window.setTimeout(() => URL.revokeObjectURL(url), 1000);
    else URL.revokeObjectURL(url);
  }
}

export function useTimedAcknowledgement(duration: number) {
  const [visible, setVisible] = useState(false);
  const timers = useRef(new Set<ReturnType<typeof setTimeout>>());
  useEffect(() => {
    const owned = timers.current;
    return () => {
      for (const timer of owned) clearTimeout(timer);
      owned.clear();
    };
  }, []);
  const acknowledge = () => {
    setVisible(true);
    const timer = setTimeout(() => {
      timers.current.delete(timer);
      setVisible(false);
    }, duration);
    timers.current.add(timer);
  };
  return [visible, acknowledge] as const;
}
