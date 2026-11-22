import type { Place } from '@panda/types';

/** A place tag as the owner sees it. */
export type PlaceTagKind = 'home' | 'office' | 'other';

const LABEL: Record<PlaceTagKind, string> = { home: 'Home', office: 'Office', other: 'Other' };

export function placeTagLabel(kind: PlaceTagKind) {
  return LABEL[kind];
}

/** The saved Home or Office place whose rate a tag uses. */
function taggedPlace(places: readonly Place[], kind: 'home' | 'office') {
  const flagged = places.filter(place => (kind === 'home' ? place.is_home : place.is_work));
  return flagged.find(place => place.charging) ?? flagged[0]
    ?? places.find(place => place.name.trim().toLowerCase() === kind) ?? null;
}

/** The price a tag applies, in words: "$0.2006 per kWh", "free", or "the price you enter on each charge". */
export function placeTagPrice(places: readonly Place[], kind: PlaceTagKind) {
  if (kind === 'other') return 'the price you enter on each charge';
  const charging = taggedPlace(places, kind)?.charging;
  if (!charging) return `no ${LABEL[kind]} rate yet (set it in Settings)`;
  if (charging.billing_type === 'free' || charging.rate === 0) return 'free';
  if (charging.billing_type === 'per_minute') return `$${charging.rate.toFixed(2)} per minute`;
  if (charging.billing_type === 'tou') return 'your time-of-use rate';
  return `$${charging.rate.toFixed(4)} per kWh`;
}

/**
 * The confirmation for a place tag. A tag applies to every charge at the place at once,
 * so the owner sees the place, the number of charges and the new price before it saves.
 */
export function placeTagConfirm(name: string, count: number, kind: PlaceTagKind, places: readonly Place[]) {
  const charges = count === 1 ? '1 charge' : `${count} charges`;
  const [move, are] = count === 1 ? ['moves', 'is'] : ['move', 'are'];
  const price = placeTagPrice(places, kind);
  const body = kind === 'other'
    ? `${charges} at ${name} ${move} to Other: no saved place and no rate. You enter the price on each charge.`
    : `${charges} at ${name} ${move} to ${LABEL[kind]} and ${are} priced ${price === 'free' ? 'free' : `at ${price}`}.`;
  return {
    title: `Tag ${name} as ${LABEL[kind]}?`,
    body: `${body} A cost you entered on a charge stays.`,
    action: `Tag as ${LABEL[kind]}`,
  };
}
