type DriveFamily = { aliases: readonly string[]; label: string; badge: string };
const FAMILIES: DriveFamily[] = [
  { aliases: ['unknown'], label: 'Unknown', badge: 'bg-dm-unknown/10 text-dm-unknown border border-dm-unknown/20' },
  { aliases: ['winter', 'snow'], label: 'Snow', badge: 'bg-bg-elevated text-fg border border-border-strong' },
  { aliases: ['sport'], label: 'Sport', badge: 'bg-accent-muted text-accent border border-accent/20' },
  { aliases: ['everyday', 'all_purpose', 'allpurpose'], label: 'All-Purpose', badge: 'bg-dm-everyday/10 text-dm-everyday border border-dm-everyday/20' },
  { aliases: ['conserve', 'distance'], label: 'Conserve', badge: 'bg-dm-conserve/10 text-dm-conserve border border-dm-conserve/20' },
  { aliases: ['off_road_auto', 'all_terrain', 'offroadauto', 'allterrain'], label: 'All-Terrain', badge: 'bg-dm-terrain/10 text-dm-terrain border border-dm-terrain/20' },
  { aliases: ['soft_sand', 'off_road_sand', 'softsand', 'offroadsand'], label: 'Soft Sand', badge: 'bg-dm-sand/10 text-dm-sand border border-dm-sand/20' },
  { aliases: ['rock_crawl', 'off_road_rocks', 'rockcrawl', 'offroadrocks'], label: 'Rock Crawl', badge: 'bg-dm-rock/10 text-dm-rock border border-dm-rock/20' },
  { aliases: ['rally', 'off_road_sport_auto', 'offroadsportauto'], label: 'Rally', badge: 'bg-dm-rally/10 text-dm-rally border border-dm-rally/20' },
  { aliases: ['drift', 'off_road_sport_drift', 'offroadsportdrift'], label: 'Drift', badge: 'bg-dm-drift/10 text-dm-drift border border-dm-drift/20' },
  { aliases: ['towing'], label: 'Towing', badge: 'bg-dm-towing/10 text-dm-towing border border-dm-towing/20' },
];
const FAMILY_BY_ALIAS = new Map(FAMILIES.flatMap(family => family.aliases.map(alias => [alias, family] as const)));
function driveKey(value: string) {
  return value.trim().split(/(?<=[a-z0-9])(?=[A-Z])|[\s-]+/).join('_').toLowerCase();
}
export function formatDriveMode(value: string) {
  const key = driveKey(value);
  const family = FAMILY_BY_ALIAS.get(key);
  return family?.label ?? key.replace(/_/g, ' ').replace(/\b\w/g, char => char.toUpperCase());
}
export function getDriveModeBadgeClass(value: string) {
  return FAMILY_BY_ALIAS.get(driveKey(value))?.badge ?? 'bg-bg-elevated text-fg-secondary border border-border';
}
