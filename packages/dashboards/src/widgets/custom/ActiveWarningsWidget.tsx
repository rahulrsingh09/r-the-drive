import React from 'react';
import { ShieldAlert, Droplets, Wrench, Flame, ZapOff, Bell, CheckCircle2 } from 'lucide-react';
import { useCurrentVehicleStatus } from '@panda/hooks';
import type { VehicleStatus } from '@panda/types';
import { registerWidget } from '../../registry';
import type { WidgetCtx, WidgetInstance } from '../../registry';

// ── Warning definitions ───────────────────────────────────────────────────────

interface Warning {
  key: string;
  label: string;
  icon: React.ReactNode;
  severity: 'error' | 'warn' | 'info';
  field: keyof VehicleStatus;
  inactive?: unknown[];
}

const WARNINGS: Warning[] = [
  {
    key: 'brake_fluid',
    label: 'Brake Fluid Low',
    icon: <Droplets className="h-3.5 w-3.5" />,
    severity: 'error',
    field: 'brake_fluid_low',
  },
  {
    key: 'wiper_fluid',
    label: 'Wiper Fluid Low',
    icon: <Droplets className="h-3.5 w-3.5" />,
    severity: 'warn',
    field: 'wiper_fluid_low',
  },
  {
    key: 'alarm',
    label: 'Alarm Active',
    icon: <Bell className="h-3.5 w-3.5" />,
    severity: 'error',
    field: 'alarm_active',
  },
  {
    key: 'charger_derate',
    label: 'Charger Derate',
    icon: <ZapOff className="h-3.5 w-3.5" />,
    severity: 'warn',
    field: 'charger_derate_active',
  },
  {
    key: 'hv_thermal',
    label: 'HV Thermal Event',
    icon: <Flame className="h-3.5 w-3.5" />,
    severity: 'error',
    field: 'hv_thermal_event', inactive: [null, undefined, '', false, 0, NaN, 'none', 'idle'],
  },
  {
    key: 'service_mode',
    label: 'Service Mode',
    icon: <Wrench className="h-3.5 w-3.5" />,
    severity: 'info',
    field: 'service_mode',
  },
  {
    key: 'tire_pressure',
    label: 'Tire Pressure',
    icon: <ShieldAlert className="h-3.5 w-3.5" />,
    severity: 'warn',
    field: 'tire_pressure_status', inactive: [null, undefined, 'ok', 'none'],
  },
];

const SEVERITY_COLORS: Record<Warning['severity'], string> = {
  error: 'bg-status-danger/10 text-status-danger border-status-danger/20',
  warn:  'bg-status-warning/10 text-status-warning border-status-warning/20',
  info:  'bg-status-info/10 text-status-info border-status-info/20',
};

// ── Component ─────────────────────────────────────────────────────────────────

function ActiveWarningsWidget({
  instance: _instance,
  ctx,
}: {
  instance: WidgetInstance;
  ctx: WidgetCtx;
}) {
  const vehicleId = ctx.vehicleId ?? null;
  const { data: status } = useCurrentVehicleStatus(vehicleId);

  const activeWarnings = status
    ? WARNINGS.filter(({ field, inactive }) => inactive ? !inactive.includes(status[field]) : status[field] === true)
    : [];

  return (
    <div className="flex h-full flex-col gap-3 p-4">
      {/* Header */}
      <div className="flex items-center gap-2">
        <ShieldAlert className={`h-4 w-4 ${activeWarnings.length > 0 ? 'text-status-danger' : 'text-fg-tertiary'}`} />
        <span className="text-sm font-medium text-fg">Warnings</span>
        {activeWarnings.length > 0 && (
          <span className="rounded-full bg-status-danger/15 px-2 py-0.5 text-xs font-medium text-status-danger">
            {activeWarnings.length}
          </span>
        )}
      </div>

      {/* Content */}
      {activeWarnings.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 text-center">
          <CheckCircle2 className="h-7 w-7 text-status-positive" />
          <p className="text-sm font-medium text-status-positive">All clear</p>
          <p className="text-xs text-fg-tertiary">No active warnings</p>
        </div>
      ) : (
        <div className="flex flex-col gap-1.5">
          {activeWarnings.map((w) => (
            <div
              key={w.key}
              className={`flex items-center gap-2.5 rounded-xl border px-3 py-2 text-sm ${SEVERITY_COLORS[w.severity]}`}
            >
              <span className="shrink-0">{w.icon}</span>
              <span className="font-medium">{w.label}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Registry ──────────────────────────────────────────────────────────────────

registerWidget({
  componentType: 'custom',
  definitionId: 'overview.warnings',
  title: 'Active Warnings',
  defaultSize: { w: 4, h: 4 },
  minSize: { w: 3, h: 3 },
  defaultOptions: {},
  dataRequirements: () => ({ status: true }),
  component: ActiveWarningsWidget,
});
