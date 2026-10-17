import React from 'react';
import { useParkedEnergy } from '@panda/hooks';
import type { ParkedEnergySample, ParkedEnergyWindow } from '@panda/types';
import { Badge, Card, CardContent, CardHeader, CardTitle, Skeleton } from '@panda/ui/primitives';
import { formatAppDateTime } from '@panda/ui/lib/dateTime';

const WINDOWS: Array<{ value: ParkedEnergyWindow; label: string }> = [
  { value: 'since_parked', label: 'Since parked' },
  { value: '8h', label: '8 hours' },
  { value: '24h', label: '24 hours' },
];

const CATEGORIES = [
  { key: 'vehicle_systems_kwh', label: 'Vehicle systems', color: 'bg-accent' },
  { key: 'climate_kwh', label: 'Climate', color: 'bg-status-warning' },
  { key: 'gear_guard_kwh', label: 'Gear Guard', color: 'bg-status-positive' },
  { key: 'outlets_kwh', label: 'Outlets', color: 'bg-fg-tertiary' },
] as const;

export function ParkedEnergyPanel({
  vehicleId,
  from,
  to,
}: {
  vehicleId: string | null;
  from: string | null;
  to: string | null;
}) {
  const { data, isLoading } = useParkedEnergy(vehicleId, from, to);
  const samples = data?.samples ?? [];
  const byWindow = new Map<ParkedEnergyWindow, ParkedEnergySample>();
  for (const candidate of samples) {
    if (!byWindow.has(candidate.window)) byWindow.set(candidate.window, candidate);
  }
  const preferred = chooseDefaultWindow(samples);
  const [selection, setWindow] = React.useState<ParkedEnergyWindow>(preferred);
  const window = byWindow.has(selection) ? selection : preferred;
  if (window !== selection) setWindow(window);
  const sample = byWindow.get(window) ?? null;
  const categories = CATEGORIES.map(category => ({
    ...category,
    raw: sample?.[category.key] ?? null,
    value: Math.max(0, sample?.[category.key] ?? 0),
  }));
  const categoryTotal = categories.reduce((sum, category) => sum + category.value, 0);

  return (
    <Card data-testid="parked-energy-panel">
      <CardHeader className="flex-col items-stretch gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <CardTitle>Parked Energy</CardTitle>
            <Badge variant="accent">Rivian reported</Badge>
          </div>
          <p className="mt-1 text-xs text-fg-tertiary">
            Vehicle-reported energy use by system. This is separate from Panda&apos;s battery-change estimate below.
          </p>
        </div>
        <div role="group" aria-label="Parked energy window" className="grid w-full grid-cols-[1.25fr_1fr_1fr] gap-1 rounded-lg border border-border bg-bg-elevated p-1 lg:w-auto lg:shrink-0">
          {WINDOWS.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => setWindow(option.value)}
              disabled={!byWindow.has(option.value)}
              aria-pressed={window === option.value}
              className={`min-h-11 whitespace-nowrap rounded-md px-2.5 py-2 text-xs font-medium transition-colors disabled:opacity-35 ${
                window === option.value
                  ? 'bg-accent text-fg-on-accent'
                  : 'text-fg-secondary hover:bg-bg-surface hover:text-fg'
              }`}
            >
              {option.label}
            </button>
          ))}
        </div>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="grid gap-3 sm:grid-cols-3">
            <Skeleton className="h-16" />
            <Skeleton className="h-16" />
            <Skeleton className="h-16" />
          </div>
        ) : !sample ? (
          <div className="rounded-xl border border-dashed border-border p-5 text-sm text-fg-secondary">
            Start the optional Parallax collector to populate Rivian&apos;s parked-energy breakdown.
          </div>
        ) : (
          <div className="grid gap-5">
            <div className="grid grid-cols-3 gap-2 sm:gap-3" data-testid="parked-energy-metrics">
              <Metric label="Energy used" value={formatEnergy(sample.total_kwh)} />
              <Metric label="Duration" value={formatDuration(sample.duration_minutes)} />
              <Metric label="Range impact" value={formatRange(sample.total_range_impact_km)} />
            </div>

            <div>
              <div className="flex h-4 overflow-hidden rounded-full bg-bg-elevated" aria-label="Parked energy category breakdown">
                {categories.map((category) => {
                  const width = categoryTotal > 0 ? (category.value / categoryTotal) * 100 : 0;
                  return (
                    <div
                      key={category.key}
                      className={category.color}
                      style={{ width: `${width}%` }}
                      title={`${category.label}: ${formatEnergy(category.value)}`}
                    />
                  );
                })}
              </div>
              <div className="mt-3 grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
                {categories.map((category) => (
                  <div key={category.key} className="flex items-center justify-between gap-3 text-xs">
                    <span className="flex items-center gap-2 text-fg-secondary">
                      <span className={`h-2.5 w-2.5 rounded-full ${category.color}`} />
                      {category.label}
                    </span>
                    <span className="font-medium text-fg">{formatEnergy(category.raw)}</span>
                  </div>
                ))}
              </div>
            </div>

            <p className="text-xs text-fg-tertiary">
              Updated {formatAppDateTime(sample.source_at)}
              {sample.parked_started_at ? ` · Parked since ${formatAppDateTime(sample.parked_started_at)}` : ''}
            </p>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function chooseDefaultWindow(samples: ParkedEnergySample[]): ParkedEnergyWindow {
  let recentParked = false;
  let hasDay = false;
  let inspectedParked = false;
  const cutoff = Date.now() - 6 * 60 * 60 * 1000;
  for (const sample of samples) {
    hasDay ||= sample.window === '24h';
    if (sample.window !== 'since_parked' || inspectedParked) continue;
    inspectedParked = true;
    recentParked = new Date(sample.received_at).getTime() >= cutoff;
  }
  const preferred: ParkedEnergyWindow[] = [];
  if (recentParked) preferred.push('since_parked');
  if (hasDay) preferred.push('24h');
  return preferred[0] ?? samples[0]?.window ?? 'since_parked';
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl border border-border bg-bg-elevated/40 p-3">
      <p className="text-xs text-fg-tertiary">{label}</p>
      <p className="mt-1 break-words text-base sm:text-xl font-semibold text-fg">{value}</p>
    </div>
  );
}

function formatEnergy(value: number | null) {
  return value == null ? '—' : `${value.toFixed(2)} kWh`;
}

function formatRange(value: number | null) {
  return value == null ? '—' : `${(value * 0.621371).toFixed(1)} mi`;
}

function formatDuration(value: number | null) {
  if (value == null) return '—';
  if (value < 60) return `${value} min`;
  return `${(value / 60).toFixed(value % 60 === 0 ? 0 : 1)} h`;
}
