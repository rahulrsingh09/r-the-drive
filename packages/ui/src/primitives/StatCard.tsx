import * as React from 'react';
import { TrendingUp, TrendingDown, Minus } from 'lucide-react';
import { cn } from '../lib/utils';
import { Card } from './Card';

export interface StatCardProps {
  label: string;
  value: string | number;
  unit?: string;
  delta?: number;
  deltaLabel?: string;
  detail?: string;
  icon?: React.ReactNode;
  accent?: boolean;
  className?: string;
}

export function StatCard({
  label,
  value,
  unit,
  delta,
  deltaLabel,
  detail,
  icon,
  accent = false,
  className,
}: StatCardProps) {
  let change: React.ReactNode = detail ? <div className="mt-2 text-xs text-fg-tertiary">{detail}</div> : null;
  if (delta !== undefined) {
    const sign = delta === 0 ? 0 : delta > 0 ? 1 : 2;
    const styles = ['text-fg-tertiary', 'text-status-positive', 'text-status-danger'];
    const Glyph = [Minus, TrendingUp, TrendingDown][sign]!;
    const prefix = sign === 1 ? '+' : '';
    change = <div className={cn('mt-2 flex items-center gap-1 text-xs font-medium', styles[sign])}>
      <Glyph className="h-3 w-3" /><span>{prefix}{delta}{deltaLabel && ` ${deltaLabel}`}</span>
    </div>;
  }

  return (
    <Card
      className={cn(
        accent && 'border-accent/30 shadow-glow-sm',
        className
      )}
    >
      <div className="flex items-start justify-between">
        <p className="text-xs font-medium text-fg-tertiary uppercase tracking-wider">{label}</p>
        {icon && (
          <span className={cn('text-fg-tertiary', accent && 'text-accent')}>
            {icon}
          </span>
        )}
      </div>

      <div className="mt-2 flex items-baseline gap-1">
        <span className={cn(
          'text-2xl font-semibold font-mono tabular-nums tracking-tight',
          accent ? 'text-accent' : 'text-fg'
        )}>
          {value}
        </span>
        {unit && <span className="text-sm text-fg-tertiary">{unit}</span>}
      </div>

      {change}
    </Card>
  );
}
