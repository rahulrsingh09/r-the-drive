import React from 'react';
import { useParkedEnergy, usePhantomDrainPeriods, useStateTimeline } from '@panda/hooks';
import { getUnitPreferences } from '@panda/ui/lib/utils';
import { BarChart, Card, Chart, CountUp, DataTable, Legend, LineChart, RangeBars, tip } from '../chartKit';
import { Kpi, Spec } from '../controls';
import type { InsightsData } from '../InsightsContext';
import { batteryHealthPct, usableCapacityNow } from '../../../lib/batteryHealth';
import {
  addDays, batteryBand, dayKeyOf, dayRange, monthDay, n0, n1, n2, sum, weekdayMonthDay, type CapacityDay,
} from '../insightsModel';
import {
  buildParkedDaySummaries,
  buildParkedOnlyTimeline,
  formatDurationMinutes,
  formatParkedRateCopy,
  formatTempReading,
  pickDefaultParkedDay,
  SPIKE_LOSS_PCT,
  summarizeParkedEnergy,
} from '../parkedDiagnostics';

/** Parked periods shorter than this are left out. */
const MIN_PARK_HOURS = 3;

export function BatteryTab({ data, capacity }: { data: InsightsData; capacity: CapacityDay[] }) {
  const { charges, trips, first, today, status, vehicleId, timezone } = data;
  const from30 = addDays(today, -29) > first ? addDays(today, -29) : first;
  const span = React.useMemo(() => ({
    fromIso: new Date(new Date(`${from30}T00:00:00Z`).getTime() - 864e5).toISOString(),
    toIso: new Date().toISOString(),
  }), [from30]);

  const parks = usePhantomDrainPeriods(vehicleId, span.fromIso, span.toIso, 250, MIN_PARK_HOURS);
  const parkedEnergy = useParkedEnergy(vehicleId, span.fromIso, span.toIso);
  const days = dayRange(first, today);
  const band = batteryBand(days, trips, charges);
  const limit = status?.battery_limit ?? null;
  const kwh = sum(charges, c => c.kwh);

  const capNow = usableCapacityNow(capacity);
  const cap0 = capacity[0]?.kwh ?? null;
  const capValues = capacity.map(c => c.kwh);
  const cmin = capValues.length ? Math.min(...capValues) : 0;
  const cmax = capValues.length ? Math.max(...capValues) : 0;
  const health = batteryHealthPct(capacity);
  const lost = capNow != null && cap0 != null ? cap0 - capNow : 0;
  const level = status?.battery_level ?? null;
  const range = status?.range_miles ?? null;
  const full = level && range != null && level > 0 ? range / (level / 100) : null;

  const summaries = React.useMemo(
    () => buildParkedDaySummaries(parks.data?.periods ?? [], timezone),
    [parks.data?.periods, timezone],
  );
  const days30 = dayRange(from30, today);
  const drainDays = days30.filter(d => summaries.has(d));
  const parkedHours = sum(drainDays, d => summaries.get(d)!.hours);
  const parkedLoss = sum(drainDays, d => summaries.get(d)!.loss);
  const perDay = parkedHours ? parkedLoss / (parkedHours / 24) : null;
  const drainMax = Math.max(1, ...drainDays.map(d => summaries.get(d)!.loss));

  const defaultDay = React.useMemo(() => pickDefaultParkedDay(days30, summaries), [days30, summaries]);
  const [pickedDay, setPickedDay] = React.useState<string | null>(null);
  const activeDay = pickedDay && summaries.has(pickedDay) ? pickedDay : defaultDay;
  const selectedSummary = activeDay ? summaries.get(activeDay) ?? null : null;
  const selectedIndex = activeDay ? days30.indexOf(activeDay) : -1;

  const spikeWindow = React.useMemo(() => {
    if (!selectedSummary?.isSpike || !selectedSummary.periods.length) return null;
    const starts = selectedSummary.periods.map(p => Date.parse(p.period_start!)).filter(Number.isFinite);
    const ends = selectedSummary.periods.map(p => (p.period_end ? Date.parse(p.period_end) : Date.now())).filter(Number.isFinite);
    if (!starts.length || !ends.length) return null;
    // Look back 48h before period_start because state_timeline filters started_at >= from
    // and sleep/offline intervals commonly start prior to a multi-day or overnight window.
    return {
      fromIso: new Date(Math.min(...starts) - 48 * 3_600_000).toISOString(),
      toIso: new Date(Math.max(...ends)).toISOString(),
    };
  }, [selectedSummary]);

  const timelineQuery = useStateTimeline(
    spikeWindow ? vehicleId : null,
    spikeWindow?.fromIso ?? null,
    spikeWindow?.toIso ?? null,
    500,
  );

  const fahrenheit = getUnitPreferences().temperature_unit !== 'celsius';
  const parkedTimeline = React.useMemo(
    () => (selectedSummary?.isSpike
      ? buildParkedOnlyTimeline(selectedSummary, timelineQuery.data?.periods ?? [])
      : null),
    [selectedSummary, timelineQuery.data?.periods],
  );
  const energyBreakdown = React.useMemo(
    () => summarizeParkedEnergy(parkedEnergy.data?.samples ?? [], timezone),
    [parkedEnergy.data?.samples, timezone],
  );

  return (
    <>
      <section className="ri-card ri-pad ri-rise ri-wide" aria-label="Battery health">
        <Kpi label="Battery health" aside={health == null ? 'Not reported' : <>{lost > 0.05 ? `${n1(lost)} kWh below` : 'Same as'} the first day<br />
          {lost > 0.05 && cap0 ? `${n1(lost / cap0 * 100)}% degradation` : 'No degradation'}</>}>
          {health == null ? <span className="ri-muted">–</span> : <><CountUp value={health} decimals={1} /><small>%</small></>}
        </Kpi>
        {health != null && <div className="ri-hbar" role="presentation"><div className="ri-hbar-fill" style={{ width: `${Math.min(100, health)}%` }} /></div>}
        <Spec className="ri-spec-in" items={[
          { label: 'Usable capacity now', value: capNow != null ? `${n1(capNow)} kWh` : 'Not reported' },
          { label: 'On the first day', value: cap0 != null ? `${n1(cap0)} kWh` : 'Not reported' },
          { label: 'Range at 100%', value: full != null ? `${n0(full)} mi` : 'Not reported', note: full != null ? `car estimate, from ${n0(range!)} mi at ${n0(level!)}%` : undefined },
          { label: 'Full charge cycles', value: capNow ? n1(kwh / capNow) : '–' },
          { label: 'Charges', value: charges.length },
          { label: 'Energy added', value: `${n0(kwh)} kWh` },
        ]} />
        <p className="ri-note">
          Health compares today’s usable capacity with the first day of data ({monthDay(capacity[0]?.day ?? first)}). The factory figure is not in the data.
          {capValues.length > 1 && ` Day-to-day readings move between ${n1(cmin)} and ${n1(cmax)} kWh, so a change under about 1 kWh is noise, not wear.`}
        </p>
      </section>

      <Card title="Usable capacity" sub="the car reports, by day" index={1}>
        {capacity.length ? <Chart title="Usable capacity" render={(w, big) => (
          <LineChart width={w} big={big} height={130} label="Usable capacity the car reports, by day" color="var(--ri-home)"
            points={capacity.map((c, i) => ({ x: i, y: c.kwh, tip: tip(`${n1(c.kwh)} kWh`, monthDay(c.day)) }))}
            yDomain={[Math.floor(cmin) - 1, Math.ceil(cmax) + 1]} ticks={[Math.floor(cmin) - 1, Math.ceil(cmax) + 1]} tickFormat={v => `${v} kWh`}
            endLabels={[[0, n1(capacity[0]!.kwh), 'start'], [-1, n1(capacity[capacity.length - 1]!.kwh), 'end']]}
            xLabels={[[0, monthDay(capacity[0]!.day), 'start'], [capacity.length - 1, 'Today', 'end']]} />
        )} table={() => <DataTable caption="Usable capacity by day" head={['Day', 'Usable capacity']}
          rows={[...capacity].reverse().map(c => [weekdayMonthDay(c.day), `${n1(c.kwh)} kWh`])} />} />
          : <p className="ri-empty">The car has not reported its usable capacity yet.</p>}
      </Card>

      <Card title="Battery level each day" sub="lowest to highest" index={2}>
        <Legend items={[{ label: 'Daily range', color: 'var(--ri-acc)' }, ...(limit != null ? [{ label: `Limit ${n0(limit)}%`, color: 'var(--ri-t2)', line: true, dashed: true }] : [])]} />
        {band.length ? <Chart title="Battery level each day" render={(w, big) => (
          <RangeBars width={w} big={big} height={160} label="Daily battery range" count={days.length} limit={limit}
            startLabel={monthDay(days[0]!)} endLabel="Today"
            bars={band.map(b => ({ index: b.index, lo: b.lo, hi: b.hi, color: 'var(--ri-acc)', tip: tip(weekdayMonthDay(b.day), `${n0(b.lo)}% – ${n0(b.hi)}%`) }))} />
        )} table={() => <DataTable caption="Battery level each day" head={['Day', 'Lowest', 'Highest']}
          rows={[...band].reverse().map(b => [weekdayMonthDay(b.day), `${n0(b.lo)}%`, `${n0(b.hi)}%`])} />} />
          : <p className="ri-empty">No battery readings yet.</p>}
      </Card>

      <section className="ri-card ri-pad ri-rise ri-wide" style={{ '--i': 3 } as React.CSSProperties} aria-label="Parked loss, last 30 days">
        <Kpi label="Parked loss · last 30 days" aside={<>{n0(parkedHours / 24)} days parked<br />{n1(parkedLoss)}% lost in total</>}>
          {perDay == null ? <span className="ri-muted">–</span> : <><CountUp value={perDay} decimals={2} /><small>% / day</small></>}
        </Kpi>
        <div className="ri-gap">
          <Legend items={[
            { label: `Normal (< ${SPIKE_LOSS_PCT}%)`, color: 'var(--ri-acc)' },
            { label: `Spike (≥ ${SPIKE_LOSS_PCT}%)`, color: 'var(--ri-office)' },
          ]} />
          <Chart title="Parked loss, last 30 days" render={(w, big) => (
            <BarChart
              width={w}
              big={big}
              height={124}
              label="Battery lost while parked, per day, last 30 days"
              max={drainMax}
              selectedIndex={selectedIndex >= 0 ? selectedIndex : null}
              onSelect={(index) => {
                const day = days30[index];
                if (day && summaries.has(day)) setPickedDay(day);
              }}
              bars={days30.map((d, i) => {
                const day = summaries.get(d);
                return {
                  label: i === 0 ? monthDay(d) : i === days30.length - 1 ? 'Today' : '',
                  ariaLabel: day
                    ? `${weekdayMonthDay(d)}, ${n1(day.loss)} percent lost over ${n0(day.hours)} hours parked`
                    : `${weekdayMonthDay(d)}, no long park`,
                  segs: [{
                    v: day?.loss ?? 0,
                    color: day?.isSpike ? 'var(--ri-office)' : 'var(--ri-acc)',
                    tip: tip(weekdayMonthDay(d), day ? `${n1(day.loss)}% over ${n0(day.hours)} h parked` : 'No long park'),
                  }],
                };
              })}
            />
          )} table={() => <DataTable caption="Parked loss per day" head={['Day', 'Parked', 'Lost', 'Per day']}
            rows={[...drainDays].reverse().map(d => { const day = summaries.get(d)!; return [weekdayMonthDay(d), `${n0(day.hours)} h`, `${n1(day.loss)}%`, `${n2(day.loss / (day.hours / 24))}%`]; })} />} />
        </div>

        {selectedSummary && (
          <div className="ri-park-detail" data-parked-detail={selectedSummary.day}>
            <div className="ri-park-sel-h">
              <div className="ri-park-sel-title">
                <b>{weekdayMonthDay(selectedSummary.day)}</b>
                <span className={`ri-park-badge ${selectedSummary.isSpike ? 'spike' : 'calm'}`}>
                  {selectedSummary.isSpike ? 'High drain day' : 'Healthy sleep'}
                </span>
              </div>
              <span className="ri-muted ri-small ri-num">{formatParkedRateCopy(selectedSummary)}</span>
            </div>

            {!selectedSummary.isSpike ? (
              <p className="ri-park-calm" data-parked-normal>
                Normal overnight sleep — {n1(selectedSummary.loss)}% over {n0(selectedSummary.hours)} h parked
                {selectedSummary.sleepPct != null && selectedSummary.sleepPct > 0 ? ` · ${selectedSummary.sleepPct}% deep sleep` : ''}.
              </p>
            ) : (
              <div className="ri-park-spike" data-parked-spike>
                {parkedTimeline && parkedTimeline.totalMinutes > 0 && (
                  <div className="ri-park-tl" aria-label="Parked sleep versus awake timeline">
                    <div className="ri-park-tl-h">
                      <span className="ri-small">Parked sleep vs awake</span>
                      <span className="ri-muted ri-small ri-num">
                        {parkedTimeline.sleepPct != null ? `${parkedTimeline.sleepPct}% deep sleep` : 'Sleep states not reported'}
                      </span>
                    </div>
                    <div
                      className="ri-stack"
                      role="img"
                      aria-label={`Deep sleep ${formatDurationMinutes(parkedTimeline.sleepMinutes)}, Kept awake ${formatDurationMinutes(parkedTimeline.awakeMinutes)}`}
                    >
                      {parkedTimeline.sleepMinutes > 0 && (
                        <div
                          style={{
                            width: `${(parkedTimeline.sleepMinutes / parkedTimeline.totalMinutes) * 100}%`,
                            background: 'var(--ri-home)',
                          }}
                          data-tip={tip('Deep sleep', formatDurationMinutes(parkedTimeline.sleepMinutes))}
                        />
                      )}
                      {parkedTimeline.awakeMinutes > 0 && (
                        <div
                          style={{
                            width: `${(parkedTimeline.awakeMinutes / parkedTimeline.totalMinutes) * 100}%`,
                            background: 'var(--ri-office)',
                          }}
                          data-tip={tip(
                            'Kept awake',
                            formatDurationMinutes(parkedTimeline.awakeMinutes),
                            ...(parkedTimeline.awakeTriggers.length ? [`Triggers: ${parkedTimeline.awakeTriggers.join(', ')}`] : []),
                          )}
                        />
                      )}
                      {parkedTimeline.unreportedMinutes > 1 && (
                        <div
                          style={{
                            width: `${(parkedTimeline.unreportedMinutes / parkedTimeline.totalMinutes) * 100}%`,
                            background: 'var(--ri-s3)',
                          }}
                          data-tip={tip('Unreported state', formatDurationMinutes(parkedTimeline.unreportedMinutes))}
                        />
                      )}
                    </div>
                    <div className="ri-park-tl-rows">
                      <span className="ri-small">
                        <i className="ri-sw" style={{ background: 'var(--ri-home)' }} />
                        Deep sleep · {formatDurationMinutes(parkedTimeline.sleepMinutes)}
                      </span>
                      <span className="ri-small">
                        <i className="ri-sw" style={{ background: 'var(--ri-office)' }} />
                        Kept awake · {formatDurationMinutes(parkedTimeline.awakeMinutes)}
                        {parkedTimeline.awakeTriggers.length > 0 ? ` (${parkedTimeline.awakeTriggers.join(', ')})` : ''}
                      </span>
                      {parkedTimeline.unreportedMinutes >= 15 && (
                        <span className="ri-small">
                          <i className="ri-sw" style={{ background: 'var(--ri-s3)' }} />
                          Unreported · {formatDurationMinutes(parkedTimeline.unreportedMinutes)}
                        </span>
                      )}
                    </div>
                  </div>
                )}

                <Spec
                  className="ri-spec-in"
                  items={[
                    {
                      label: 'SoC & burn rate',
                      value: selectedSummary.socStart != null && selectedSummary.socEnd != null
                        ? `${n0(selectedSummary.socStart)}% → ${n0(selectedSummary.socEnd)}%`
                        : `${n1(selectedSummary.loss)}% lost`,
                      note: selectedSummary.periodCount > 1 && selectedSummary.primaryLoss != null
                        ? `Largest of ${selectedSummary.periodCount} parks (${n1(selectedSummary.primaryLoss)}% of ${n1(selectedSummary.loss)}% total)`
                        : selectedSummary.drainPctPerHour != null
                          ? `${n1(selectedSummary.loss)}% · ${n2(selectedSummary.drainPctPerHour)}%/h`
                          : undefined,
                    },
                    {
                      label: 'Outside / cabin temp',
                      value: selectedSummary.avgOutsideTempC != null
                        ? `${formatTempReading(selectedSummary.avgOutsideTempC, fahrenheit)}${selectedSummary.avgCabinTempC != null ? ` / ${formatTempReading(selectedSummary.avgCabinTempC, fahrenheit)}` : ''}`
                        : 'Not reported',
                      note: selectedSummary.avgOutsideTempC != null && selectedSummary.avgOutsideTempC < 0
                        ? 'Below freezing · extra thermal draw'
                        : undefined,
                    },
                    {
                      label: 'Sleep & wakes',
                      value: `${selectedSummary.sleepPct != null && selectedSummary.sleepPct > 0 ? `${selectedSummary.sleepPct}%` : parkedTimeline?.sleepPct != null ? `${parkedTimeline.sleepPct}%` : '–'} · ${selectedSummary.wakeCount != null ? `${selectedSummary.wakeCount} ${selectedSummary.wakeCount === 1 ? 'wake' : 'wakes'}` : '–'}`,
                      note: selectedSummary.maxSleepStreakMinutes != null
                        ? `Longest sleep ${formatDurationMinutes(selectedSummary.maxSleepStreakMinutes)}`
                        : parkedTimeline && parkedTimeline.awakeMinutes > 0 && parkedTimeline.sleepMinutes === 0
                          ? 'Stayed awake (no deep sleep)'
                          : parkedTimeline?.sleepPct == null
                            ? 'Sleep telemetry unavailable'
                            : undefined,
                    },
                    {
                      label: 'Active triggers',
                      value: parkedTimeline?.awakeTriggers.length ? parkedTimeline.awakeTriggers.join(', ') : 'None detected',
                      note: parkedTimeline?.awakeTriggers.length ? 'Detected while awake' : 'No OTA, climate, or Gear Guard flagged',
                    },
                    {
                      label: 'Closures & locks',
                      value: selectedSummary.openClosureDetected === true
                        ? 'Open closure detected'
                        : selectedSummary.openClosureDetected === false
                          ? 'All closed'
                          : 'Not reported',
                      note: selectedSummary.doorsUnlocked === true
                        ? 'Doors unlocked during park'
                        : selectedSummary.doorsUnlocked === false
                          ? 'Doors stayed locked'
                          : undefined,
                    },
                    {
                      label: '12V & plug',
                      value: selectedSummary.pluggedIn === true
                        ? 'Plugged in'
                        : selectedSummary.pluggedIn === false
                          ? 'Unplugged'
                          : 'Plug not reported',
                      note: selectedSummary.twelveVoltAttention === true
                        ? '12V health needs attention'
                        : selectedSummary.twelveVoltAttention === false
                          ? '12V battery healthy'
                          : undefined,
                    },
                  ]}
                />
              </div>
            )}
          </div>
        )}

        <p className="ri-note">
          Battery lost while parked for {MIN_PARK_HOURS} hours or more, grouped by the day parking started. Tap any bar to inspect that day.{perDay != null && perDay < 0.5 ? ' Overall loss is low.' : ''}
        </p>
      </section>

      <Card
        title="Where parked energy went"
        sub={energyBreakdown.totalKwh > 0 && energyBreakdown.windowLabel
          ? `Latest ${energyBreakdown.windowLabel} report · ${n1(energyBreakdown.totalKwh)} kWh total`
          : 'Latest Rivian report'}
        wide
        index={4}
      >
        {energyBreakdown.categories.length ? (
          <>
            <div
              className="ri-stack"
              role="img"
              aria-label={energyBreakdown.categories.map(c => `${c.label} ${n1(c.kwh)} kWh`).join(', ')}
            >
              {energyBreakdown.categories.map(cat => (
                <div
                  key={cat.key}
                  style={{ width: `${cat.sharePct}%`, background: cat.color }}
                  data-tip={tip(cat.label, `${n1(cat.kwh)} kWh (${n0(cat.sharePct)}%)`)}
                />
              ))}
            </div>
            <div className="ri-rows3">
              {energyBreakdown.categories.map(cat => (
                <div key={cat.key} className="ri-rowx4">
                  <span><i className="ri-sw" style={{ background: cat.color }} />{cat.label}</span>
                  <span className="ri-num ri-muted">{n0(cat.sharePct)}%</span>
                  <span className="ri-num ri-muted">of snapshot</span>
                  <b className="ri-num">{n1(cat.kwh)} kWh</b>
                </div>
              ))}
            </div>
            <p className="ri-note">
              Rivian’s latest rolling parked-energy report
              {energyBreakdown.durationMinutes != null ? ` (${formatDurationMinutes(energyBreakdown.durationMinutes)}` : ''}
              {energyBreakdown.sourceAt ? `${energyBreakdown.durationMinutes != null ? ' · ' : ' ('}${monthDay(dayKeyOf(energyBreakdown.sourceAt, timezone))})` : energyBreakdown.durationMinutes != null ? ')' : ''}
              {energyBreakdown.rangeImpactMi != null ? ` · ~${n1(energyBreakdown.rangeImpactMi)} mi range impact` : ''}. Rolling reports show the most recent window rather than a 30-day cumulative total.
            </p>
          </>
        ) : (
          <p className="ri-empty">Rivian has not reported parked energy breakdowns yet.</p>
        )}
      </Card>
    </>
  );
}
