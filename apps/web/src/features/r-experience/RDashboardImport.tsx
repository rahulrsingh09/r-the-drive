import React from 'react';
import { useNavigate } from '@tanstack/react-router';
import { importDashboardYaml, materializeUserDashboardDraft, useCreateDashboard } from '@panda/dashboards';
import { ResponsiveDialog } from '@panda/ui/primitives';
import { attemptDashboard, dashboardDestination, importState } from './dashboardModel';

export function RDashboardImport() {
  const input = React.useRef<HTMLInputElement>(null);
  const titleId = React.useId();
  const create = useCreateDashboard();
  const navigate = useNavigate();
  const [{ draft, error }, dispatch] = React.useReducer(importState, { draft: null, error: null });
  const read = async (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file) return;
    dispatch({ type: 'begin' });
    const result = await attemptDashboard(async () => importDashboardYaml(await file.text()));
    dispatch(result.ok ? { type: 'loaded', draft: result.value }
      : { type: 'failed', message: 'This file is not a valid dashboard YAML export.' });
  };
  const save = async () => {
    if (!draft) return;
    dispatch({ type: 'begin' });
    const result = await attemptDashboard(async () => {
      const saved = await create.mutateAsync(materializeUserDashboardDraft(draft));
      dispatch({ type: 'dismiss' });
      return navigate(dashboardDestination(saved));
    });
    if (!result.ok) dispatch({ type: 'failed', message: 'The dashboard could not be opened. Check the dashboard library before retrying.' });
  };
  const dismiss = () => dispatch({ type: 'dismiss' });
  const close = () => { if (!create.isPending) dismiss(); };
  const pick = () => input.current?.click();
  return <>
    <input ref={input} type="file" accept=".yaml,.yml" className="sr-only" aria-label="Import dashboard file" onChange={read} />
    <button type="button" onClick={pick}>Import dashboard</button>
    {error && !draft && <p role="alert">{error}</p>}
    {draft && <ResponsiveDialog titleId={titleId} onClose={close}>
      <div className="grid gap-5 p-6">
        <h2 id={titleId} className="text-xl">Import dashboard</h2>
        <p>{draft.name} · {draft.widgets.length} widgets</p>
        <p className="text-sm text-fg-secondary">Creates a personal copy. Existing dashboards stay unchanged.</p>
        {error && <p role="alert" className="text-sm text-status-danger">{error}</p>}
        <div className="flex gap-3">
          <button type="button" disabled={create.isPending} onClick={dismiss} className="min-h-11 rounded-xl border border-border px-4">Cancel</button>
          <button type="button" disabled={create.isPending} onClick={save} className="min-h-11 rounded-xl bg-fg px-4 text-bg">Import copy</button>
        </div>
      </div>
    </ResponsiveDialog>}
  </>;
}
