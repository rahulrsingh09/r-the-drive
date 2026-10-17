import * as React from 'react';
import { cn } from '../lib/utils';

export interface SwitchProps extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'onChange'> {
  checked: boolean;
  onChange?: (checked: boolean) => void;
  size?: 'sm' | 'md';
}

/** Compact, keyboard-accessible boolean control shared by settings and editors. */
export const Switch = React.forwardRef<HTMLButtonElement, SwitchProps>(
  ({ checked, onChange, size = 'sm', className, disabled, onClick, ...props }, ref) => {
    const geometry = size === 'sm'
      ? { track: 'h-[22px] w-10', thumb: 'top-[2px] h-4 w-4', off: 'translate-x-[2px] bg-fg-tertiary', on: 'translate-x-[22px] bg-fg' }
      : { track: 'h-6 w-11', thumb: 'top-1 h-4 w-4', off: 'translate-x-1 bg-fg-tertiary', on: 'translate-x-6 bg-fg' };
    const state = checked
      ? { track: 'border-accent/60 bg-accent', thumb: geometry.on }
      : { track: 'border-border bg-bg-elevated', thumb: geometry.off };
    const handleClick: React.MouseEventHandler<HTMLButtonElement> = event => {
      onClick?.(event);
      if (event.defaultPrevented) return;
      if (disabled) return;
      onChange?.(!checked);
    };
    return (
    <button
      {...props}
      ref={ref}
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={handleClick}
      className={cn(
        'relative inline-flex shrink-0 items-center rounded-full border transition-colors duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-bg-page disabled:cursor-not-allowed disabled:opacity-40',
        geometry.track,
        state.track,
        disabled ? 'cursor-not-allowed' : 'cursor-pointer',
        className,
      )}
    >
      <span
        aria-hidden="true"
        className={cn(
          'pointer-events-none absolute rounded-full shadow-sm transition-transform duration-150',
          geometry.thumb,
          state.thumb,
        )}
      />
    </button>
  );
  },
);

Switch.displayName = 'Switch';
