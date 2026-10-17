export function activeDestination(activeKey: string, visibleKeys: readonly string[]): string {
  const requested = activeKey.startsWith('battery') ? 'battery' : activeKey;
  return requested === 'battery' || visibleKeys.includes(requested) ? requested : 'explore';
}

export function canManagePeople(role: string | undefined) {
  return new Set(['admin', 'super_user']).has(role ?? '');
}

export const batterySections = [
  { to: '/battery', label: 'Battery overview' },
  { to: '/battery/phantom-drain', label: 'Phantom drain' },
] as const;

const exploreSections = [
  { to: '/efficiency', title: 'Efficiency', description: 'Trends, temperature and trip comparisons' },
  { to: '/vehicle-health', title: 'Vehicle health', description: 'Readings, tires, software and connection' },
  { to: '/battery', title: 'Battery', description: 'State of charge and parked energy' },
  { to: '/battery/phantom-drain', title: 'Phantom drain', description: 'Parked energy, sleep and battery loss' },
  { to: '/settings', search: { section: 'dashboards' }, title: 'Dashboard library', description: 'Your views and saved layouts' },
  { to: '/settings', title: 'All settings', description: 'Preferences, connections and administration' },
] as const;
const peopleSection = { to: '/users', title: 'People & access', description: 'Accounts, invitations and permissions' } as const;

export function exploreDirectory(role: string | undefined) {
  const sections: Array<typeof exploreSections[number] | typeof peopleSection> = [...exploreSections];
  if (canManagePeople(role)) sections.push(peopleSection);
  return sections;
}

export function primaryNavigation<T extends { key: string }>(activeKey: string, entries: readonly T[]) {
  const active = activeDestination(activeKey, entries.map(entry => entry.key));
  return entries.map(entry => ({
    ...entry, current: entry.key === active ? 'page' as const : undefined,
  }));
}

export function lockPresentation(locked: boolean | null) {
  const state = locked === null ? 'unknown' : String(locked) as 'true' | 'false';
  const labels = {
    unknown: 'Vehicle lock status unavailable', true: 'Vehicle locked', false: 'Vehicle unlocked',
  };
  return { label: labels[state], value: locked ?? 'unknown', icon: state === 'unknown' ? 'unknown' as const : 'lock' as const };
}
