import * as React from 'react';
import { cn } from '../lib/utils';
import { presentationClasses } from '../lib/presentationClasses';

export interface SkeletonProps extends React.HTMLAttributes<HTMLDivElement> {
  rounded?: boolean;
}

export function Skeleton({ className, rounded = false, ...props }: SkeletonProps) {
  return (
    <div
      className={cn(
        'animate-pulse bg-bg-elevated',
        presentationClasses([[rounded, { true: 'rounded-full', false: 'rounded-lg' }]]),
        className
      )}
      {...props}
    />
  );
}

export function ChartSkeleton({ className, height = 240 }: { className?: string; height?: number }) {
  const bars = [52, 68, 44, 74, 58, 86, 63, 49, 78, 56, 71, 61].map(height => ({ height: `${height}%` }));
  const ticks = (count: number, className: string) => Array.from({ length: count }, (_, index) => <Skeleton key={index} className={className} />);

  return (
    <div
      className={cn('flex flex-col gap-3 rounded-lg border border-border/70 bg-bg-surface/70 p-3', className)}
      style={{ height }}
    >
      <div className="flex items-end gap-2" style={{ height: Math.max(120, height - 54) }}>
        <div className="flex h-full flex-col justify-between py-1">
          {ticks(5, "w-8 h-2.5")}
        </div>
        <div className="flex h-full flex-1 items-end gap-1">
          {bars.map((style, i) => (
            <Skeleton
              key={i}
              className="flex-1"
              style={style}
            />
          ))}
        </div>
      </div>
      <div className="flex justify-between pl-10">
        {ticks(6, "w-10 h-2.5")}
      </div>
    </div>
  );
}

export function StatCardSkeleton() {
  return (
    <div className="bg-bg-surface border border-border rounded-xl p-5 flex flex-col gap-3">
      <Skeleton className="w-24 h-3" />
      <Skeleton className="w-32 h-8" />
      <Skeleton className="w-20 h-2.5" />
    </div>
  );
}
