import type { useChargeSession } from '@panda/hooks';
type ChargeSessionDetail = NonNullable<ReturnType<typeof useChargeSession>['data']>;
import type { ChargeSessionUpdate, Place } from '@panda/types';
export interface ChargeCorrectionDraft {
  locationMode: NonNullable<ChargeSessionDetail['location_override_mode']>;
  costMode: NonNullable<ChargeSessionDetail['cost_override_mode']>;
  placeId: string;
  manualCost: string;
}
export function chargeCorrectionDraft(session: ChargeSessionDetail, places: Place[]): ChargeCorrectionDraft {
  const location = session.location_name?.trim().toLocaleLowerCase();
  const matchedPlace = places.find((place) => place.name.trim().toLocaleLowerCase() === location);
  return { locationMode: session.location_override_mode ?? 'automatic', costMode: session.cost_override_mode ?? 'automatic', placeId: matchedPlace?.id ?? '', manualCost: session.cost_override_usd?.toFixed(2) ?? '' };
}
export function chargeCorrectionResult(draft: ChargeCorrectionDraft) {
  const amount = Number(draft.manualCost.trim());
  const manualCostError = draft.costMode === 'manual' && (!draft.manualCost.trim() || !Number.isFinite(amount) || amount < 0) ? 'Enter a non-negative USD amount.' : undefined;
  const locationError = draft.locationMode === 'saved_place' && !draft.placeId ? 'Choose a saved place before saving.' : undefined;
  const body: ChargeSessionUpdate = { location_mode: draft.locationMode, cost_mode: draft.costMode };
  if (draft.locationMode === 'saved_place') body.place_id = draft.placeId;
  if (draft.costMode === 'manual') body.cost_usd = amount;
  return { manualCostError, locationError, body: manualCostError || locationError ? null : body };
}
export function chargeCorrectionTransition(state: ChargeCorrectionDraft, event: { type: 'load'; session: ChargeSessionDetail; places: Place[] } | { type: 'edit'; patch: Partial<ChargeCorrectionDraft> }) {
  return event.type === 'load' ? chargeCorrectionDraft(event.session, event.places) : { ...state, ...event.patch };
}
