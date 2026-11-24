import React from 'react';
import { Link } from '@tanstack/react-router';
import { Info } from 'lucide-react';
import type { Place } from '@panda/types';
import { AppLink, BarChart, Card, Chart, CountUp, DataTable, tip } from '../chartKit';
import { CategoryLegend, Kpi, Seg, useStoredValue } from '../controls';
import { ACTIVITY_PATH, activityPlaceSearch, type InsightsData } from '../InsightsContext';
import {
  CATEGORIES, CATEGORY_COLOR, CATEGORY_LABEL, DEFAULT_GAS, byCategory, gasCost, inWeek, monthDay, n0, n1, placeGroups, sum, usd,
  weeksFrom, type Category, type GasSettings,
} from '../insightsModel';
import { rateDetail, ratePlace, usePlaceTag } from '../placeTags';
import { placeTagConfirm } from '../../../lib/placeTagCopy';

const isGas = (value: unknown): value is GasSettings =>
  !!value && typeof value === 'object' && Number.isFinite((value as GasSettings).mpg) && Number.isFinite((value as GasSettings).price)
  && (value as GasSettings).mpg > 0 && (value as GasSettings).price >= 0;

const TAG_OPTIONS = CATEGORIES.map(c => [c, CATEGORY_LABEL[c]] as const);

function rateText(place: Place | null) {
  const profile = place?.charging;
  if (!profile) return 'Not set';
  if (profile.billing_type === 'free' || (profile.billing_type !== 'tou' && profile.rate === 0)) return 'Free';
  if (profile.billing_type === 'tou') return 'Time of use';
  if (profile.billing_type === 'per_minute') return `$${profile.rate.toFixed(2)}/min`;
  return <>${profile.rate.toFixed(4)}<small className="ri-muted">/kWh</small></>;
}

export function CostTab({ data }: { data: InsightsData }) {
  const { charges, trips, first, today, places, vehicleId, userId } = data;
  const priced = charges.filter(c => c.cost != null);
  const unpriced = charges.filter(c => c.cost == null);
  const total = sum(priced, c => c.cost);
  const miles = sum(trips, t => t.miles);
  const weeks = weeksFrom(first, today);
  const by = byCategory(charges);
  const allKwh = sum(by, b => b.kwh);
  const groups = placeGroups(charges);
  const [gas, setGas] = useStoredValue(userId, 'gas', DEFAULT_GAS, isGas);
  const [draft, setDraft] = React.useState({ mpg: String(gas.mpg), price: String(gas.price) });
  React.useEffect(() => setDraft({ mpg: String(gas.mpg), price: String(gas.price) }), [gas]);
  const gasTotal = gasCost(miles, gas);
  const tag = usePlaceTag(vehicleId);
  const [confirm, setConfirm] = React.useState<{ key: string; category: Category } | null>(null);
  const latestUnpriced = unpriced[unpriced.length - 1];

  const weekCost = (week: string, category?: Category) =>
    sum(charges.filter(c => inWeek(c.day, week) && (!category || c.category === category)), c => c.cost);

  const submitGas = (event: React.FormEvent) => {
    event.preventDefault();
    const next = { mpg: Number(draft.mpg), price: Number(draft.price) };
    if (isGas(next)) setGas(next);
  };

  return (
    <>
      <section className="ri-card ri-pad ri-rise ri-wide" aria-label="Spent on charging">
        <Kpi label={`Spent on charging since ${monthDay(first)}`} aside={<>{n0(miles)} mi driven<br />
          <b className="ri-num ri-t1">{miles ? `${n1(total / miles * 100)}¢` : '–'}</b> per mile</>}>
          <CountUp value={total} decimals={2} prefix="$" />
        </Kpi>
        {latestUnpriced && (
          <Link className="ri-callout" to="/charging/$sessionId" params={{ sessionId: latestUnpriced.id }}>
            <Info aria-hidden="true" />
            <span><b>{unpriced.length} {unpriced.length === 1 ? 'charge has' : 'charges have'} no cost yet.</b>{' '}
              The total and per-mile cost leave {unpriced.length === 1 ? 'it' : 'them'} out. Add what you paid.</span>
          </Link>
        )}
      </section>

      <Card title="Where you charge" sub="by energy" index={1} link={{ to: ACTIVITY_PATH, label: 'See every charge', search: { filter: 'charges' } }}>
        <div className="ri-stack" role="img" aria-label={by.filter(b => b.kwh).map(b => `${CATEGORY_LABEL[b.category]} ${n0(b.kwh)} kWh`).join(', ') || 'No charges yet'}>
          {by.filter(b => b.kwh).map(b => <div key={b.category} style={{ width: `${b.kwh / allKwh * 100}%`, background: CATEGORY_COLOR[b.category] }}
            data-tip={tip(CATEGORY_LABEL[b.category], `${n0(b.kwh)} kWh`)} />)}
        </div>
        <div className="ri-rows3">
          {by.map(b => (
            <div key={b.category} className="ri-rowx4">
              <span><i className="ri-sw" style={{ background: CATEGORY_COLOR[b.category] }} />{CATEGORY_LABEL[b.category]}</span>
              <span className="ri-num ri-muted">{n0(b.kwh)} kWh</span>
              <span className="ri-num ri-muted">{b.perKwh != null ? `$${b.perKwh.toFixed(3)}/kWh` : '–'}</span>
              <b className="ri-num">{b.unpriced ? `${b.unpriced} not set` : b.kwh && b.cost === 0 ? 'Free' : usd(b.cost)}</b>
            </div>
          ))}
        </div>
      </Card>

      <Card title="Cost per week" sub="by place" index={2}>
        <CategoryLegend />
        <Chart title="Cost per week" render={(w, big) => (
          <BarChart width={w} big={big} label="Charging cost per week by place" bars={weeks.map(week => {
            const value = weekCost(week);
            return {
              label: monthDay(week), top: value ? usd(value) : '$0', zero: !value,
              segs: CATEGORIES.map(c => ({ v: weekCost(week, c), color: CATEGORY_COLOR[c], tip: tip(`Week of ${monthDay(week)}`, `${CATEGORY_LABEL[c]}: ${usd(weekCost(week, c))}`) })),
            };
          })} />
        )} table={() => <DataTable caption="Cost per week" head={['Week', 'Home', 'Office', 'Other', 'Total']}
          rows={weeks.map(week => [monthDay(week), ...CATEGORIES.map(c => usd(weekCost(week, c))), usd(weekCost(week))])} />} />
      </Card>

      <Card title="Places" sub="by place" wide index={3}>
        {groups.length ? (
          <div className="ri-places">
            {groups.map(group => {
              const singleCharge = group.charges.length === 1;
              return (
                <div key={group.key} className="ri-place">
                  <AppLink className="ri-pl-name" href={`${ACTIVITY_PATH}?${new URLSearchParams(activityPlaceSearch(group.name))}`}>
                    <b>{group.name}</b>
                    <span className="ri-muted">{group.charges.length} {singleCharge ? 'charge' : 'charges'} · {n0(group.kwh)} kWh · {group.unpriced ? 'cost not set' : group.cost === 0 ? 'free' : usd(group.cost)}</span>
                  </AppLink>
                  {singleCharge ? (
                    <Seg small label={`Tag ${group.name}`} value={confirm?.key === group.key ? confirm.category : group.category} options={TAG_OPTIONS}
                      disabled={tag.pending != null} onChange={category => setConfirm(category === group.category ? null : { key: group.key, category })} />
                  ) : (
                    <span className="ri-muted">{CATEGORY_LABEL[group.category]} · per charge</span>
                  )}
                  {singleCharge && confirm?.key === group.key ? (() => {
                    const text = placeTagConfirm(group.name, group.charges.length, confirm.category, places);
                    return (
                      <div className="ri-confirm" role="group" aria-label={text.title}>
                        <p><b>{text.title}</b> {text.body}</p>
                        <div className="ri-confirm-actions">
                          <button type="button" className="ri-btn" onClick={() => { setConfirm(null); void tag.apply(group, confirm.category); }}>{text.action}</button>
                          <button type="button" className="ri-btn ri-ghost" onClick={() => setConfirm(null)}>Cancel</button>
                        </div>
                      </div>
                    );
                  })() : null}
                </div>
              );
            })}
          </div>
        ) : <p className="ri-empty">No charges yet.</p>}
        {tag.message && <p className={`ri-note${tag.message.error ? ' ri-error' : ''}`} role="status">{tag.message.text}</p>}
        <p className="ri-note">Home and Office are auto-selected when you charge there and use the rates in Settings. Open any charge to change its tag or cost individually without affecting previous charges.</p>
      </Card>

      <Card title="Compared with gas" sub="estimate" index={4}>
        <div className="ri-kpi">
          <div><div className="ri-big ri-big-sm">{usd(Math.max(0, gasTotal - total))}</div><div className="ri-label">saved on {n0(miles)} mi</div></div>
          <div className="ri-label ri-kpi-aside">Gas would cost<br /><b className="ri-num ri-t1">{usd(gasTotal)}</b></div>
        </div>
        <form className="ri-gasform" onSubmit={submitGas} aria-label="Gas car to compare">
          <label>Car <input inputMode="decimal" name="mpg" value={draft.mpg} onChange={e => setDraft(d => ({ ...d, mpg: e.target.value }))} aria-label="Miles per gallon" /> mpg</label>
          <label>Gas <input inputMode="decimal" name="price" value={draft.price} onChange={e => setDraft(d => ({ ...d, price: e.target.value }))} aria-label="Gas price in dollars per gallon" /> $/gal</label>
          <button className="ri-btn ri-ghost" type="submit">Update</button>
        </form>
        <p className="ri-note">Unpriced charges count as $0 here. Change the car and gas price to match what you compare against.</p>
      </Card>

      <Card title="Your rates" index={5} link={{ to: '/settings', label: 'Change rates in Settings' }}>
        <div className="ri-rows3">
          {(['home', 'office'] as const).map(category => {
            const place = ratePlace(places, category);
            return (
              <div key={category} className="ri-rate">
                <i className="ri-sw" style={{ background: CATEGORY_COLOR[category] }} />
                <div><b>{CATEGORY_LABEL[category]}</b><div className="ri-muted">{rateDetail(place, charges)}</div></div>
                <b className="ri-num">{rateText(place)}</b>
              </div>
            );
          })}
          <div className="ri-rate">
            <i className="ri-sw" style={{ background: CATEGORY_COLOR.other }} />
            <div><b>Other</b><div className="ri-muted">DC fast and other places</div></div>
            <b>Per charge</b>
          </div>
        </div>
      </Card>
    </>
  );
}
