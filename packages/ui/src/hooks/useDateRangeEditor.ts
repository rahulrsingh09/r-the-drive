import * as React from 'react';
import { startOfMonth, parse, isValid } from 'date-fns';
import type { DashboardTimeframe, DateRange } from '@panda/types';
import { appDatePartsToDate, formatAppDate, getAppDateParts } from '../lib/dateTime';
import { timeframeRange } from '../lib/dateRange';

type Endpoint = 'from' | 'to';
type DraftDate = { date: Date; input: string };
type State = { from: DraftDate; to: DraftDate; month: Date; target: Endpoint; expanded: boolean; error: string | null };
type Action =
  | { type: 'reset'; timeframe: DashboardTimeframe }
  | { type: 'input'; target: Endpoint; value: string }
  | { type: 'time'; target: Endpoint; patch: Partial<{ hour: number; minute: number }> }
  | { type: 'target'; target: Endpoint }
  | { type: 'day'; date: Date }
  | { type: 'month'; date: Date }
  | { type: 'expand' }
  | { type: 'preset' }
  | { type: 'invalid' };
const PARSE_PATTERNS = [
  'M/d/yy', 'M/d/yyyy', 'M/d/yy h:mm a', 'M/d/yyyy h:mm a', 'M/d/yy HH:mm', 'M/d/yyyy HH:mm',
  'MM/dd/yy', 'MM/dd/yyyy', 'MM/dd/yy h:mm a', 'MM/dd/yyyy h:mm a', 'MM/dd/yy HH:mm', 'MM/dd/yyyy HH:mm',
];
function draftDate(date: Date): DraftDate {
  return { date, input: formatAppDate(date, { year: '2-digit', month: 'numeric', day: 'numeric' }) };
}
function initial(timeframe: DashboardTimeframe): State {
  const range = timeframeRange(timeframe) ?? { from: new Date(), to: new Date() };
  return { from: draftDate(range.from), to: draftDate(range.to), month: startOfMonth(range.from), target: 'from', expanded: timeframe.kind === 'custom', error: null };
}
function parseDateInput(value: string, fallback: Date): Date | null {
  const text = value.trim();
  if (!text) return null;
  const attempts = PARSE_PATTERNS.values();
  for (const pattern of attempts) {
    const date = parse(text, pattern, fallback);
    if (!isValid(date)) continue;
    return appDatePartsToDate({ year: date.getFullYear(), month: date.getMonth() + 1, day: date.getDate(), hour: date.getHours(), minute: date.getMinutes(), second: 0 });
  }
  const native = new Date(text);
  return isValid(native) ? native : null;
}
function transition(state: State, action: Action): State {
  switch (action.type) {
    case 'reset': {
      const next = initial(action.timeframe);
      return { ...next, target: state.target, expanded: next.expanded || state.expanded };
    }
    case 'month': return { ...state, month: action.date };
    case 'target': return { ...state, target: action.target, month: startOfMonth(state[action.target].date) };
    case 'expand': return { ...state, expanded: !state.expanded, ...(!state.expanded ? { target: 'from' as const, month: startOfMonth(state.from.date) } : {}) };
    case 'preset': return { ...state, expanded: false };
    case 'invalid': return { ...state, error: 'Enter a valid From and To date, like 1/7/25 6:30 PM.' };
    case 'input': {
      const parsed = parseDateInput(action.value, state[action.target].date);
      const updated = { ...state[action.target], input: action.value, ...(parsed ? { date: parsed } : {}) };
      return { ...state, [action.target]: updated, ...(parsed ? { month: startOfMonth(parsed), error: null } : {}) };
    }
    case 'time': {
      const previous = state[action.target].date;
      const parts = getAppDateParts(previous);
      const date = parts ? appDatePartsToDate({ ...parts, ...action.patch }) : previous;
      return { ...state, [action.target]: draftDate(date) };
    }
    case 'day': {
      const parts = getAppDateParts(state[state.target].date);
      const date = appDatePartsToDate({ hour: parts?.hour ?? 0, minute: parts?.minute ?? 0, second: parts?.second ?? 0, year: action.date.getFullYear(), month: action.date.getMonth() + 1, day: action.date.getDate() });
      return { ...state, [state.target]: draftDate(date), month: startOfMonth(date), error: null };
    }
  }
}
export function useDateRangeEditor(timeframe: DashboardTimeframe) {
  const [state, send] = React.useReducer(transition, timeframe, initial);
  React.useEffect(() => { send({ type: 'reset', timeframe }); }, [timeframe]);
  function range(): DateRange | null {
    const from = parseDateInput(state.from.input, state.from.date);
    const to = parseDateInput(state.to.input, state.to.date);
    if (!from || !to) { send({ type: 'invalid' }); return null; }
    return from <= to ? { from, to } : { from: to, to: from };
  }
  return { state, send, range };
}
