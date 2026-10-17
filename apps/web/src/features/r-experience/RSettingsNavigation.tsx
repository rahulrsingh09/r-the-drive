import React from 'react';
import { ArrowLeft, ArrowRight, Search } from 'lucide-react';
import { SelectPicker } from '@panda/ui/primitives';
import { descriptions, settingsDirectory, settingsLabel } from './settingsModel';
import './r-settings.css';


export function RSettingsNavigation<T extends string>({ sections, active, onSelect }: {
  sections: Array<{ id: T; label: string; icon: React.ElementType }>;
  active: T | 'directory';
  onSelect: (section: T | 'directory') => void;
}) {
  const [search, setSearch] = React.useState('');
  const available = sections;
  const selectSection = (id: string) => {
    if (id === 'directory') onSelect('directory');
    else {
      const selected = available.find(section => section.id === id);
      if (selected) onSelect(selected.id);
    }
  };
  if (active !== 'directory') return (
    <nav className="r-settings-navigation" aria-label="Settings sections">
      <button type="button" onClick={() => selectSection('directory')}><ArrowLeft />All settings</button>
      <SelectPicker value={active} onChange={selectSection} aria-label="Settings section"
        options={available.map(section => ({ value: section.id, label: settingsLabel(section) }))} />
    </nav>
  );
  const directory = settingsDirectory(available, search);
  const renderGroup = ({ title, items }: typeof directory.primary[number]) => {
    if (items.length === 0) return null;
    return <section key={title} className="r-settings-group"><h2>{title}</h2><div className="r-directory">
      {items.map(section => {
        const Icon = section.icon;
        return <button type="button" key={section.id} onClick={() => selectSection(section.id)}>
          <Icon aria-hidden="true" /><span><strong>{settingsLabel(section)}</strong><small>{descriptions[section.id]}</small></span><ArrowRight aria-hidden="true" />
        </button>;
      })}
    </div></section>;
  };
  return (
    <div className="r-settings-directory">
      <label className="r-settings-search"><Search aria-hidden="true" /><input value={search} onChange={event => setSearch(event.target.value)} placeholder="Find a setting" aria-label="Find a setting" type="search" /></label>
      {!directory.matches && <p className="r-empty" role="status">No settings match “{search}”.</p>}
      <div className="r-settings-columns"><div>{directory.primary.map(renderGroup)}</div><div>{directory.secondary.map(renderGroup)}</div></div>
    </div>
  );
}
