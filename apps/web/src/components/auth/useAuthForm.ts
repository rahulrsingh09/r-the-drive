import { useCallback, useReducer } from 'react';

type FormState = { password: string; error: string; pending: { password: boolean; sso: boolean } };
type FormAction =
  | { type: 'password'; value: string }
  | { type: 'error'; value: string }
  | { type: 'start' | 'finish'; method: 'password' | 'sso'; separate: boolean };

function authFormReducer(state: FormState, action: FormAction): FormState {
  switch (action.type) {
    case 'password': return { ...state, password: action.value };
    case 'error': return { ...state, error: action.value };
    case 'start':
    case 'finish': return {
      ...state,
      error: action.type === 'start' ? '' : state.error,
      pending: {
        ...(action.separate ? state.pending : { password: false, sso: false }),
        [action.method]: action.type === 'start',
      },
    };
  }
}

export function useAuthForm(separateMethods = false) {
  const [state, dispatch] = useReducer(authFormReducer, { password: '', error: '', pending: { password: false, sso: false } });
  const setPassword = useCallback((value: string) => dispatch({ type: 'password', value }), []);
  const setError = useCallback((value: string) => dispatch({ type: 'error', value }), []);
  const run = useCallback(async (
    method: 'password' | 'sso',
    action: () => Promise<void>,
    failure: (cause: unknown) => string,
    retainPending = false,
  ) => {
    dispatch({ type: 'start', method, separate: separateMethods });
    let succeeded = false;
    try {
      await action();
      succeeded = true;
    } catch (cause) {
      dispatch({ type: 'error', value: failure(cause) });
    } finally {
      if (!retainPending || !succeeded) dispatch({ type: 'finish', method, separate: separateMethods });
    }
  }, [separateMethods]);
  return { password: state.password, error: state.error, loading: state.pending.password, ssoLoading: state.pending.sso, setPassword, setError, run };
}
