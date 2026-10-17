import { liveChargingModel } from './liveChargingModel';
import { chargeGauge } from './vehiclePresentation';
import { formatWidgetChargeTime } from '../widgetLifecycle';
import React from 'react';
import { Zap, Clock, Battery, Plug, RouteIcon, TrendingUp } from 'lucide-react';
import { useCurrentVehicleStatus, useLiveSession } from '@panda/hooks';
import { isVehicleCharging } from '@panda/types';
import { registerWidget } from '../../registry';
import type { WidgetCtx, WidgetInstance } from '../../registry';

// ── Helpers ───────────────────────────────────────────────────────────────────

function formatTimeRemaining(mins: number | null): string {
  return formatWidgetChargeTime(mins, '—');
}

function formatKw(kw: number | null): string {
  if (kw === null) return '—';
  return `${kw.toFixed(1)} kW`;
}

function formatKwh(kwh: number | null): string {
  if (kwh === null) return '—';
  return `${kwh.toFixed(2)} kWh`;
}

function formatSoc(soc: number | null): string {
  if (soc === null) return '—';
  return `${Math.round(soc)}%`;
}

// ── Live stat row ─────────────────────────────────────────────────────────────

function LiveStat({ label, value, icon, accent = false }: {
  label: string;
  value: string;
  icon: React.ReactNode;
  accent?: boolean;
}) {
  return (
    <div className="flex flex-col items-center gap-1 rounded-xl border border-border bg-bg-elevated px-4 py-3 text-center">
      <span className={`${accent ? 'text-accent' : 'text-fg-tertiary'}`}>{icon}</span>
      <span className={`text-lg font-mono font-semibold tabular-nums ${accent ? 'text-accent' : 'text-fg'}`}>
        {value}
      </span>
      <span className="text-xs text-fg-tertiary">{label}</span>
    </div>
  );
}

// ── Not charging placeholder ──────────────────────────────────────────────────

function NotChargingPlaceholder({ soc, range }: { soc: number | null; range: number | null }) {
  return (
    <div className="flex h-full items-center gap-6 px-4">
      <div className="flex items-center gap-2 text-fg-tertiary">
        <Plug className="h-5 w-5" />
        <span className="text-sm">Not plugged in</span>
      </div>
      {soc !== null && (
        <div className="flex items-center gap-1.5 text-sm">
          <Battery className="h-4 w-4 text-fg-tertiary" />
          <span className="font-mono font-medium text-fg">{Math.round(soc)}%</span>
          <span className="text-fg-tertiary">SoC</span>
        </div>
      )}
      {range !== null && (
        <div className="flex items-center gap-1.5 text-sm">
          <RouteIcon className="h-4 w-4 text-fg-tertiary" />
          <span className="font-mono font-medium text-fg">{Math.round(range)} mi</span>
          <span className="text-fg-tertiary">range</span>
        </div>
      )}
    </div>
  );
}

// ── Main widget ───────────────────────────────────────────────────────────────

function LiveChargingWidget({
  instance: _instance,
  ctx,
}: {
  instance: WidgetInstance;
  ctx: WidgetCtx;
}) {
  const vehicleId = ctx.vehicleId ?? null;
  const { data: status } = useCurrentVehicleStatus(vehicleId);

  const isCharging = isVehicleCharging(status);

  const { data: session, isLoading: isLiveSessionLoading } = useLiveSession(vehicleId, isCharging);

  const readout = liveChargingModel(session, status);
  const headerText = isCharging ? 'Live Charging' : 'Charging Status';

  return (
    <div className="flex h-full flex-col gap-3 p-4">
      {/* Header */}
      <div className="flex items-center gap-2">
        <Zap className={`h-4 w-4 ${isCharging ? 'text-accent animate-pulse' : 'text-fg-tertiary'}`} />
        <span className="text-sm font-medium text-fg">{headerText}</span>
        {isCharging && session?.charger_type && (
          <span className="rounded-full bg-bg-elevated px-2 py-0.5 text-xs text-fg-tertiary">
            {session.charger_type.toUpperCase()}
          </span>
        )}
      </div>

      {isCharging && !session && (
        <div className="text-xs text-fg-tertiary" role="status" aria-live="polite">
          {isLiveSessionLoading
            ? 'Waiting for live charging data…'
            : 'Live charging data unavailable; showing vehicle status where available.'}
        </div>
      )}

      {/* Content */}
      {isCharging ? (
        <div className="grid flex-1 grid-cols-4 gap-2">
          <LiveStat
            label="Power"
            value={formatKw(readout.power)}
            icon={<Zap className="h-4 w-4" />}
            accent
          />
          <LiveStat
            label="Battery"
            value={formatSoc(readout.soc)}
            icon={<Battery className="h-4 w-4" />}
          />
          <LiveStat
            label="Added"
            value={formatKwh(readout.energy)}
            icon={<TrendingUp className="h-4 w-4" />}
          />
          <LiveStat
            label="Time Left"
            value={formatTimeRemaining(readout.time)}
            icon={<Clock className="h-4 w-4" />}
          />
        </div>
      ) : (
        <NotChargingPlaceholder soc={readout.restingSoc} range={readout.range} />
      )}

      {/* SoC progress bar when charging */}
      {isCharging && (
        <SocBar soc={readout.soc} limit={readout.limit} />
      )}
    </div>
  );
}

function SocBar({ soc, limit }: { soc: number | null; limit: number | null }) {
  const { level: pct, limit: limitPct } = chargeGauge(soc, limit);

  return (
    <div className="relative h-2 overflow-hidden rounded-full bg-bg-elevated">
      {/* Filled */}
      <div
        className="absolute inset-y-0 left-0 rounded-full bg-accent transition-all duration-700"
        style={{ width: `${pct}%` }}
      />
      {/* Limit marker */}
      {limitPct !== null && (
        <div
          className="absolute inset-y-0 w-0.5 bg-fg-tertiary/50"
          style={{ left: `${limitPct}%` }}
        />
      )}
    </div>
  );
}

// ── Registry ──────────────────────────────────────────────────────────────────

registerWidget({
  componentType: 'custom',
  definitionId: 'overview.live.charging',
  title: 'Live Charging',
  defaultSize: { w: 8, h: 4 },
  minSize: { w: 6, h: 3 },
  defaultOptions: {},
  dataRequirements: () => ({ status: true }),
  component: LiveChargingWidget,
});
