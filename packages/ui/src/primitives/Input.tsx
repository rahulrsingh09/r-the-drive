import * as React from 'react';
import { cn } from '../lib/utils';

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  error?: string;
  hint?: string;
  iconLeft?: React.ReactNode;
  iconRight?: React.ReactNode;
}

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ label, error, hint, iconLeft, iconRight, className, id, ...props }, ref) => {
    const inputId = id ?? label?.toLowerCase().split(/\s+/).join('-');
    const feedback = error
      ? { text: error, border: 'border-status-danger/50 focus:ring-status-danger focus:border-status-danger', color: 'text-xs text-status-danger' }
      : { text: hint, border: 'border-border hover:border-border-strong', color: 'text-xs text-fg-tertiary' };
    const adornments = [iconLeft, iconRight].map((icon, index) => icon && <span key={index} className={[
      'absolute left-3 top-1/2 -translate-y-1/2 text-fg-tertiary',
      'absolute right-3 top-1/2 -translate-y-1/2 text-fg-tertiary',
    ][index]}>{icon}</span>);

    return (
      <div className="flex flex-col gap-1.5">
        {label && (
          <label htmlFor={inputId} className="text-sm font-medium text-fg-secondary">
            {label}
          </label>
        )}
        <div className="relative">
          {adornments[0]}
          <input
            ref={ref}
            id={inputId}
            className={cn(
              'w-full h-9 rounded-lg border bg-bg-elevated text-fg text-base sm:text-sm',
              'placeholder:text-fg-tertiary',
              'transition-colors duration-150',
              'focus:outline-none focus:ring-1 focus:ring-accent focus:border-accent',
              feedback.border,
              iconLeft ? 'pl-9' : 'pl-3',
              iconRight ? 'pr-9' : 'pr-3',
              className
            )}
            {...props}
          />
          {adornments[1]}
        </div>
        {feedback.text && <p className={feedback.color}>{feedback.text}</p>}
      </div>
    );
  }
);

Input.displayName = 'Input';
