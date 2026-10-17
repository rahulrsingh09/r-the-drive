import { formatWidgetChargeTime } from '../widgetLifecycle';
import React from 'react';
import { Zap } from 'lucide-react';
import { AuthenticatedVehicleArtwork, useChargingSummary } from '@panda/hooks';
import { formatKwh, formatNumber, formatPercent as formatDashboardPercent } from '@panda/ui/lib/utils';
import { isVehicleCharging } from '@panda/types';
import type { VehicleStatus } from '@panda/types';
import { registerWidget } from '../../registry';
import type { WidgetCtx, WidgetInstance } from '../../registry';
import { isVehiclePluggedIn } from '../../dashboardVisibility';
import { chargingArtwork, chargeGauge } from './vehiclePresentation';
import { useWidgetVehicle } from './useWidgetVehicle';

type ChargingCropFamily = 'R1T' | 'R1S' | 'default';

interface ChargingCropConfig {
  translateX: number;
  scale: number;
  translateY?: number;
  objectPosition?: string;
}

// The image element spans the whole card, while object-contain places the
// artwork on its right. Anchor at the visible vehicle's lower-left position in
// that full element so the scale cannot turn the 20px nudge into a large shift.
const DEMO_CHARGING_FALLBACK_TRANSFORM = 'translate(20px, 5px) scale(2.25)';
const DEMO_CHARGING_FALLBACK_ORIGIN = '33% 95%';

const CHARGING_CROP_CONFIG: Record<ChargingCropFamily, ChargingCropConfig> = {
  R1T: {
    translateX: -34,
    translateY: 2,
    scale: 1.92,
    objectPosition: 'left center',
  },
  R1S: {
    // Preserve the original Rivian-provided charging-art composition.
    translateX: -12,
    scale: 1.12,
    objectPosition: 'left center',
  },
  default: {
    translateX: -12,
    scale: 1.12,
    objectPosition: 'left center',
  },
};

interface ChargingSummarySnapshot {
  session_count: number;
  total_energy_kwh: number | null;
  total_cost_usd: number | null;
  home_kwh: number | null;
  away_kwh: number | null;
  ac_kwh: number | null;
  dc_kwh: number | null;
  charging_cycles: number | null;
  charging_efficiency_pct: number | null;
  max_charge_rate_kw: number | null;
  max_charge_limit_pct: number | null;
}

function ChargingConnectionWidget({
  instance: _instance,
  ctx,
}: {
  instance: WidgetInstance;
  ctx: WidgetCtx;
}) {
  const { status, images, vehicleModel, isDemoVehicle } = useWidgetVehicle(ctx.vehicleId);
  const { data: summary } = useChargingSummary(ctx.vehicleId, ctx.from, ctx.to);
  const pluggedIn = ctx.visibilityState
    ? ctx.visibilityState['vehicle-connection'] === 'plugged'
    : isVehiclePluggedIn(status);
  const charging = isActivelyCharging(status);
  const timeToFull = status?.time_to_end_of_charge_min;
  const snapshot = summary as ChargingSummarySnapshot | undefined;
  const cropFamily = chargingCropFamily(vehicleModel);
  const { fallback: fallbackChargingSource, light: chargingSideLight, dark: chargingSideDark } = chargingArtwork(images, vehicleModel, isDemoVehicle);
  const imageMode = 'side-charging';
  const displaySideLight = chargingSideLight ?? fallbackChargingSource;
  const displaySideDark = chargingSideDark ?? fallbackChargingSource;
  const rows = [
    {
      label: 'Status',
      value: charging ? 'Charging' : 'Standby',
      accent: charging,
    },
    {
      label: 'Charge Efficiency',
      value: snapshot ? formatMaybePercent(snapshot.charging_efficiency_pct, 1) : '-',
    },
    {
      label: 'Avg / Session',
      value:
        snapshot && snapshot.session_count > 0 && snapshot.total_energy_kwh != null
          ? formatKwh(snapshot.total_energy_kwh / snapshot.session_count)
          : '-',
    },
    {
      label: 'Max Charge Rate',
      value: snapshot?.max_charge_rate_kw == null ? '-' : `${formatNumber(snapshot.max_charge_rate_kw, 1)} kW`,
    },
    {
      label: 'Max Charge Limit',
      value: formatMaybePercent(status?.battery_limit ?? snapshot?.max_charge_limit_pct ?? null, 0),
    },
    {
      label: 'Full In',
      value: formatTimeToFull(timeToFull),
    },
  ];

  if (!pluggedIn) return null;

  return (
    <section
      data-testid="charging-connection-chip"
      data-charging-state={charging ? 'charging' : 'connected'}
      data-crop-family={cropFamily}
      data-image-mode={imageMode}
      data-image-light={displaySideLight}
      data-image-dark={displaySideDark}
      data-fallback-image={fallbackChargingSource ?? undefined}
      data-artwork-variant={isDemoVehicle ? 'demo-charging-fallback' : 'rivian-charging'}
      className="relative h-full min-h-0 overflow-hidden rounded-2xl border border-border bg-[linear-gradient(135deg,var(--rm-bg-surface),var(--rm-bg-elevated))] shadow-lg"
    >
      <div className="absolute inset-0 flex items-stretch justify-end">
        {displaySideLight ? <VehicleSideImage source={chargingSideLight} fallbackSource={fallbackChargingSource} darkClassName="dark:hidden" cropConfig={CHARGING_CROP_CONFIG[cropFamily]} /> : null}
        {displaySideDark ? <VehicleSideImage source={chargingSideDark} fallbackSource={fallbackChargingSource} darkClassName="hidden dark:block" cropConfig={CHARGING_CROP_CONFIG[cropFamily]} /> : null}
      </div>

      <div className="pointer-events-none absolute inset-y-0 left-0 w-[62%] bg-gradient-to-r from-bg via-bg/88 to-transparent" />

      <div className="absolute left-3 top-4 z-10 grid w-[40%] min-w-[180px] max-w-[260px] gap-2 pb-[12%]">
        {rows.map((row) => (
          <ChargingSummaryRow key={row.label} label={row.label} value={row.value} accent={row.accent === true} />
        ))}
      </div>

      <ChargingBatteryLedBar level={status?.battery_level} charging={charging} />
    </section>
  );
}

function ChargingSummaryRow({
  label,
  value,
  accent = false,
}: {
  label: string;
  value: React.ReactNode;
  accent?: boolean;
}) {
  return (
    <div
      className="flex min-w-0 items-center justify-between gap-3 rounded-xl border border-border bg-bg-elevated/90 px-3 py-2 text-xs backdrop-blur-sm"
    >
      <span className={`inline-flex min-w-0 items-center gap-2 truncate text-fg-tertiary`}>
        {accent ? <Zap className="h-3.5 w-3.5 shrink-0" /> : null}
        {label}
      </span>
      <span className={`shrink-0 font-mono font-medium tabular-nums ${accent ? 'text-accent' : 'text-fg'}`}>{value}</span>
    </div>
  );
}

function ChargingBatteryLedBar({
  level,
  charging,
}: {
  level: number | null | undefined;
  charging: boolean;
}) {
  const { level: normalizedLevel, segments } = chargeGauge(level);
  const segmentCount = 20;
  const segmentGapPx = 2;
  const segmentWidth = `calc((100% - ${segmentGapPx * (segmentCount - 1)}px) / ${segmentCount})`;
  const segmentGridStyle = {
    display: 'grid',
    gridTemplateColumns: `repeat(${segmentCount}, ${segmentWidth})`,
    gap: segmentGapPx,
    height: '100%',
  } as React.CSSProperties;

  return (
    <div
      data-testid="charging-battery-led-bar"
      aria-label={`Battery ${Math.round(normalizedLevel)} percent`}
      style={{
        position: 'absolute',
        left: 0,
        right: 0,
        bottom: 0,
        height: 14,
        zIndex: 100,
        pointerEvents: 'none',
      }}
    >
      <div style={{ position: 'relative', height: '100%', overflow: 'visible' }}>
        <div
          data-testid="charging-battery-led-segments"
          style={segmentGridStyle}
        >
          {segments.map(({ index, filled }) => {
            return (
              <span
                key={index}
                data-testid="charging-battery-led-segment"
                data-filled={filled ? 'true' : 'false'}
                style={{
                  display: 'block',
                  height: '100%',
                  borderRadius: 4,
                  backgroundColor: filled ? 'var(--rm-status-positive)' : 'var(--rm-text-tertiary)',
                  boxShadow: filled ? '0 0 10px color-mix(in oklab, var(--rm-status-positive) 72%, transparent)' : 'none',
                }}
              />
            );
          })}
        </div>
        {charging ? (
          <div
            data-testid="charging-battery-led-sweep"
            className="charging-led-sweep"
            style={{
              position: 'absolute',
              top: 0,
              bottom: 0,
              left: 0,
              display: 'block',
              width: segmentWidth,
            }}
          >
            <span
              style={{
                display: 'block',
                height: '100%',
                width: '100%',
                borderRadius: 4,
                backgroundColor: 'color-mix(in oklab, var(--rm-charging-done) 100%, white 15%)',
                boxShadow: '0 0 8px 2px color-mix(in oklab, var(--rm-charging-done) 100%, transparent), 0 0 24px 4px color-mix(in oklab, var(--rm-charging-done) 70%, transparent)',
                filter: 'brightness(1.35)',
              }}
            />
          </div>
        ) : null}
      </div>
      <style>{`
        @keyframes charging-led-sweep {
          0% { left: 0; }
          100% { left: calc(100% - ${segmentWidth}); }
        }

        .charging-led-sweep {
          animation: charging-led-sweep 14.25s steps(19, end) infinite;
        }
      `}</style>
    </div>
  );
}

function VehicleSideImage({
  source,
  fallbackSource,
  darkClassName,
  cropConfig,
}: {
  source: string | null | undefined;
  fallbackSource?: string | null | undefined;
  darkClassName: string;
  cropConfig: ChargingCropConfig;
}) {
  const translation = [`${cropConfig.translateX}%`];
  if (cropConfig.translateY != null && cropConfig.translateY !== 0) translation.push(`${cropConfig.translateY}%`);
  const transform = [
    `translate${translation.length === 1 ? 'X' : ''}(${translation.join(', ')})`,
    `scale(${cropConfig.scale})`,
  ].join(' ');
  return (
    <div className={`absolute inset-y-0 right-0 flex h-full w-full items-center justify-end ${darkClassName}`}>
      <AuthenticatedVehicleArtwork
        source={source}
        fallbackSource={fallbackSource}
        fallbackProps={{
          className: 'h-full w-full max-w-none object-contain object-right',
          style: {
            objectPosition: 'right center',
            transform: DEMO_CHARGING_FALLBACK_TRANSFORM,
            transformOrigin: DEMO_CHARGING_FALLBACK_ORIGIN,
          },
        }}
        alt="Vehicle side view showing charging port location"
        data-testid="charging-side-image"
        data-image-mode="charging"
        className="h-full w-auto max-w-none object-contain"
        style={{
          objectPosition: cropConfig.objectPosition ?? 'left center',
          transform,
          transformOrigin: 'left top',
        }}
      />
    </div>
  );
}

function chargingCropFamily(model: string | null | undefined): ChargingCropFamily {
  const normalized = (model ?? '').toUpperCase();
  if (normalized.includes('R1T')) return 'R1T';
  if (normalized.includes('R1S')) return 'R1S';
  return 'default';
}

function formatTimeToFull(minutes: number | null | undefined) {
  return formatWidgetChargeTime(minutes, '-');
}

function formatMaybePercent(value: number | null | undefined, digits: number) {
  return value == null ? '-' : formatDashboardPercent(value, digits);
}

function isActivelyCharging(status: VehicleStatus | null | undefined) {
  return isVehicleCharging(status);
}

registerWidget({
  componentType: 'custom',
  definitionId: 'charging.connection',
  title: 'Charging Connection',
  defaultSize: { w: 6, h: 6 },
  minSize: { w: 5, h: 5 },
  defaultOptions: {},
  dataRequirements: () => ({ status: true, chargingSummary: true }),
  editor: {
    category: 'Charging',
    fixedSize: true,
    description: 'Custom charging chip with plug-aware artwork and fixed composition.',
  },
  component: ChargingConnectionWidget,
});
