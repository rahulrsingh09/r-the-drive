import { describe, expect, it } from 'vitest';
import { resolveTheme } from '@panda/themes';
import { getBundledChartDefinition, ChartDefinitionV1Schema } from '@panda/dashboards';
import type { AuthenticationSettingsUpdate, BackupOverview, ChartRecord, RestoreJob, ThemeResource } from '@panda/types';
import { authenticationDraft, authenticationFormTransition, type AuthenticationFormState } from '../components/settings/authenticationDraft';
import { backupCatalog, emptyRestoreWorkflow, restoreWorkflowTransition } from '../components/settings/restoreWorkflow';
import { buildNextSchedule, normalizeScheduleEdges, parseTimeToMinute, scheduleDraftToPeriods, validateScheduleDraft, placeEditorInitial, placeEditorTransition } from '../components/settings/placeEditor';
import { initialTelemetryExplorer, telemetryExplorerTransition } from '../components/settings/telemetryExplorerState';
import { chartDraftInitial, chartDraftTransition } from '../features/charts/editor/chartDraft';
import { themeDraftTransition } from '../features/settings/themeDraft';
import { auditThemeContrast } from '../features/settings/themeContrast';
import { chargeCorrectionResult } from '../features/charging/chargeCorrectionDraft';
import { accountEditorTransition, emptyAccountEditor } from '../components/users/accountEditor';
import { emptyInvitation, invitationDraftTransition } from '../components/users/invitationDraft';
import { searchPickerTransition } from '../features/charts/editor/useSearchPicker';
import { externalConnectionDraftTransition, initialExternalConnectionDraft } from '../components/settings/externalConnectionDraft';

function chart(): ChartRecord {
  const config = ChartDefinitionV1Schema.parse(getBundledChartDefinition('soc-history'));
  return { id: 'local-chart', slug: 'soc-history', name: 'Battery', ownerId: 'synthetic', isDefault: false, isLocked: false, isEnabled: true, config } as ChartRecord;
}

describe('feature draft contracts', () => {
  it('keeps authentication secret replacement and removal mutually exclusive', () => {
    const initial: AuthenticationFormState = { form: {}, secret: '', removeSecret: false, showAdvancedAccess: false, message: 'Old failure', messageRole: 'alert' };
    const removed = authenticationFormTransition(initial, { type: 'remove-secret', value: true });
    const blank = authenticationFormTransition(removed, { type: 'secret', value: ' ' });
    expect(blank.removeSecret).toBe(true);
    const replacement = authenticationFormTransition(blank, { type: 'secret', value: ' new secret ' });
    expect(replacement).toMatchObject({ secret: ' new secret ', removeSecret: false, message: '', messageRole: 'status' });
    expect(authenticationFormTransition(replacement, { type: 'saved' })).toMatchObject({ secret: '', removeSecret: false, message: 'Authentication settings saved.' });
  });

  it('clears all connection secrets and preview together when the server snapshot changes', () => {
    const initial = initialExternalConnectionDraft({ enabled: true, mode: 'remote' });
    const secret = externalConnectionDraftTransition(initial, { type: 'secret', key: 'apiKey', value: 'local-key' });
    const replacing = externalConnectionDraftTransition(secret, { type: 'replace' });
    const tested = externalConnectionDraftTransition(replacing, { type: 'tested', message: 'Verified', preview: 'data:image/png;base64,AA==' });
    expect(externalConnectionDraftTransition(tested, { type: 'load', draft: { enabled: false, mode: 'disabled' } })).toEqual({ ...initialExternalConnectionDraft({ enabled: false, mode: 'disabled' }), message: 'Verified' });
    expect(tested.apiKey).toBe('local-key');
  });

  it('does not reveal a prior invitation token in a reopened dialog', () => {
    const first = invitationDraftTransition(emptyInvitation, { type: 'open' });
    const closed = invitationDraftTransition(first, { type: 'close' });
    const next = invitationDraftTransition(closed, { type: 'open' });
    expect(invitationDraftTransition(next, { type: 'created', generation: first.generation, link: 'old-link' })).toBe(next);
    expect(invitationDraftTransition(closed, { type: 'created', generation: first.generation, link: 'old-link' })).toBe(closed);
    expect(invitationDraftTransition(next, { type: 'created', generation: next.generation, link: 'current-link' }).activationLink).toBe('current-link');
  });

  it('keeps a newer account editor open when a previous account save completes', () => {
    const loaded = accountEditorTransition(emptyAccountEditor, { type: 'load', user: { id: 'next', email: 'next@example.invalid', role: 'user' } });
    const editing = accountEditorTransition(loaded, { type: 'begin' });
    const changed = accountEditorTransition(editing, { type: 'edit', patch: { email: 'draft@example.invalid' } });
    expect(accountEditorTransition(changed, { type: 'saved', userId: 'previous' })).toBe(changed);
    expect(accountEditorTransition(changed, { type: 'load', user: loaded.source! })).toBe(changed);
    expect(accountEditorTransition(changed, { type: 'cancel' }).draft.email).toBe('next@example.invalid');
  });

  it('retains picker search on escape and clears it only after selection', () => {
    const searching = { query: 'battery', open: true };
    expect(searchPickerTransition(searching, { type: 'close' })).toEqual({ query: 'battery', open: false });
    expect(searchPickerTransition(searching, { type: 'chosen' })).toEqual({ query: '', open: false });
  });

  it('keeps the last valid chart when advanced JSON fails and clears the error on a structured edit', () => {
    const initial = chartDraftInitial(chart());
    const broken = chartDraftTransition(initial, { type: 'json', text: '{' });
    expect(broken.draft).toBe(initial.draft);
    expect(broken.advancedError?.path).toBe('advanced');
    expect(broken.advancedText).toBe('{');
    const edited = chartDraftTransition(broken, { type: 'definition', update: (value) => ({ ...value, timeframe: { mode: 'lifetime' } }) });
    expect(edited.advancedError).toBeNull();
    expect(JSON.parse(edited.advancedText).timeframe).toEqual({ mode: 'lifetime' });
    expect(initial.draft?.config.timeframe).not.toEqual({ mode: 'lifetime' });
  });

  it('does not replace unsaved chart changes with a refetch', () => {
    const state = chartDraftInitial(chart());
    expect(chartDraftTransition(state, { type: 'load', record: { ...chart(), name: 'Server' } })).toBe(state);
  });

  it('retains the saved theme revision across stale refetch and resets when the resource owner changes', () => {
    const resource: ThemeResource = { themeId: 'theme', name: 'Local', baseThemeId: 'classic', publishedRevision: null, publishedDefinition: null, retiredAt: null, etag: 'old', revisions: [{ revision: 1, definition: { theme: 'classic' }, definitionHash: 'synthetic', createdAt: '2026-10-17T00:00:00Z', publishedAt: null }] };
    const loaded = themeDraftTransition({ owner: '', draft: null, etag: '', revision: null }, { type: 'load', owner: 'user:a', resource });
    const saved = themeDraftTransition(loaded, { type: 'saved', etag: 'new', revision: 2 });
    expect(themeDraftTransition(saved, { type: 'load', owner: 'user:a', resource })).toBe(saved);
    const switched = themeDraftTransition(saved, { type: 'load', owner: 'user:b', resource });
    expect(switched.revision).toBe(1);
    expect(switched.etag).toBe('old');
    expect(switched.draft).not.toBe(resource.revisions[0]?.definition);
  });

  it('reports critical contrast and gives background warnings priority over adjacent series', () => {
    const theme = resolveTheme({ theme: 'classic' });
    theme.tokens.light['text-primary'] = theme.tokens.light['bg-page'];
    theme.series['series-01'] = { light: theme.tokens.light['bg-page'], dark: theme.tokens.dark['bg-page'] };
    const audit = auditThemeContrast(theme);
    expect(audit.primary).toContain('Primary text on page in light mode is below 4.5:1.');
    expect(audit.charts).toContain('series-01 has weak light-mode chart-background contrast.');
  });

  it('omits environment fields and preserves secret intent independently of other settings', () => {
    const fields: Array<keyof AuthenticationSettingsUpdate> = ['issuer_url', 'client_id'];
    const model = authenticationDraft({ issuer_url: { value: 'locked', source: 'environment' }, client_id: { value: 'before', source: 'database' } }, fields, {});
    expect(model.patch({ issuer_url: 'changed', client_id: 'after' }, ' secret ', false)).toEqual({ client_id: 'after', client_secret: ' secret ' });
    expect(model.patch(model.form, '', true)).toEqual({ client_secret: null });
    const incomplete = authenticationDraft({ issuer_url: { source: 'environment' } }, ['issuer_url'], { issuer_url: 'default' });
    expect(incomplete.patch({ issuer_url: 'changed' }, '', false)).toEqual({});
  });

  it('resets telemetry pagination and sample selection atomically with a filter change', () => {
    const state = { ...initialTelemetryExplorer, page: 4, selectedSampleTime: 'old' };
    expect(telemetryExplorerTransition(state, { key: 'search', value: 'tire' })).toEqual({ ...state, search: 'tire', page: 1, selectedSampleTime: null });
    expect(telemetryExplorerTransition(state, { key: 'collectorOpen', value: true }).page).toBe(4);
  });

  it('validates manual charge amounts without treating blank input as zero', () => {
    const draft = { locationMode: 'automatic' as const, costMode: 'manual' as const, placeId: '', manualCost: '' };
    expect(chargeCorrectionResult(draft).body).toBeNull();
    expect(chargeCorrectionResult({ ...draft, manualCost: '-1' }).body).toBeNull();
    expect(chargeCorrectionResult({ ...draft, manualCost: '0' }).body).toEqual({ location_mode: 'automatic', cost_mode: 'manual', cost_usd: 0 });
    expect(chargeCorrectionResult({ ...draft, manualCost: '1', locationMode: 'saved_place' }).locationError).toBe('Choose a saved place before saving.');
  });
});

describe('place schedule contracts', () => {
  it('allows midnight only as an end and preserves contiguous full-day periods', () => {
    expect(parseTimeToMinute('24:00', false)).toBeNull();
    expect(parseTimeToMinute('24:00', true)).toBe(1440);
    expect(parseTimeToMinute('9:05', false)).toBe(545);
    expect(parseTimeToMinute('09:5', false)).toBeNull();
    const schedule = [{ label: 'All day', start: '00:00', end: '24:00', rate: '0.13' }];
    expect(buildNextSchedule(schedule)).toEqual([{ label: 'All day', start: '00:00', end: '12:00', rate: '0.13' }, { label: 'Period 2', start: '12:00', end: '24:00', rate: '0.13' }]);
    expect(validateScheduleDraft(buildNextSchedule(schedule))).toBeNull();
    expect(schedule[0]?.end).toBe('24:00');
  });

  it('preserves validation precedence and does not normalize gaps while parsing', () => {
    const schedule = [{ label: '', start: '01:00', end: '02:00', rate: '-1' }];
    expect(validateScheduleDraft(schedule)).toBe('Each TOU period needs a label.');
    expect(scheduleDraftToPeriods(schedule)?.[0]?.start_minute).toBe(60);
    expect(normalizeScheduleEdges(schedule)[0]).toEqual({ ...schedule[0], start: '00:00', end: '24:00' });
    expect(validateScheduleDraft([{ ...schedule[0]!, label: 'Night', start: '00:00', end: '24:00' }])).toBe('Each TOU period needs a non-negative rate.');
  });

  it('clears the complete place editor in one reset transition', () => {
    const state = placeEditorInitial('imperial');
    const changed = placeEditorTransition(state, { type: 'query', value: 'pending address' });
    const reset = placeEditorTransition(changed, { type: 'reset', unit: 'metric' });
    expect(reset.addressQuery).toBe('');
    expect(reset.selectedAddress).toBeNull();
    expect(reset.draft.radius_m).toBe('75');
  });
});

describe('backup workflow contracts', () => {
  it('ignores a poll from a different restore job', () => {
    const job: RestoreJob = { id: 'new', artifact_id: 'archive', phase: 'queued', progress_percent: 0, message: '', error_message: null, created_at: '', updated_at: '', plan: null, validation_report: null, retryable: false, rollback_state: 'not_required' };
    const state = restoreWorkflowTransition(emptyRestoreWorkflow, { type: 'start', job, token: 'local-only', size: 1024 });
    expect(restoreWorkflowTransition(state, { type: 'poll', id: 'old', job: { ...job, id: 'old' } })).toBe(state);
    expect(restoreWorkflowTransition(state, { type: 'dismiss' }).activeRestore).toBeNull();
  });

  it('keeps original artifact order for all and falls back when a selected source disappears', () => {
    const local = { id: 'local', storage_type: 'local' }, remote = { id: 'remote', storage_type: 's3' };
    const overview = { settings: { endpoint: '', bucket: '' }, artifacts: [remote, local] } as BackupOverview;
    expect(backupCatalog(overview, 'all').visibleArtifacts).toEqual([remote, local]);
    expect(backupCatalog({ ...overview, artifacts: [local] } as BackupOverview, 's3').selectedCatalogSource).toBe('local');
  });
});
