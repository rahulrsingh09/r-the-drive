import { createWidgetStore } from '../widgetStore';

type TableState = { page: number; pageSize: number; search: string; contextKey: string };
type TableEvent = { type: 'reset'; context: string; force?: boolean | undefined } | { type: 'page'; value: number } | { type: 'pageSize'; value: number } | { type: 'search'; value: string };
const table = createWidgetStore<TableState, TableEvent>(
  { page: 1, pageSize: 15, search: '', contextKey: '' },
  (state, event) => {
    if (event.type === 'reset') return !event.force && state.contextKey === event.context ? state : { page: 1, pageSize: 15, search: '', contextKey: event.context };
    if (state[event.type] === event.value) return state;
    return { ...state, page: 1, [event.type]: event.value };
  },
);
export function resetTripTableState(contextKey: string, options?: { force?: boolean }) { table.send({ type: 'reset', context: contextKey, force: options?.force }); }
export function setTripTablePage(page: number) { table.send({ type: 'page', value: page }); }
export function setTripTablePageSize(pageSize: number) { table.send({ type: 'pageSize', value: pageSize }); }
export function setTripTableSearch(search: string) { table.send({ type: 'search', value: search }); }
export function useTripTableState() {
  const { page, pageSize, search } = table.useSnapshot();
  return { page, pageSize, search };
}
