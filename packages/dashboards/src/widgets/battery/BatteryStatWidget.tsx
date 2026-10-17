import React from 'react';
import { Icon } from '@iconify/react';
import { useBatteryHealth, useCurrentVehicleStatus } from '@panda/hooks';
import { Card } from '@panda/ui/primitives';
import { cn, formatKwh, formatNumber, formatPercent } from '@panda/ui/lib/utils';
import type { BatteryHealthSummary } from '@panda/types';
import type { VehicleStatus } from '@panda/types';
import { registerWidget } from '../../registry';
import type { WidgetCtx, WidgetInstance } from '../../registry';



interface BatteryStatDefinition {
  id: string;
  title: string;
  icon: string;
  accentBorder?: boolean;
  labelSuffix?: string;

}

function batteryReadout(id: string, health: BatteryHealthSummary, status: VehicleStatus | null | undefined) {
  const fields: Record<string, [number | null | undefined, 'percent' | 'energy' | 'count']> = {
    health_pct: [health.battery_health_pct, 'percent'],
    estimated_degradation_pct: [health.estimated_degradation_pct, 'percent'],
    usable_capacity: [health.usable_now_kwh, 'energy'],
    charge_count: [health.charge_count, 'count'],
    charging_cycles: [health.charging_cycles || health.charge_count, 'count'],
    energy_added: [health.total_energy_added_kwh, 'energy'],
    charge_efficiency: [health.charging_efficiency_pct, 'percent'],
  };
  const result: { value: string; inlineSecondary?: string; secondary?: string } = { value: '—' };
  if (id === 'max_range') {
    const range = status?.range_miles;
    const level = status?.battery_level;
    if (range == null || level == null || level <= 0) return result;
    const projected = range / level * 100;
    result.value = `${formatNumber(projected, 0)} mi`;
    if (health.battery_health_pct != null && health.battery_health_pct > 0) result.inlineSecondary = `/${formatNumber(projected / health.battery_health_pct * 100, 0)} mi`;
    return result;
  }
  const [number, kind] = fields[id]!;
  if (kind === 'count') result.value = formatNumber(number as number, 0);
  else if (number != null) result.value = kind === 'energy' ? formatKwh(number) : formatPercent(number, 1);
  if (id === 'usable_capacity' && health.usable_new_kwh != null) result.inlineSecondary = `/${formatKwh(health.usable_new_kwh)}`;
  if (id === 'charge_count' && health.charging_cycles != null) result.secondary = `${formatNumber(health.charging_cycles, 0)} cycles`;
  return result;
}

const BATTERY_STAT_DEFINITIONS: BatteryStatDefinition[] = [
  {
    id: 'health_pct',
    title: 'Battery Health',
    icon: 'lucide:shield-check',
    accentBorder: true,
  },
  {
    id: 'estimated_degradation_pct',
    title: 'Estimated Degradation',
    icon: 'lucide:trending-down',
  },
  {
    id: 'usable_capacity',
    title: 'Capacity',
    icon: 'lucide:battery',
    labelSuffix: 'now/new',
  },
  {
    id: 'max_range',
    title: 'Max Range',
    icon: 'lucide:route',
    labelSuffix: 'now/new',
  },
  {
    id: 'charge_count',
    title: 'Charges',
    icon: 'lucide:refresh-cw',
  },
  {
    id: 'charging_cycles',
    title: 'Charging Cycles',
    icon: 'lucide:refresh-ccw',
  },
  {
    id: 'energy_added',
    title: 'Energy Added',
    icon: 'lucide:bolt',
  },
  {
    id: 'charge_efficiency',
    title: 'Charge Efficiency',
    icon: 'lucide:zap',
  },
];

const definitionById = new Map(BATTERY_STAT_DEFINITIONS.map((d) => [d.id, d]));

export function BatteryStatWidget({ instance, ctx }: { instance: WidgetInstance; ctx: WidgetCtx }) {
  const definition = definitionById.get(instance.definitionId);
  const { data: health, isLoading: healthLoading } = useBatteryHealth(ctx.vehicleId);
  const { data: status, isLoading: statusLoading } = useCurrentVehicleStatus(ctx.vehicleId);
  const isLoading = healthLoading || (definition?.id === 'max_range' && statusLoading);
  const title = instance.title ?? definition?.title ?? instance.definitionId;
  const accentBorder = definition?.accentBorder ?? false;

  if (!definition) {
    return (
      <div className="flex h-full items-center justify-center rounded-lg border border-dashed border-border text-xs text-fg-tertiary">
        Unknown stat: {instance.definitionId}
      </div>
    );
  }

  const { value, inlineSecondary, secondary } = isLoading
    ? { value: '…', inlineSecondary: undefined, secondary: undefined }
    : health ? batteryReadout(definition.id, health, status) : { value: '—', inlineSecondary: undefined, secondary: undefined };

  return (
    <Card
      padding="none"
      className={cn(
        'relative flex h-full min-h-[72px] flex-col overflow-hidden border p-3',
        accentBorder
          ? 'border-accent/60 shadow-[inset_0_0_0_1px_var(--rm-border-accent)]'
          : 'border-border',
      )}
      data-testid="battery-stat-chip"
    >
      <div className="flex items-start justify-between gap-3">
        <p className="truncate text-xs font-medium uppercase tracking-wider text-fg-tertiary">
          {title}
          {definition.labelSuffix ? (
            <span className="ml-1 text-[10px] font-normal normal-case tracking-normal">
              ({definition.labelSuffix})
            </span>
          ) : null}
        </p>
        <Icon icon={definition.icon} className="h-4 w-4 shrink-0 text-accent" />
      </div>

      <div className="mt-1.5 flex items-baseline gap-1.5">
        <span
          className={cn(
            'font-mono text-2xl font-semibold tabular-nums tracking-tight',
            accentBorder ? 'text-accent' : 'text-fg',
          )}
        >
          {value}
        </span>
        {inlineSecondary ? (
          <span className="font-mono text-sm tabular-nums text-fg-tertiary">
            {inlineSecondary}
          </span>
        ) : null}
      </div>

      {secondary ? (
        <p className="mt-0.5 truncate text-xs text-fg-tertiary">{secondary}</p>
      ) : null}
    </Card>
  );
}

for (const definition of BATTERY_STAT_DEFINITIONS) {
  registerWidget({
    componentType: 'battery',
    definitionId: definition.id,
    title: definition.title,
    defaultSize: { w: 3, h: 2 },
    minSize: { w: 2, h: 2 },
    dataRequirements: () => ({
      batteryHealth: true,
      ...(definition.id === 'max_range' ? { status: true } : {}),
    }),
    component: BatteryStatWidget,
  });
}
