import type { RawTelemetryQuery } from '@panda/types';

export function presentParams(values: Record<string, unknown>, presence: 'truthy' | 'defined' = 'truthy'): Record<string, string | number> {
  const entries = Object.entries(values).filter(([, value]) => presence === 'defined' ? value !== undefined : Boolean(value));
  return Object.fromEntries(entries) as Record<string, string | number>;
}

export function pageParams(page: number, size: number) {
  return { page, per_page: size, limit: size, offset: (page - 1) * size };
}

export function tripSelection(filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }) {
  const unique = (filters?.tagIds ?? []).slice().sort().filter((id, index, ordered) => index === 0 || id !== ordered[index - 1]);
  const tagged = unique.length ? { tag_ids: unique.join(','), tag_match: filters?.tagMatch ?? 'all' } : {};
  return { ...tagged, ...presentParams({ untagged: filters?.untagged ? 1 : undefined }) };
}

export function telemetryParams(options: RawTelemetryQuery | number) {
  const query: RawTelemetryQuery = typeof options === 'number' ? {} : options;
  const pagination = typeof options === 'number' ? { limit: options } : { per_page: query.per_page };
  const fields = query.fields?.reduce<string[]>((selected, field) => field ? [...selected, field] : selected, []).join(',');
  return presentParams({ from: query.from, to: query.to, page: query.page, ...pagination, search: query.search?.trim(), fields, populated_only: query.populated_only ? 'true' : undefined });
}
