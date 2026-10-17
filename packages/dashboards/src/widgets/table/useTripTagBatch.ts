import React from 'react';
import { useUpdateTripTagAssignments } from '@panda/hooks';
import { clearTripSelection } from './tripSelectionStore';

type Action = 'clear' | 'add' | 'save';
type BatchState = { tags: string[]; pending: Action | null };
type Event =
  | { type: 'stage'; tags: string[] }
  | { type: 'start'; action: Action }
  | { type: 'settle'; success: boolean };
export function useTripTagBatch(vehicleId: string | null, selectedIds: string[]) {
  const updateAssignments = useUpdateTripTagAssignments(vehicleId);
  const [state, dispatch] = React.useReducer(
    (current: BatchState, event: Event): BatchState => {
      switch (event.type) {
        case 'stage':
          return { ...current, tags: event.tags };
        case 'start':
          return { ...current, pending: event.action };
        case 'settle':
          return { tags: event.success ? [] : current.tags, pending: null };
      }
    },
    { tags: [], pending: null }
  );
  const stage = React.useCallback((tags: string[]) => dispatch({ type: 'stage', tags }), []);
  async function run(action: Action, mode: 'add' | 'replace', tags: string[]) {
    const command = { trip_ids: selectedIds, tag_ids: tags, mode };
    dispatch({ type: 'start', action });
    const success = await updateAssignments.mutateAsync(command).then(
      () => true,
      () => false
    );
    dispatch({ type: 'settle', success });
    if (success) clearTripSelection();
  }
  return { tags: state.tags, pending: state.pending, stage, run, updateAssignments };
}
