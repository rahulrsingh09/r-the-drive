import React from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { api, invalidateChargingData, useSavedPlaces } from '@panda/hooks';
import type { Place, PlaceAddress, UpsertPlaceBody } from '@panda/types';

export type RRateKind = 'home' | 'office';

/** The Home and Office places whose cost profiles price charges. */
export function rRatePlaces(places: Place[] | undefined): Record<RRateKind, Place | null> {
  const list = places ?? [];
  return {
    home: list.find(place => place.is_home) ?? list.find(place => /^home$/i.test(place.name.trim())) ?? null,
    office: list.find(place => place.is_work) ?? list.find(place => /^(office|work)$/i.test(place.name.trim())) ?? null,
  };
}

/** $ per kWh of a place, 0 when free, null when there is no flat rate (none set, or time-of-use). */
export function rPlaceRate(place: Place | null): number | null {
  const charging = place?.charging;
  if (!charging) return null;
  if (charging.billing_type === 'free') return 0;
  if (charging.billing_type === 'per_kwh' || charging.billing_type === 'flat') return charging.rate;
  return null;
}

/** Parses "$0.19", "0,19" or "0.189119". Returns null for anything that is not a rate from 0 to 5. */
export function rParseRate(input: string): number | null {
  const cleaned = input.trim().replace(/^\$/, '').replace(',', '.');
  if (!/^\d*\.?\d+$/.test(cleaned)) return null;
  const value = Number(cleaned);
  return Number.isFinite(value) && value >= 0 && value <= 5 ? value : null;
}

function fallbackAddress(place: Place): PlaceAddress {
  return {
    display_name: place.name, osm_id: null, latitude: place.latitude, longitude: place.longitude,
    road: null, city: null, state: null, postcode: null, country: null, raw: null,
  };
}

/** Body for PUT /v1/places/{id} that keeps the place and changes only its flat rate. */
export function rRateUpdateBody(place: Place, rate: number): UpsertPlaceBody {
  return {
    name: place.name,
    radius_m: place.radius_m,
    is_home: place.is_home,
    is_work: place.is_work,
    address: place.address ?? fallbackAddress(place),
    charging: {
      name: place.charging?.name ?? null,
      billing_type: 'per_kwh',
      rate,
      session_fee: place.charging?.session_fee ?? 0,
      currency: place.charging?.currency ?? 'USD',
      timezone: place.charging?.timezone ?? null,
    },
  };
}

const LABEL: Record<RRateKind, string> = { home: 'Home', office: 'Office' };

/** Where the Home default rate ($0.189119 per kWh) comes from, as in v5. */
const R_HOME_RATE_SOURCE = 'Home default: PSE Electric Summary Sheet S-6, Schedule 7, effective 2026-10-01, first 600 kWh per month, before city tax.';
const SWATCH: Record<RRateKind, string> = { home: 'var(--r-home)', office: 'var(--r-office)' };

/** Home and Office $ per kWh form. Saves to the place cost profiles through the places API. */
export function RRatesForm({ submitLabel = 'Save rates', big = false, onSaved }: { submitLabel?: string; big?: boolean; onSaved?: () => void }) {
  const queryClient = useQueryClient();
  const places = useSavedPlaces();
  const targets = rRatePlaces(places.data);
  const [values, setValues] = React.useState<Record<RRateKind, string> | null>(null);
  const [message, setMessage] = React.useState<{ text: string; error: boolean } | null>(null);
  React.useEffect(() => {
    if (!places.data || values) return;
    const show = (kind: RRateKind) => { const rate = rPlaceRate(targets[kind]); return rate == null ? '' : String(rate); };
    setValues({ home: show('home'), office: show('office') });
  }, [places.data, targets, values]);

  const save = useMutation({
    mutationFn: async (next: Record<RRateKind, number | null>) => {
      const writes = (Object.keys(next) as RRateKind[]).flatMap(kind => {
        const place = targets[kind];
        const rate = next[kind];
        if (!place || rate == null || rate === rPlaceRate(place)) return [];
        return [api.updatePlace(place.id, rRateUpdateBody(place, rate))];
      });
      await Promise.all(writes);
      return writes.length;
    },
    onSuccess: (count) => {
      void queryClient.invalidateQueries({ queryKey: ['places'] });
      invalidateChargingData(queryClient);
      setMessage({ text: count ? 'Rates saved. Charge costs update with them.' : 'No change to save.', error: false });
      onSaved?.();
    },
    onError: () => setMessage({ text: 'The rates did not save. Try again.', error: true }),
  });

  if (places.isLoading || !values) return <p className="r-note" role="status">Loading rates…</p>;
  if (places.isError) return <p className="r-form-error" role="alert">The rates did not load. Reload the page to try again.</p>;

  const parsed = { home: values.home === '' ? null : rParseRate(values.home), office: values.office === '' ? null : rParseRate(values.office) };
  const invalid = (kind: RRateKind) => values[kind] !== '' && parsed[kind] == null;
  const editable = (kind: RRateKind) => Boolean(targets[kind]) && !(targets[kind]?.charging?.billing_type === 'tou');
  const nothingToSave = !editable('home') && !editable('office');

  return (
    <form className="r-setform" noValidate onSubmit={(event) => {
      event.preventDefault();
      setMessage(null);
      if (invalid('home') || invalid('office')) return;
      if (nothingToSave) { onSaved?.(); return; }
      save.mutate(parsed);
    }}>
      {(['home', 'office'] as const).map(kind => {
        const place = targets[kind];
        const errorId = `rate-${kind}-error`;
        return (
          <label key={kind}>
            <span><i className="r-sw" style={{ background: SWATCH[kind] }} aria-hidden="true" />{LABEL[kind]} · $ per kWh</span>
            <input inputMode="decimal" name={kind} value={values[kind]} disabled={!editable(kind)} placeholder={place ? '' : 'No place yet'}
              aria-invalid={invalid(kind)} {...(invalid(kind) ? { 'aria-describedby': errorId } : {})}
              onChange={event => setValues(current => current && ({ ...current, [kind]: event.target.value }))} />
            {invalid(kind) && <span id={errorId} className="r-form-error">Enter a rate such as 0.19.</span>}
            {!place && <span className="r-trow-sub">No {LABEL[kind]} place is set up yet.</span>}
            {place?.charging?.billing_type === 'tou' && <span className="r-trow-sub">{place.name} uses a time-of-use plan. It stays as it is.</span>}
          </label>
        );
      })}
      <button type="submit" className={big ? 'r-btn r-btn-big' : 'r-btn'} disabled={save.isPending || invalid('home') || invalid('office')}>
        {save.isPending ? 'Saving…' : submitLabel}
      </button>
      {message && <p className={message.error ? 'r-form-error' : 'r-form-ok'} role={message.error ? 'alert' : 'status'}>{message.text}</p>}
    </form>
  );
}

export function RRatesCard() {
  return (
    <section id="rates" className="r-card r-pad r-rise" style={{ '--i': 3 } as React.CSSProperties} aria-labelledby="settings-rates-title">
      <div className="r-ctitle"><h2 id="settings-rates-title">Charging rates</h2><span className="r-muted">used for cost</span></div>
      <RRatesForm />
      <p className="r-note">{R_HOME_RATE_SOURCE} Office 0 means free. Other places are priced per charge.</p>
    </section>
  );
}
