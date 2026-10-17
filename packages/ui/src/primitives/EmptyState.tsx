import * as React from 'react';
import { cn } from '../lib/utils';
import { Button, type ButtonProps } from './Button';

export interface EmptyStateProps {
  icon?: React.ReactNode | undefined;
  title: string;
  description?: string | undefined;
  action?: {
    label: string;
    onClick: () => void;
    variant?: ButtonProps['variant'] | undefined;
  } | undefined;
  className?: string | undefined;
}

export function EmptyState({
  icon,
  title,
  description,
  action,
  className,
}: EmptyStateProps) {
  const supporting = [
    { visible: Boolean(description), element: <p key="description" className="mt-1 text-sm text-fg-tertiary max-w-xs">{description}</p> },
    { visible: Boolean(action), element: <Button key="action" variant={action?.variant ?? 'secondary'} size="sm" className="mt-4" onClick={action?.onClick}>{action?.label}</Button> },
  ];
  return (
    <div className={cn('flex flex-col items-center justify-center py-16 text-center', className)}>
      {icon && (
        <div className="mb-4 text-fg-tertiary [&>svg]:h-10 [&>svg]:w-10">
          {icon}
        </div>
      )}
      <h3 className="text-base font-semibold text-fg">{title}</h3>
      {supporting.filter(slot => slot.visible).map(slot => slot.element)}
    </div>
  );
}
