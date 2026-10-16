import React from 'react';
import { ArrowLeft, ArrowRight, Search } from 'lucide-react';
import { SelectPicker } from '@panda/ui/primitives';
import './r-settings.css';

const descriptions: Record<string, string> = {
  vehicles: 'Connection, sharing and vehicle configuration',
  dashboards: 'Saved layouts and dashboard editing',
  charts: 'Your charts, sources and definitions',
  units: 'Measurements, efficiency display, time zone and maps',
  places: 'Named places and location boundaries',
  charging: 'Energy prices and charging preferences',
  external: 'Optional services and connections',
  api: 'Scoped API keys and endpoint reference',
  jobs: 'Imports and history processing',
  raw: 'Recorded telemetry and diagnostic captures',
  backup: 'Backup schedules, downloads and restore',
  account: 'Profile, password and sign out',
  authentication: 'Password and single sign-on policies',
};
const groups = [
  { title: 'Your vehicle', ids: ['vehicles', 'places', 'charging'] },
  { title: 'Your workspace', ids: ['dashboards', 'charts', 'units'] },
  { title: 'Your account', ids: ['account'] },
  { title: 'Data & administration', ids: ['external', 'api', 'jobs', 'raw', 'backup', 'authentication'] },
];

export function RSettingsNavigation<T extends string>({ sections, active, onSelect }: {
  sections: Array<{ id: T; label: string; icon: React.ElementType }>;
  active: T | 'directory';
  onSelect: (section: T | 'directory') => void;
}) {
  const [search, setSearch] = React.useState('');
  if (active !== 'directory') return (
    <nav className="r-settings-navigation" aria-label="Settings sections">
      <button type="button" onClick={() => onSelect('directory')}><ArrowLeft />All settings</button>
      <SelectPicker value={active} onChange={value => onSelect(value as T)} aria-label="Settings section"
        options={sections.map(section => ({ value: section.id, label: section.id === 'units' ? 'Units & time' : section.label }))} />
    </nav>
  );
  const query = search.trim().toLowerCase();
  const shown = sections.filter(section => `${section.label} ${descriptions[section.id] ?? ''}`.toLowerCase().includes(query));
  const known = new Set(groups.flatMap(group => group.ids));
  const additional = { title: 'More settings', ids: sections.filter(section => !known.has(section.id)).map(section => section.id) };
  const renderGroup = (group: typeof groups[number]) => {
    const items = group.ids.flatMap(id => shown.filter(section => section.id === id));
    return items.length ? <section key={group.title} className="r-settings-group"><h2>{group.title}</h2><div className="r-directory">
      {items.map(({ id, label, icon: Icon }) => <button type="button" key={id} onClick={() => onSelect(id)}>
        <Icon aria-hidden="true" /><span><strong>{id === 'units' ? 'Units & time' : label}</strong><small>{descriptions[id]}</small></span><ArrowRight aria-hidden="true" />
      </button>)}
    </div></section> : null;
  };
  return (
    <div className="r-settings-directory">
      <label className="r-settings-search"><Search aria-hidden="true" /><input value={search} onChange={event => setSearch(event.target.value)} placeholder="Find a setting" aria-label="Find a setting" type="search" /></label>
      {!shown.length && <p className="r-empty" role="status">No settings match “{search}”.</p>}
      <div className="r-settings-columns"><div>{groups.slice(0, 3).map(renderGroup)}</div><div>{groups.slice(3).map(renderGroup)}{renderGroup(additional)}</div></div>
    </div>
  );
}
