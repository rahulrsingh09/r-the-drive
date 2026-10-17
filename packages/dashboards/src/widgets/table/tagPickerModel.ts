import type { TripTag } from '@panda/types';
export function normalizeTagQuery(value: string) {
  return value.normalize('NFC').trim().replace(/\s+/g, ' ').toLocaleLowerCase();
}
export function tagCatalog(tags: TripTag[], selectedIds: string[], query: string) {
  const needle = normalizeTagQuery(query),
    selection = new Set(selectedIds);
  const filtered: TripTag[] = [],
    selected: TripTag[] = [];
  let exact: TripTag | undefined;
  for (const tag of tags) {
    const name = normalizeTagQuery(tag.name);
    if (name.includes(needle)) filtered.push(tag);
    if (name === needle && !exact) exact = tag;
    if (selection.has(tag.id)) selected.push(tag);
  }
  return { filtered, selected, exact, normalizedQuery: needle };
}
export function commonTripTags(trips: Array<{ tags?: Array<Pick<TripTag, 'id'>> | undefined }>) {
  const counts = new Map<string, number>();
  for (const trip of trips) {
    const unique = new Set(trip.tags?.map((tag) => tag.id));
    unique.forEach((id) => counts.set(id, (counts.get(id) ?? 0) + 1));
  }
  return [...counts].flatMap(([id, count]) => (count === trips.length ? [id] : [])).sort();
}
export function changeTagSelection(
  selected: string[],
  id: string,
  intent: 'add' | 'remove' | 'toggle'
) {
  const include = intent === 'add' || (intent === 'toggle' && !selected.includes(id));
  if (include) return selected.includes(id) ? selected : selected.concat(id).sort();
  return selected.filter((selectedId) => selectedId !== id);
}
export function staleTagSelection(tags: TripTag[], selected: string[], pending: Set<string>) {
  const available = new Set(tags.map((tag) => tag.id));
  available.forEach((id) => pending.delete(id));
  const stale: string[] = [],
    retained: string[] = [];
  for (const id of selected) (available.has(id) || pending.has(id) ? retained : stale).push(id);
  return { stale, retained };
}
export async function createOrRecoverTag(
  create: () => Promise<TripTag>,
  refetch: () => Promise<{ data?: TripTag[] | undefined }>,
  normalizedQuery: string
) {
  try {
    return { tag: await create(), created: true };
  } catch {
    try {
      const catalog = await refetch();
      const tag = catalog.data?.find(
        (candidate) => normalizeTagQuery(candidate.name) === normalizedQuery
      );
      return tag ? { tag, created: false } : null;
    } catch {
      return null;
    }
  }
}

type KeyIntent =
  | { type: 'index'; value: number }
  | { type: 'pick' | 'remove'; id: string }
  | { type: 'create' | 'close' };
export function tagKeyIntent(
  key: string,
  inline: boolean,
  query: string,
  activeIndex: number,
  filtered: TripTag[],
  selectedIds: string[],
  canCreate: boolean,
  optionCount: number
): KeyIntent | null {
  const direction = key === 'ArrowDown' ? 1 : key === 'ArrowUp' ? -1 : 0;
  if (direction)
    return {
      type: 'index',
      value:
        direction > 0
          ? Math.min(activeIndex + 1, Math.max(optionCount - 1, 0))
          : Math.max(activeIndex - 1, 0),
    };
  if (key === 'Escape' && !inline) return { type: 'close' };
  const submit = inline
    ? (key === 'Enter' || key === 'Tab') && Boolean(query.trim())
    : key === 'Enter';
  if (submit) {
    const option = filtered[activeIndex];
    if (option) return { type: 'pick', id: option.id };
    if (canCreate && (!inline || activeIndex === filtered.length)) return { type: 'create' };
  }
  if (key === 'Backspace' && !query && selectedIds.length)
    return { type: 'remove', id: selectedIds.at(-1)! };
  return null;
}
export function cyclePickerFocus(event: React.KeyboardEvent, panel: HTMLElement | null) {
  if (event.key !== 'Tab' || !panel) return;
  const candidates = panel.querySelectorAll<HTMLElement>(
    'button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])'
  );
  const first = candidates[0],
    last = candidates[candidates.length - 1];
  const from = event.shiftKey ? first : last,
    to = event.shiftKey ? last : first;
  if (from && to && document.activeElement === from) {
    event.preventDefault();
    to.focus();
  }
}

export function tagChoices(tags: TripTag[], selectedIds: string[], activeIndex: number) {
  const selected = new Set(selectedIds);
  return tags.map((tag, index) => ({
    tag,
    selected: selected.has(tag.id),
    active: index === activeIndex,
  }));
}

export function tagBadgeWindow(tags: TripTag[] | undefined) {
  const visible = tags?.slice(0, 3) ?? [];
  return { visible, extra: Math.max(0, (tags?.length ?? 0) - visible.length) };
}
