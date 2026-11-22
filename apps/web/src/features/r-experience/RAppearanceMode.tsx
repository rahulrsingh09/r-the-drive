import React from 'react';
import type { ThemeMode } from '@panda/types';
import { useThemePreferenceController } from '../../hooks/useThemePreferenceController';

const MODES: Array<{ value: ThemeMode; label: string; help: string }> = [
  { value: 'system', label: 'System', help: 'Follows your device setting.' },
  { value: 'light', label: 'Light', help: 'Always light.' },
  { value: 'dark', label: 'Dark', help: 'Always dark.' },
];

/** Light, dark or system appearance for the R experience. Saved to the account. */
export function RAppearanceMode() {
  const theme = useThemePreferenceController();
  const groupId = React.useId();
  const active = MODES.find(mode => mode.value === theme.mode) ?? MODES[0]!;
  return (
    <section className="r-appearance-mode grid gap-3 rounded-xl border border-border bg-bg-surface p-4" aria-labelledby={`${groupId}-title`}>
      <h2 id={`${groupId}-title`} className="text-base font-medium text-fg">Appearance</h2>
      <div role="radiogroup" aria-labelledby={`${groupId}-title`} className="grid grid-cols-3 gap-1 rounded-xl bg-bg-elevated p-1">
        {MODES.map(mode => (
          <button key={mode.value} type="button" role="radio" aria-checked={theme.mode === mode.value}
            disabled={theme.isPending}
            onClick={() => { if (theme.mode !== mode.value) theme.onModeChange(mode.value); }}
            className={`min-h-11 rounded-lg px-3 text-sm transition-colors disabled:opacity-60 ${theme.mode === mode.value ? 'bg-bg-surface font-semibold text-fg' : 'text-fg-secondary hover:text-fg'}`}>
            {mode.label}
          </button>
        ))}
      </div>
      <p className="text-sm text-fg-secondary">{active.help}</p>
    </section>
  );
}
