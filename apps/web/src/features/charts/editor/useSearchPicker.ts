import { useMemo, useReducer } from 'react';

interface SearchChoice {
  value: string;
  label: string;
  description?: string;
  disabled?: boolean;
}

type PickerEvent = { type: 'toggle' | 'close' | 'chosen' } | { type: 'search'; query: string };
export function searchPickerTransition(state: { query: string; open: boolean }, event: PickerEvent) {
  switch (event.type) {
    case 'search': return { ...state, query: event.query };
    case 'toggle': return { ...state, open: !state.open };
    case 'close': return { ...state, open: false };
    case 'chosen': return { query: '', open: false };
  }
}

export function useSearchPicker<T extends SearchChoice>(options: T[], value: string, onChange: (value: string) => void) {
  const [state, dispatch] = useReducer(searchPickerTransition, { query: '', open: false });
  const index = useMemo(() => ({
    byValue: new Map(options.slice().reverse().map((option) => [option.value, option])),
    entries: options.map((option) => ({ option, text: `${option.label} ${option.description ?? ''}`.toLowerCase() })),
  }), [options]);
  const query = state.query.toLowerCase();
  return {
    ...state,
    selected: index.byValue.get(value),
    filtered: index.entries.filter((entry) => entry.text.includes(query)).map((entry) => entry.option),
    dispatch,
    choose: (option: T) => {
      if (option.disabled) return;
      onChange(option.value);
      dispatch({ type: 'chosen' });
    },
  };
}
