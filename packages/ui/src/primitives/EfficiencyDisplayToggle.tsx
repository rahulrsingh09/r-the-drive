import React, { useSyncExternalStore } from 'react';
import { listen } from '../lib/browserRuntime';
import { PiSpeedometerFill, PiSpeedometerLight } from 'react-icons/pi';
import { Tooltip } from './Tooltip';
import {
  getEfficiencyDisplay,
  getEfficiencyUnitLabel,
  setEfficiencyDisplay,
} from '../lib/utils';

export interface EfficiencyDisplayToggleProps {
  className?: string;
}

function subscribeEfficiency(changed: () => void) {
  return listen(['rm-efficiency-display-change', 'rm-units-change', 'storage'].map(name => [window, name, changed] as const));
}
export function EfficiencyDisplayToggle({ className }: EfficiencyDisplayToggleProps) {
  const efficiencyDisplay = useSyncExternalStore(subscribeEfficiency, getEfficiencyDisplay, getEfficiencyDisplay);

  const isDistancePerEnergy = efficiencyDisplay === 'distance_per_energy';
  const displayLabel = getEfficiencyUnitLabel();
  const tooltip = isDistancePerEnergy
    ? `Showing ${displayLabel}. Click to switch to energy per distance.`
    : `Showing ${displayLabel}. Click to switch to distance per energy.`;
  const Icon = isDistancePerEnergy ? PiSpeedometerFill : PiSpeedometerLight;

  return (
    <Tooltip
      content={(
        <div className="grid gap-1">
          <span className="text-xs font-medium text-fg">Efficiency units</span>
          <span className="text-[11px] text-fg-secondary">{tooltip}</span>
        </div>
      )}
      contentClassName="w-60"
    >
      <button
        type="button"
        className={[
          'inline-flex h-9 w-9 items-center justify-center rounded-lg border border-border bg-bg-elevated text-fg-secondary transition-colors hover:border-border-strong hover:text-fg focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent',
          className,
        ]
          .filter(Boolean)
          .join(' ')}
        onClick={() => {
          const next = isDistancePerEnergy ? 'energy_per_distance' : 'distance_per_energy';
          setEfficiencyDisplay(next);
        }}
        aria-label={`Toggle efficiency units, currently ${displayLabel}`}
        aria-pressed={isDistancePerEnergy}
      >
        <Icon className="h-4 w-4" />
      </button>
    </Tooltip>
  );
}
