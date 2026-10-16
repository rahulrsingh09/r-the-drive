import { useCallback, useReducer, useState, type Dispatch, type SetStateAction } from 'react';
import { dashboardKey, type DashboardConfig } from '@panda/dashboards';
import { createDashboardEditSession, dashboardEditSessionReducer } from './dashboardEditSession';

export interface UseDashboardEditDraftArgs {
  savedConfig: DashboardConfig | undefined;
  slug: string;
  controlledEditMode?: boolean | undefined;
  onEditModeChange?: ((isEditMode: boolean) => void) | undefined;
}

export function useDashboardEditDraft({
  savedConfig,
  slug,
  controlledEditMode,
  onEditModeChange,
}: UseDashboardEditDraftArgs) {
  const [internalEditMode, setInternalEditMode] = useState(false);
  const isEditMode = controlledEditMode ?? internalEditMode;
  const key = dashboardKey(savedConfig, slug);
  const [session, dispatch] = useReducer(
    dashboardEditSessionReducer,
    { key, saved: savedConfig, editing: isEditMode },
    createDashboardEditSession,
  );

  // Reconcile before React commits children, so a new dashboard cannot commit
  // the previous dashboard's draft while waiting for an effect to run.
  // Equivalent fresh objects must not cause a render loop for inline callers.
  const savedChanged = session.input.saved !== savedConfig
    && JSON.stringify(session.input.saved) !== JSON.stringify(savedConfig);
  if (session.input.key !== key
    || session.input.editing !== isEditMode
    || savedChanged) {
    dispatch({ type: 'observe', input: { key, saved: savedConfig, editing: isEditMode } });
  }

  const setLocalConfig = useCallback<Dispatch<SetStateAction<DashboardConfig | null>>>(
    update => dispatch({ type: 'edit', update }),
    [],
  );
  const setSaveError = useCallback<Dispatch<SetStateAction<string | null>>>(
    update => dispatch({ type: 'error', update }),
    [],
  );

  const requestEditMode = useCallback((next: boolean) => {
    if (onEditModeChange) onEditModeChange(next);
    else setInternalEditMode(next);
  }, [onEditModeChange]);

  const enterEdit = useCallback(() => {
    dispatch({ type: 'enter' });
    requestEditMode(true);
  }, [requestEditMode]);

  const exitEdit = useCallback(() => {
    dispatch({ type: 'exit' });
    requestEditMode(false);
  }, [requestEditMode]);

  const localConfig = session.draft.config;
  return {
    activeConfig: localConfig ?? savedConfig,
    localConfig,
    setLocalConfig,
    isEditMode,
    isDirty: isEditMode && session.draft.kind === 'dirty',
    saveError: session.error,
    setSaveError,
    enterEdit,
    exitEdit,
  };
}
