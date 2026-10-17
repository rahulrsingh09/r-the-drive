import * as React from 'react';
import {
  BatteryCharging,
  BatteryFull,
  BatteryLow,
  BatteryMedium,
  BatteryWarning,
  Loader2,
  TriangleAlert,
  Wifi,
  WifiOff,
} from 'lucide-react';
import { cn, formatMiles } from '../lib/utils';

export type VehicleOnlineState = 'online' | 'offline' | 'connecting' | 'unhealthy' | 'error';

export interface StatusBarProps {
  vehicleName?: string | undefined;
  onlineState: VehicleOnlineState;
  socPercent?: number | undefined;
  isCharging?: boolean | undefined;
  rangeEstimateMi?: number | undefined;
  compact?: boolean | undefined;
  size?: 'default' | 'menu' | undefined;
  className?: string | undefined;
}

const BATTERY_LEVELS = [
  { above: 66, Component: BatteryFull, variant: 'full' },
  { above: 33, Component: BatteryMedium, variant: 'medium' },
  { above: 10, Component: BatteryLow, variant: 'low' },
] as const;
const CONNECTION = {
  online: { label: 'Online', icon: Wifi, color: 'text-status-positive', spin: false },
  connecting: { label: 'Reconnecting...', icon: Loader2, color: 'text-accent', spin: true },
  unhealthy: { label: 'Feed unhealthy', icon: TriangleAlert, color: 'text-status-danger', spin: false },
  error: { label: 'Connection failed', icon: WifiOff, color: 'text-status-danger', spin: false },
  offline: { label: 'Offline', icon: WifiOff, color: 'text-fg-tertiary', spin: false },
};
export function getBatteryIcon(socPercent: number) {
  const level = BATTERY_LEVELS.find(level => socPercent > level.above);
  return level ? { Component: level.Component, variant: level.variant } : { Component: BatteryWarning, variant: 'critical' };
}

export function StatusBar({
  vehicleName,
  onlineState,
  socPercent,
  isCharging,
  rangeEstimateMi,
  compact = false,
  size = 'default',
  className,
}: StatusBarProps) {
  const batteryIcon =
    socPercent !== undefined
      ? isCharging
        ? { Component: BatteryCharging, variant: 'charging' }
        : getBatteryIcon(socPercent)
      : undefined;
  const connection = CONNECTION[onlineState] ?? CONNECTION.offline;
  const statusLabel = connection.label;
  const ConnectionIcon = connection.icon;

  const showBattery = socPercent !== undefined && onlineState === 'online';
  const shouldCenterConnection = compact && !showBattery;

  return (
    <div
      className={cn(
        'w-full flex items-center gap-3 px-3 py-2 rounded-lg',
        compact && 'justify-between px-2',
        shouldCenterConnection && 'justify-center',
        className
      )}
    >
      <div
        className={cn('flex items-center gap-1.5', compact && 'gap-0')}
        title={`Vehicle status: ${statusLabel}`}
        aria-label={`Vehicle status: ${statusLabel}`}
      >
        <ConnectionIcon className={cn(size === 'menu' ? 'h-5 w-5' : 'h-4 w-4', 'shrink-0', connection.color, connection.spin && 'animate-spin')} />
        {!compact && (
          <span
            className={cn(
              'text-sm font-medium',
              connection.color
            )}
          >
            {statusLabel}
          </span>
        )}
      </div>

      {!compact && vehicleName && (
        <span className="text-sm font-medium text-fg-tertiary truncate max-w-[120px]">{vehicleName}</span>
      )}

      {showBattery && (
        <div
          className={cn('flex items-center gap-1 ml-auto', compact && 'gap-0')}
          title={`Battery status: ${Math.round(socPercent)}%`}
          aria-label={`Battery status: ${Math.round(socPercent)}%`}
        >
          {batteryIcon && (
            <batteryIcon.Component
              className={cn(
                size === 'menu' ? 'h-6 w-6' : 'h-5 w-5',
                'relative top-px',
                'shrink-0',
                isCharging
                  ? 'text-accent'
                  : socPercent > 50
                    ? 'text-status-positive'
                    : socPercent > 20
                      ? 'text-status-warning'
                      : 'text-status-danger'
              )}
              data-battery-icon={`battery-${batteryIcon.variant}`}
            />
          )}
          {!compact && (
            <span className="text-sm font-medium tabular-nums">
              <span className="text-fg">{Math.round(socPercent)}%</span>
              {rangeEstimateMi !== undefined && (
                <span className="text-fg-tertiary">({formatMiles(rangeEstimateMi).replace(' ', '')})</span>
              )}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
