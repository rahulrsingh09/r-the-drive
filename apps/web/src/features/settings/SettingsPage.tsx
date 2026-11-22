import React from 'react';
import { RAppLayout, RPageHeader } from '../r-experience/RAppLayout';
import { RAccountCard } from './RAccountCard';
import { RSignInCard } from './RSignInCard';
import { RRivianCard } from './RRivianCard';
import { RRatesCard } from './RRatesCard';
import { RBackupsCard } from './RBackupsCard';
import '../r-experience/r-settings.css';

/** The five Settings cards. Old `?section=` values open the card that replaced them. */
export const R_SETTINGS_SECTIONS = ['account', 'sign-in', 'rivian', 'rates', 'backups'] as const;
export type RSettingsSection = typeof R_SETTINGS_SECTIONS[number];

const LEGACY_SECTIONS: Record<string, RSettingsSection> = {
  account: 'account',
  authentication: 'sign-in',
  vehicles: 'rivian',
  charging: 'rates',
  places: 'rates',
  backup: 'backups',
};

export function rSettingsSection(value: string | undefined): RSettingsSection | undefined {
  if (!value) return undefined;
  return (R_SETTINGS_SECTIONS as readonly string[]).includes(value) ? value as RSettingsSection : LEGACY_SECTIONS[value];
}

export function SettingsContent({ initialSection, oidcFeedback, oidcFeedbackKind }: {
  initialSection?: RSettingsSection | undefined;
  oidcFeedback?: string;
  oidcFeedbackKind?: 'success' | 'error';
} = {}) {
  React.useEffect(() => {
    if (!initialSection) return;
    const target = document.getElementById(initialSection);
    if (target && typeof target.scrollIntoView === 'function') target.scrollIntoView({ block: 'start' });
    if (initialSection === 'backups') target?.querySelector('details')?.setAttribute('open', '');
  }, [initialSection]);
  return (
    <RAppLayout activeKey="settings">
      <RPageHeader title="Settings" sub="Account, sign-in and rates" />
      {oidcFeedback && (
        <p className={`r-banner ${oidcFeedbackKind === 'error' ? 'r-banner-error' : 'r-banner-offline'}`} role={oidcFeedbackKind === 'error' ? 'alert' : 'status'}>
          <span>{oidcFeedback}</span>
        </p>
      )}
      <div className="r-set-grid">
        <RAccountCard />
        <RSignInCard />
        <RRivianCard />
        <RRatesCard />
        <RBackupsCard />
      </div>
    </RAppLayout>
  );
}
