/**
 * NetworkBreakdownWidget — shows a per-network breakdown of charging energy, cost,
 * session count, and free session count.
 *
 * Categories shown:
 *   - "Home Charging"  — sessions where is_home=true
 *   - "Other AC"       — non-home AC sessions with no public network vendor
 *   - Named DC networks (Rivian, Electrify America, EVgo, …)
 */
import React from 'react';
import { Icon } from '@iconify/react';
import { useChargingSummary } from '@panda/hooks';
import { formatKwh, formatCurrency, formatNumber } from '@panda/ui/lib/utils';
import { DASHBOARD_ROW_HEIGHT } from '../../dashboardModel';
import { registerWidget } from '../../registry';
import type { WidgetInstance, WidgetCtx } from '../../registry';

interface NetworkBreakdownEntry {
  network_vendor: string | null;
  session_count: number;
  energy_kwh: number | null;
  cost_usd: number | null;
  free_sessions: number;
}

interface ChargingSummaryWithBreakdown {
  network_breakdown?: NetworkBreakdownEntry[];
}

function NetworkBar({ fraction, className }: { fraction: number; className?: string }) {
  return (
    <div className="h-1 w-full overflow-hidden rounded-full bg-bg-elevated">
      <div
        className={`h-full rounded-full transition-all ${className ?? 'bg-accent'}`}
        style={{ width: `${Math.min(100, Math.max(0, fraction * 100)).toFixed(1)}%` }}
      />
    </div>
  );
}

const MAX_VISIBLE_NETWORK_ROWS = 10;
const MAX_CONTAINER_GRID_ROWS = 20;
const NETWORK_ENTRY_SKELETON_ROWS = 3;
const DASH_WIDGET_HEIGHT_TOLERANCE = 0.999;

const NETWORK_ICON: Record<string, string> = {
  'home charging': 'lucide:home',
  'other ac': 'lucide:plug',
  rivian: 'lucide:zap',
  'electrify america': 'lucide:bolt',
  evgo: 'lucide:plug-zap',
  chargepoint: 'lucide:plug',
  tesla: 'lucide:zap',
};

function networkIcon(vendor: string | null): string {
  if (!vendor) return 'lucide:help-circle';
  return NETWORK_ICON[vendor.toLowerCase()] ?? 'lucide:building';
}

function clampRows(rowCount: number) {
  if (!Number.isFinite(rowCount)) return 1;
  return Math.max(1, Math.min(MAX_CONTAINER_GRID_ROWS, Math.round(rowCount)));
}

function measuredNetworkRows(element: HTMLDivElement | null, count: number) {
  if (count === 0) return clampRows(NETWORK_ENTRY_SKELETON_ROWS);
  const visible = Math.min(count, MAX_VISIBLE_NETWORK_ROWS);
  const children = element ? [...element.children] : [];
  if (!children.length) return clampRows(visible);
  const style = window.getComputedStyle(element ?? document.body);
  const padding = Number.parseFloat(style.paddingTop || '0') + Number.parseFloat(style.paddingBottom || '0');
  const gap = Number.parseFloat(style.rowGap || style.gap || '0');
  const heights = children.slice(0, visible).map((child) => child.getBoundingClientRect().height);
  const content = heights.reduce((total, height) => total + height, 0);
  const pixels = Math.ceil(padding + content + Math.max(0, visible - 1) * gap);
  return clampRows(Math.max(1, Math.ceil(pixels / DASHBOARD_ROW_HEIGHT - DASH_WIDGET_HEIGHT_TOLERANCE)));
}

function networkRows(entries: NetworkBreakdownEntry[]) {
  const energy = entries.reduce((total, row) => total + (row.energy_kwh ?? 0), 0);
  return entries.map((row) => ({ row, fraction: energy > 0 ? (row.energy_kwh ?? 0) / energy : 0, name: row.network_vendor ?? 'Unknown' }));
}

function NetworkBreakdownWidget({ instance, ctx }: { instance: WidgetInstance; ctx: WidgetCtx }) {
  const { data, isLoading } = useChargingSummary(ctx.vehicleId, ctx.from, ctx.to);
  const summary = data as ChargingSummaryWithBreakdown | undefined;
  const breakdown: NetworkBreakdownEntry[] = summary?.network_breakdown ?? [];
  const rowsRef = React.useRef<HTMLDivElement>(null);
  const contentSignature = React.useMemo(
    () => breakdown.map((row) => `${row.network_vendor ?? ''}:${row.energy_kwh ?? 'x'}:${row.session_count}`).join('|'),
    [breakdown],
  );

  React.useEffect(() => {
    const updateLayout = ctx.updateWidgetLayout;
    if (updateLayout) updateLayout(instance.id, measuredNetworkRows(rowsRef.current, isLoading ? 0 : breakdown.length));
  }, [ctx, instance.id, isLoading, contentSignature]);

  const model = networkRows(breakdown);

  if (isLoading) {
    return (
      <div className="flex h-full flex-col gap-2 p-1">
        {Array.from({ length: 3 }).map((_, i) => (
          <div key={i} className="h-14 animate-pulse rounded-xl border border-border bg-bg-elevated/60" />
        ))}
      </div>
    );
  }

  if (breakdown.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-fg-tertiary">
        <div className="text-center">
          <Icon icon="lucide:plug-zap" className="mx-auto mb-2 h-8 w-8 text-fg-tertiary/40" />
          <p>No charging sessions yet</p>
          <p className="mt-1 text-xs">Breakdown appears once sessions are recorded</p>
        </div>
      </div>
    );
  }

  return (
    <div ref={rowsRef} className="flex h-full flex-col gap-2 overflow-auto p-1">
      {model.map(({ row, fraction, name }) => {
        return (
          <div
            key={name}
            className="rounded-xl border border-border bg-bg-elevated/60 px-3 py-2.5"
          >
            <div className="mb-1.5 flex items-center justify-between gap-2">
              <div className="flex items-center gap-2 min-w-0">
                <Icon icon={networkIcon(row.network_vendor)} className="h-3.5 w-3.5 shrink-0 text-accent" />
                <span className="truncate text-sm font-medium text-fg">{name}</span>
                {row.free_sessions > 0 && (
                  <span className="shrink-0 rounded-full bg-status-positive/15 px-1.5 py-0.5 text-xs font-medium text-status-positive">
                    {row.free_sessions} free
                  </span>
                )}
              </div>
              <span className="font-mono text-sm text-fg-secondary">
                {row.energy_kwh != null ? formatKwh(row.energy_kwh) : '—'}
              </span>
            </div>
            <NetworkBar fraction={fraction} />
            <div className="mt-1.5 flex items-center gap-3 text-xs text-fg-tertiary">
              <span>{formatNumber(row.session_count, 0)} sessions</span>
              {row.cost_usd != null && row.cost_usd > 0 && (
                <>
                  <span>·</span>
                  <span className="font-mono">{formatCurrency(row.cost_usd)}</span>
                </>
              )}
              <span className="ml-auto">
                {(fraction * 100).toFixed(0)}%
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}

registerWidget({
  componentType: 'custom',
  definitionId: 'charging.network_breakdown',
  title: 'Network Breakdown',
  defaultSize: { w: 4, h: 2 },
  minSize: { w: 3, h: 1 },
  defaultOptions: {},
  dataRequirements: () => ({ chargingSummary: true }),
  component: NetworkBreakdownWidget,
});
