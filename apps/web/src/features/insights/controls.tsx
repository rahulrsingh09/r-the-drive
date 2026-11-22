import React from 'react';
import { CATEGORIES, CATEGORY_COLOR, CATEGORY_LABEL } from './insightsModel';
import { Legend } from './chartKit';

/** Places the sliding ink under the pressed (or current) item of a segmented control. */
export function useSegInk(deps: React.DependencyList) {
  const ref = React.useRef<HTMLDivElement>(null);
  const [ink, setInk] = React.useState<{ left: number; width: number } | null>(null);
  React.useLayoutEffect(() => {
    const root = ref.current;
    if (!root) return;
    const place = () => {
      const active = root.querySelector<HTMLElement>('[aria-pressed="true"], [aria-current="page"]');
      setInk(active ? { left: active.offsetLeft, width: active.offsetWidth } : null);
    };
    place();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(place);
    observer.observe(root);
    return () => observer.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  const inkEl = <span className="ri-seg-ink" aria-hidden="true" style={ink ? { left: ink.left, width: ink.width } : { opacity: 0 }} />;
  return { ref, inkEl };
}

/** A one-line segmented control of buttons with aria-pressed. */
export function Seg<T extends string>({ label, value, options, onChange, small, disabled, className }: {
  label: string;
  value: T | null;
  options: ReadonlyArray<readonly [T, string]>;
  onChange: (value: T) => void;
  small?: boolean;
  disabled?: boolean;
  className?: string;
}) {
  const { ref, inkEl } = useSegInk([value]);
  return (
    <div ref={ref} className={`ri-seg${small ? ' ri-seg-small' : ''}${className ? ` ${className}` : ''}`} role="group" aria-label={label}>
      {inkEl}
      {options.map(([key, text]) => (
        <button key={key} type="button" aria-pressed={value === key} disabled={disabled} onClick={() => onChange(key)}>{text}</button>
      ))}
    </div>
  );
}

export const CategoryLegend = () => <Legend items={CATEGORIES.map(c => ({ label: CATEGORY_LABEL[c], color: CATEGORY_COLOR[c] }))} />;

export function Spec({ items, className, index }: {
  items: ReadonlyArray<{ label: string; value: React.ReactNode; note?: React.ReactNode | undefined }>;
  className?: string | undefined;
  index?: number | undefined;
}) {
  return (
    <dl className={`ri-spec${className ? ` ${className}` : ''}`} style={index != null ? { '--i': index } as React.CSSProperties : undefined}>
      {items.map(item => (
        <div key={item.label}>
          <dt className="ri-label">{item.label}</dt>
          <dd className="ri-v ri-num">{item.value}</dd>
          {item.note && <dd className="ri-muted ri-small">{item.note}</dd>}
        </div>
      ))}
    </dl>
  );
}

/** The big number row of a card: the label and number on the left, a short summary on the right. */
export function Kpi({ label, children, aside }: { label: string; children: React.ReactNode; aside?: React.ReactNode }) {
  return (
    <div className="ri-kpi">
      <div><div className="ri-label">{label}</div><div className="ri-big">{children}</div></div>
      {aside && <div className="ri-label ri-kpi-aside">{aside}</div>}
    </div>
  );
}

const storageKey = (userId: string | null | undefined, name: string) => `r-insights-${name}:${userId ?? 'anonymous'}`;

/** A small value stored per user in local storage. */
export function useStoredValue<T>(userId: string | null | undefined, name: string, fallback: T, valid: (value: unknown) => value is T) {
  const key = storageKey(userId, name);
  const read = React.useCallback(() => {
    try {
      const raw = window.localStorage.getItem(key);
      const parsed: unknown = raw ? JSON.parse(raw) : null;
      return valid(parsed) ? parsed : fallback;
    } catch {
      return fallback;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  const [value, setValue] = React.useState<T>(read);
  React.useEffect(() => { setValue(read()); }, [read]);
  const save = React.useCallback((next: T) => {
    setValue(next);
    try { window.localStorage.setItem(key, JSON.stringify(next)); } catch { /* storage can be blocked */ }
  }, [key]);
  return [value, save] as const;
}
