import React from 'react';

export interface SegmentedOption { value: string; label: string }

export interface SegmentedProps {
  label: string;
  value: string | null;
  options: SegmentedOption[];
  onChange: (value: string) => void;
  small?: boolean;
  className?: string;
  disabled?: boolean;
  /** Extra attributes on each button, for example data-tag. */
  buttonProps?: (option: SegmentedOption) => Record<string, string>;
}

/** Segmented control with a sliding ink under the pressed option. */
export function Segmented({ label, value, options, onChange, small = false, className, disabled = false, buttonProps }: SegmentedProps) {
  const groupRef = React.useRef<HTMLDivElement>(null);
  const [ink, setInk] = React.useState<{ left: number; width: number } | null>(null);
  React.useLayoutEffect(() => {
    const group = groupRef.current;
    if (!group) return undefined;
    const place = () => {
      const pressed = group.querySelector<HTMLButtonElement>('button[aria-pressed="true"]');
      setInk(pressed && pressed.offsetWidth ? { left: pressed.offsetLeft, width: pressed.offsetWidth } : null);
    };
    place();
    if (typeof ResizeObserver === 'undefined') return undefined;
    const observer = new ResizeObserver(place);
    observer.observe(group);
    return () => observer.disconnect();
  }, [value, options.length]);

  return (
    <div ref={groupRef} className={['r-act-seg', small ? 'small' : '', className ?? ''].filter(Boolean).join(' ')} role="group" aria-label={label}>
      <span className="r-act-seg-ink" aria-hidden="true" style={ink ? { left: ink.left, width: ink.width } : { opacity: 0 }} />
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          aria-pressed={value === option.value}
          disabled={disabled}
          onClick={() => onChange(option.value)}
          {...buttonProps?.(option)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
