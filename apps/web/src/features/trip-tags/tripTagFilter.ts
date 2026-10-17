import { z } from 'zod';

export const tripTagSearchSchema = z.object({
  tag_ids: z.string().optional(),
  tag_match: z.enum(['all', 'any']).optional(),
  untagged: z.literal('1').optional(),
});

export type TripTagSearch = z.infer<typeof tripTagSearchSchema>;
export type TripTagMatch = 'all' | 'any';

export interface TripTagFilter {
  tagIds: string[];
  tagMatch: TripTagMatch;
  untagged: boolean;
}

export interface TripTagFilterSelection extends TripTagFilter {
  setFilter: (next: TripTagFilter) => void;
}

export function normalizeTripTagIds(tagIds: string | readonly string[] | undefined): string[] {
  const unique = new Set<string>();
  for (const part of typeof tagIds === 'string' ? tagIds.split(',') : tagIds ?? []) {
    const id = part.trim();
    if (id.length) unique.add(id);
  }
  return Array.from(unique).sort();
}

export function parseTripTagFilter(search: TripTagSearch | null | undefined): TripTagFilter {
  const tagIds = normalizeTripTagIds(search?.tag_ids);
  const filter: TripTagFilter = { tagIds, tagMatch: 'all', untagged: false };
  if (search?.tag_match) filter.tagMatch = search.tag_match;
  if (!tagIds.length) filter.untagged = search?.untagged === '1';
  return filter;
}

export function serializeTripTagFilter(filter: TripTagFilter): TripTagSearch {
  const tagIds = normalizeTripTagIds(filter.tagIds);
  const search: TripTagSearch = {};
  if (!tagIds.length) {
    if (filter.untagged) search.untagged = '1';
    return search;
  }
  search.tag_ids = tagIds.join(',');
  if (filter.tagMatch === 'any') search.tag_match = 'any';
  return search;
}

export function createTripTagFilterAdapter(
  search: TripTagSearch | null | undefined,
  setSearch: (search: TripTagSearch) => void,
): TripTagFilterSelection {
  return {
    ...parseTripTagFilter(search),
    setFilter: (next) => setSearch(serializeTripTagFilter(next)),
  };
}
