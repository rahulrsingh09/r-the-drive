interface TripTagDraft {
  editing: boolean;
  tagIds: string[];
  error: string;
}
export const emptyTripTagDraft: TripTagDraft = { editing: false, tagIds: [], error: '' };
type TagEvent =
  | { type: 'toggle'; tagIds: string[] }
  | { type: 'tags'; tagIds: string[] }
  | { type: 'saving' | 'saved' | 'failed' | 'close' };
export function tripTagDraftTransition(state: TripTagDraft, event: TagEvent): TripTagDraft {
  switch (event.type) {
    case 'toggle': return { editing: !state.editing, tagIds: state.editing ? state.tagIds : event.tagIds, error: '' };
    case 'tags': return { ...state, tagIds: event.tagIds };
    case 'saving': return { ...state, error: '' };
    case 'saved': return { ...state, editing: false };
    case 'failed': return { ...state, error: 'Couldn’t update trip tags. Try again.' };
    case 'close': return { ...state, editing: false, error: '' };
  }
}
