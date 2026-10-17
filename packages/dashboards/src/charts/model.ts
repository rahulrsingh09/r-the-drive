import type {
  ChartManagerEntry,
  ChartOrigin,
  ChartRecord,
  ChartPermissions,
} from '@panda/types';

export function chartOrigin(record: ChartRecord, systemBase?: ChartRecord): ChartOrigin {
  if (!record.ownerId) return 'system';
  return systemBase ? 'override' : 'personal';
}

export function defaultChartPermissions(record: ChartRecord, isAdmin = false): ChartPermissions {
  const isSystem = record.ownerId == null;
  const editable = isSystem ? isAdmin : true;
  return {
    read: true,
    edit: editable && !record.isLocked,
    duplicate: true,
    reset: false,
    restore: isSystem && isAdmin,
    delete: !isSystem,
    lock: isSystem && isAdmin,
  };
}

export function buildChartManagerEntries(
  rows: ChartRecord[],
  userId: string | null | undefined,
  isAdmin = false,
): ChartManagerEntry[] {
  const grouped = new Map<string, { system?: ChartRecord; personal?: ChartRecord }>();
  const systemOrder = new Set<string>();
  const personalOrder = new Set<string>();
  for (const row of rows) {
    const system = row.ownerId == null;
    const personal = row.ownerId === userId;
    if (!system && !personal) continue;
    const group = grouped.get(row.slug) ?? {};
    if (system) { group.system = row; systemOrder.add(row.slug); }
    if (personal) { group.personal = row; personalOrder.add(row.slug); }
    grouped.set(row.slug, group);
  }
  const entries: ChartManagerEntry[] = [];
  for (const slug of new Set([...systemOrder, ...personalOrder])) {
    const { system, personal } = grouped.get(slug)!;
    const effective = (personal ?? system)!;
    const entry: ChartManagerEntry = {
      effective,
      origin: chartOrigin(effective, system),
      permissions: defaultChartPermissions(effective, isAdmin),
    };
    if (system) entry.systemBase = system;
    if (personal) entry.personalOverride = personal;
    entry.permissions.reset = Boolean(personal && system);
    entries.push(entry);
  }
  return entries.sort((left, right) => left.effective.name.localeCompare(right.effective.name));
}

export function resolveAssignedCharts(
  entries: ChartManagerEntry[],
  dashboardSlug: string,
  requiredSlugs: string[] = [],
) {
  const required = new Set(requiredSlugs);
  const assigned: ChartRecord[] = [];
  for (const { effective } of entries) {
    if (!effective.isEnabled) continue;
    if (required.has(effective.slug) || chartHasPlacement(effective, dashboardSlug)) assigned.push(effective);
  }
  return assigned.sort((left, right) => left.name.localeCompare(right.name));
}

export function chartHasPlacement(record: ChartRecord, dashboardSlug: string) {
  return record.config.placements.some((placement) => placement.dashboardSlug === dashboardSlug);
}

export function withChartPlacement(record: ChartRecord, dashboardSlug: string, assigned: boolean): ChartRecord {
  const placements: ChartRecord['config']['placements'] = [];
  let ordered = true;
  for (const placement of record.config.placements) {
    if (placement.dashboardSlug === dashboardSlug) continue;
    const previous = placements.at(-1);
    if (previous && previous.dashboardSlug.localeCompare(placement.dashboardSlug) > 0) ordered = false;
    placements.push(placement);
  }
  if (!ordered) placements.sort((left, right) => left.dashboardSlug.localeCompare(right.dashboardSlug));
  if (assigned) {
    let start = 0;
    let end = placements.length;
    while (start < end) {
      const middle = Math.floor((start + end) / 2);
      if (placements[middle]!.dashboardSlug.localeCompare(dashboardSlug) <= 0) start = middle + 1;
      else end = middle;
    }
    placements.splice(start, 0, { dashboardSlug });
  }
  return { ...record, config: { ...record.config, placements } };
}
