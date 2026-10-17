interface ChargeView {
  editing: boolean;
  sourceDetails: boolean;
  saveMessage: string | null;
}
export const initialChargeView: ChargeView = { editing: false, sourceDetails: false, saveMessage: null };
type ChargeViewEvent = 'edit' | 'source' | 'saving' | 'saved';
export function chargeViewTransition(state: ChargeView, event: ChargeViewEvent): ChargeView {
  switch (event) {
    case 'edit': return { ...state, editing: !state.editing };
    case 'source': return { ...state, sourceDetails: !state.sourceDetails };
    case 'saving': return state.saveMessage === null ? state : { ...state, saveMessage: null };
    case 'saved': return state.saveMessage === 'Corrections saved.' ? state : { ...state, saveMessage: 'Corrections saved.' };
  }
}
