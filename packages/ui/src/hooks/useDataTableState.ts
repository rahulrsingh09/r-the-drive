import * as React from 'react';
import type { SortingState, VisibilityState, Updater } from '@tanstack/react-table';
import { listen } from '../lib/browserRuntime';
type MenuPosition = { x: number; y: number } | null;
type State = { sorting: SortingState; columnVisibility: VisibilityState; menuPosition: MenuPosition };
type Change = { type: 'sort'; update: Updater<SortingState> } | { type: 'columns'; update: Updater<VisibilityState> } | { type: 'menu'; position: MenuPosition };
function transition(state: State, action: Change): State {
  if (action.type === 'menu') return { ...state, menuPosition: action.position };
  if (action.type === 'sort') return { ...state, sorting: typeof action.update === 'function' ? action.update(state.sorting) : action.update };
  return { ...state, columnVisibility: typeof action.update === 'function' ? action.update(state.columnVisibility) : action.update };
}
export function useDataTableState(hidden: string[] | undefined, menuEnabled: boolean) {
  const [state, send] = React.useReducer(transition, hidden, (ids): State => ({ sorting: [], columnVisibility: Object.fromEntries((ids ?? []).map(id => [id, false])), menuPosition: null }));
  React.useEffect(() => {
    if (!menuEnabled || !state.menuPosition) return;
    const dismiss: EventListener = event => {
      if (event.type !== 'keydown' || (event as KeyboardEvent).key === 'Escape') send({ type: 'menu', position: null });
    };
    return listen(['click', 'contextmenu', 'keydown'].map(name => [document, name, dismiss] as const));
  }, [menuEnabled, state.menuPosition]);
  const setSorting = React.useCallback((update: Updater<SortingState>) => send({ type: 'sort', update }), []);
  const setColumnVisibility = React.useCallback((update: Updater<VisibilityState>) => send({ type: 'columns', update }), []);
  function openColumnMenu(event: React.MouseEvent) {
    if (!menuEnabled) return;
    event.preventDefault(); event.stopPropagation();
    send({ type: 'menu', position: { x: event.clientX, y: event.clientY } });
  }
  function chooseSort(id: string) {
    setSorting(current => id ? [{ id, desc: current[0]?.desc ?? false }] : []);
  }
  function reverseSort() {
    setSorting(current => current.map(({ id, desc }) => ({ id, desc: !desc })));
  }
  return { ...state, setSorting, setColumnVisibility, openColumnMenu, chooseSort, reverseSort };
}
