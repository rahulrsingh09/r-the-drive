import React from 'react';

type DraftState<Form> = { form: Form; open: boolean; pending: Record<string, string | null> };
type DraftEvent<Form> =
  | { type: 'form'; update: React.SetStateAction<Form> }
  | { type: 'view'; open: boolean; form?: Form }
  | { type: 'pending'; operation: string; id: string | null };
export function useScheduleDraft<Form>(initial: Form) {
  const [state, send] = React.useReducer(
    (current: DraftState<Form>, event: DraftEvent<Form>): DraftState<Form> => {
      switch (event.type) {
        case 'form':
          return {
            ...current,
            form:
              typeof event.update === 'function'
                ? (event.update as (form: Form) => Form)(current.form)
                : event.update,
          };
        case 'view':
          return { ...current, open: event.open, form: event.form ?? current.form };
        case 'pending':
          return { ...current, pending: { ...current.pending, [event.operation]: event.id } };
      }
    },
    { form: initial, open: false, pending: {} }
  );
  const setForm = React.useCallback(
    (update: React.SetStateAction<Form>) => send({ type: 'form', update }),
    []
  );
  const show = React.useCallback(
    (open: boolean, form?: Form) =>
      send({ type: 'view', open, ...(form === undefined ? {} : { form }) }),
    []
  );
  async function during(operation: string, id: string, action: () => Promise<unknown>) {
    send({ type: 'pending', operation, id });
    try {
      await action();
    } finally {
      send({ type: 'pending', operation, id: null });
    }
  }
  return { ...state, setForm, show, during };
}
export function toggleScheduleDay(days: string[], day: string) {
  const present = days.includes(day);
  return present ? days.filter((candidate) => candidate !== day) : days.concat(day);
}
export function scheduleClock(minutes: number | null | undefined) {
  if (minutes == null) return '';
  return [Math.floor(minutes / 60) % 24, minutes % 60]
    .map((part) => String(part).padStart(2, '0'))
    .join(':');
}
export function parseScheduleClock(text: string, validate: boolean): number | null {
  const values = text.split(':').map(Number);
  if (validate && (values.length < 2 || values.some(Number.isNaN))) return null;
  return 60 * (values[0] ?? 0) + (values[1] ?? 0);
}
export function scheduleNumber(text: string, multiplier = 1, round = false) {
  if (text === '') return null;
  const value = Number.parseFloat(text) * multiplier;
  return Number.isNaN(value) ? null : round ? Math.round(value) : value;
}

export function scheduleFields<Form>(setForm: React.Dispatch<React.SetStateAction<Form>>) {
  function field<Key extends keyof Form>(key: Key, value: Form[Key]) {
    setForm((current) => ({ ...current, [key]: value }));
  }
  return { field };
}
