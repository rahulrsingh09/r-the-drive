export const descriptions: Record<string, string> = {
  vehicles: 'Connection, sharing and vehicle configuration',
  dashboards: 'Open and export your saved layouts',
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


type Section = { id: string; label: string };

export function settingsDirectory<T extends Section>(sections: T[], search: string) {
  const term = search.trim().toLowerCase();
  const buckets = new Map<string, T[]>();
  const extraIds: string[] = [];
  const knownIds = new Set(groups.flatMap(group => group.ids));
  let matches = 0;
  for (const section of sections) {
    if (!knownIds.has(section.id)) extraIds.push(section.id);
    const searchable = `${section.label} ${descriptions[section.id] ?? ''}`.toLowerCase();
    if (!searchable.includes(term)) continue;
    matches++;
    const bucket = buckets.get(section.id) ?? [];
    bucket.push(section);
    buckets.set(section.id, bucket);
  }
  const directory = [...groups, { title: 'More settings', ids: extraIds }].map(group => ({
    title: group.title, items: group.ids.flatMap(id => buckets.get(id) ?? []),
  }));
  return { matches, primary: directory.slice(0, 3), secondary: directory.slice(3) };
}

export function settingsLabel(section: Section) {
  return section.id === 'units' ? 'Units & time' : section.label;
}
