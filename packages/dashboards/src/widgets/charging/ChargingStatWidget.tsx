import React from 'react';
import { Icon } from '@iconify/react';
import { useChargingSummary } from '@panda/hooks';
import { Card } from '@panda/ui/primitives';
import { cn, formatCurrency, formatKwh, formatNumber, formatPercent } from '@panda/ui/lib/utils';
import { registerWidget } from '../../registry';
import type { WidgetCtx, WidgetInstance } from '../../registry';
import { useDashboardDataAvailable, useDashboardDataSelector } from '../../dashboardData';

interface ChargingSummarySnapshot {
  session_count: number;
  total_energy_kwh: number | null;
  total_cost_usd: number | null;
  home_kwh: number | null;
  away_kwh: number | null;
  unknown_location_kwh: number | null;
  ac_kwh: number | null;
  dc_kwh: number | null;
  charging_cycles: number | null;
  charging_efficiency_pct: number | null;
  max_charge_rate_kw: number | null;
  max_charge_limit_pct: number | null;
}

interface ChargingStatDefinition {
  id: string;
  title: string;
  icon: string;
  accentBorder?: boolean;

}

function chargingReadout(id: string, summary: ChargingSummarySnapshot) {
  const fields: Record<string, [number | null, 'count' | 'energy' | 'cost' | 'percent' | 'power', number]> = {
    sessions: [summary.session_count, 'count', 0], total_energy: [summary.total_energy_kwh, 'energy', 0],
    total_cost: [summary.total_cost_usd, 'cost', 0], charging_cycles: [summary.charging_cycles, 'count', 0],
    charge_efficiency: [summary.charging_efficiency_pct, 'percent', 1], max_charge_rate: [summary.max_charge_rate_kw, 'power', 1],
    max_charge_limit: [summary.max_charge_limit_pct, 'percent', 0],
  };
  const energy = (value: number | null) => value == null ? '—' : formatKwh(value);
  const totalLocation = (summary.home_kwh ?? 0) + (summary.away_kwh ?? 0) + (summary.unknown_location_kwh ?? 0);
  fields.avg_session = [summary.session_count && summary.total_energy_kwh != null ? summary.total_energy_kwh / summary.session_count : null, 'energy', 0];
  fields.home_share = [totalLocation ? (summary.home_kwh ?? 0) / totalLocation * 100 : null, 'percent', 0];
  fields.dc_share = [summary.total_energy_kwh ? (summary.dc_kwh ?? 0) / summary.total_energy_kwh * 100 : null, 'percent', 0];
  const [number, kind, digits] = fields[id]!;
  const formatters = { count: formatNumber, energy: formatKwh, cost: formatCurrency, percent: formatPercent, power: (value: number) => `${formatNumber(value, 1)} kW` };
  return {
    value: number == null && id !== 'sessions' ? '—' : formatters[kind](number as number, digits),
    secondary: id === 'home_share' ? `Home ${energy(summary.home_kwh)} / Away ${energy((summary.away_kwh ?? 0) + (summary.unknown_location_kwh ?? 0))}` : id === 'dc_share' ? `AC ${energy(summary.ac_kwh)} / DC ${energy(summary.dc_kwh)}` : undefined,
  };
}

const CHARGING_STAT_DEFINITIONS: ChargingStatDefinition[] = [
  {
    id: 'sessions',
    title: 'Sessions',
    icon: 'lucide:calendar-days',
  },
  {
    id: 'total_energy',
    title: 'Total Energy',
    icon: 'lucide:bolt',
    accentBorder: true,
  },
  {
    id: 'total_cost',
    title: 'Total Cost',
    icon: 'lucide:dollar-sign',
  },
  {
    id: 'avg_session',
    title: 'Avg / Session',
    icon: 'lucide:zap',
  },
  {
    id: 'charging_cycles',
    title: 'Charging Cycles',
    icon: 'lucide:refresh-ccw',
  },
  {
    id: 'charge_efficiency',
    title: 'Charge Efficiency',
    icon: 'lucide:activity',
  },
  {
    id: 'max_charge_rate',
    title: 'Max Charge Rate',
    icon: 'lucide:gauge',
  },
  {
    id: 'max_charge_limit',
    title: 'Max Charge Limit',
    icon: 'lucide:battery',
  },
  {
    id: 'home_share',
    title: 'Home Charging',
    icon: 'lucide:home',
  },
  {
    id: 'dc_share',
    title: 'DC Fast Charging',
    icon: 'lucide:plug-zap',
  },
];

const definitionById = new Map(CHARGING_STAT_DEFINITIONS.map((d) => [d.id, d]));

export function ChargingStatWidget({ instance, ctx }: { instance: WidgetInstance; ctx: WidgetCtx }) {
  const definition = definitionById.get(instance.definitionId);
  const dashboardDataAvailable = useDashboardDataAvailable();
  const dashboardSummary = useDashboardDataSelector((snapshot) => snapshot.chargingSummary);
  const dashboardRefreshing = useDashboardDataSelector((snapshot) => snapshot.isRefreshing) ?? false;
  const { data: fetchedSummary, isLoading: fetchedLoading } = useChargingSummary(
    dashboardDataAvailable ? null : ctx.vehicleId,
    ctx.from,
    ctx.to,
  );
  const summary = dashboardDataAvailable ? dashboardSummary : fetchedSummary;
  const isLoading = dashboardDataAvailable ? dashboardRefreshing : fetchedLoading;
  const title = instance.title ?? definition?.title ?? instance.definitionId;
  const accentBorder = definition?.accentBorder ?? false;

  if (!definition) {
    return (
      <div className="flex h-full items-center justify-center rounded-lg border border-dashed border-border text-xs text-fg-tertiary">
        Unknown stat: {instance.definitionId}
      </div>
    );
  }

  const { value, secondary } = isLoading ? { value: '…', secondary: undefined }
    : summary ? chargingReadout(definition.id, summary as ChargingSummarySnapshot) : { value: '—', secondary: undefined };

  return (
    <Card
      padding="none"
      className={cn(
        'relative flex h-full min-h-[72px] flex-col overflow-hidden border p-3',
        accentBorder
          ? 'border-accent/60 shadow-[inset_0_0_0_1px_var(--rm-border-accent)]'
          : 'border-border',
      )}
      data-testid="charging-stat-chip"
    >
      <div className="flex items-start justify-between gap-3">
        <p className="truncate text-xs font-medium uppercase tracking-wider text-fg-tertiary">
          {title}
        </p>
        <Icon icon={definition.icon} className="h-4 w-4 shrink-0 text-accent" />
      </div>

      <div className="mt-1.5 flex items-baseline gap-1">
        <span
          className={cn(
            'font-mono text-2xl font-semibold tabular-nums tracking-tight',
            accentBorder ? 'text-accent' : 'text-fg',
          )}
        >
          {value}
        </span>
      </div>

      {secondary ? (
        <p className="mt-0.5 truncate text-xs text-fg-tertiary">{secondary}</p>
      ) : null}
    </Card>
  );
}

for (const definition of CHARGING_STAT_DEFINITIONS) {
  registerWidget({
    componentType: 'charging',
    definitionId: definition.id,
    title: definition.title,
    defaultSize: { w: 3, h: 2 },
    minSize: { w: 2, h: 2 },
    dataRequirements: () => ({ chargingSummary: true }),
    component: ChargingStatWidget,
  });
}
