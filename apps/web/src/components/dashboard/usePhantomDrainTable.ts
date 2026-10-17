import { useDeferredValue, useMemo, useReducer } from 'react';
import type { PhantomDrainPeriod } from '@panda/types';

type TableState = { search: string; page: number; pageSize: number; scope: string };
type TableAction =
  | { type: 'search'; value: string }
  | { type: 'size'; value: number }
  | { type: 'page'; value: number }
  | { type: 'scope'; value: string; page: number };

function tableReducer(state: TableState, action: TableAction): TableState {
  switch (action.type) {
    case 'search': return { ...state, search: action.value };
    case 'size': return { ...state, pageSize: action.value };
    case 'page': return { ...state, page: action.value };
    case 'scope': return { ...state, scope: action.value, page: action.page };
  }
}

export function usePhantomDrainTable(
  periods: PhantomDrainPeriod[],
  sourceKey: string,
  searchText: (period: PhantomDrainPeriod) => string,
) {
  const [state, dispatch] = useReducer(tableReducer, { search: '', page: 1, pageSize: 15, scope: '' });
  const deferred = useDeferredValue(state.search);
  const filteredPeriods = useMemo(() => {
    const query = deferred.trim().toLowerCase();
    if (query.length === 0) return periods;
    const index = periods.map(period => ({ period, text: searchText(period) }));
    return index.flatMap(entry => entry.text.includes(query) ? [entry.period] : []);
  }, [periods, searchText, deferred]);
  const scope = JSON.stringify([sourceKey, state.pageSize, deferred]);
  const totalPages = Math.max(1, Math.ceil(filteredPeriods.length / state.pageSize));
  const page = scope === state.scope ? Math.min(state.page, totalPages) : 1;
  if (state.scope !== scope || state.page !== page) dispatch({ type: 'scope', value: scope, page });
  const offset = (page - 1) * state.pageSize;
  return {
    search: state.search,
    setSearch: (value: string) => dispatch({ type: 'search', value }),
    pageSize: state.pageSize,
    setPageSize: (value: number) => dispatch({ type: 'size', value }),
    page, totalPages, filteredPeriods,
    pagedPeriods: filteredPeriods.slice(offset, offset + state.pageSize),
    previousPage: () => dispatch({ type: 'page', value: Math.max(1, page - 1) }),
    nextPage: () => dispatch({ type: 'page', value: Math.min(totalPages, page + 1) }),
  };
}
