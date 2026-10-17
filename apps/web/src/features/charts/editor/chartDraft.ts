import { ChartDefinitionV1Schema, normalizeLegacyBundledChartRecord, validateBundledRendererDefinition, validateChartDefinitionAgainstSources } from '@panda/dashboards';
import type { ChartDefinitionV1, ChartRecord, ChartSourceManifest } from '@panda/types';

export type ChartProblem = { path: string; message: string };
export interface ChartDraftState {
  draft: ChartRecord | null;
  dirty: boolean;
  advancedText: string;
  advancedError: ChartProblem | null;
  saveError: string | null;
  sourceNotice: string | null;
}
export type ChartDraftEvent =
  | { type: 'load'; record: ChartRecord }
  | { type: 'record'; patch: Partial<ChartRecord> }
  | { type: 'definition'; update: (definition: ChartDefinitionV1) => ChartDefinitionV1 }
  | { type: 'json'; text: string }
  | { type: 'saved' }
  | { type: 'error'; message: string | null }
  | { type: 'notice'; message: string | null };

export function chartDraftInitial(draft: ChartRecord | null): ChartDraftState {
  return { draft, dirty: draft !== null, advancedText: draft ? JSON.stringify(draft.config, null, 2) : '', advancedError: null, saveError: null, sourceNotice: null };
}

export function chartDraftTransition(state: ChartDraftState, event: ChartDraftEvent): ChartDraftState {
  if (event.type === 'error') return { ...state, saveError: event.message };
  if (event.type === 'notice') return { ...state, sourceNotice: event.message };
  if (event.type === 'saved') return { ...state, dirty: false, saveError: null };
  if (event.type === 'load') {
    if (state.dirty) return state;
    const normalized = normalizeLegacyBundledChartRecord(event.record);
    const draft = { ...normalized, config: structuredClone(normalized.config) };
    return { ...state, draft, advancedText: JSON.stringify(draft.config, null, 2), advancedError: null, dirty: normalized !== event.record, sourceNotice: normalized !== event.record ? 'This legacy chart draft was upgraded to the current production structure. Review it, then save to keep the repair.' : state.sourceNotice };
  }
  if (event.type === 'json') {
    try {
      const parsed = ChartDefinitionV1Schema.safeParse(JSON.parse(event.text));
      if (!parsed.success) throw new Error(parsed.error.issues.map((issue) => `${issue.path.join('.') || 'config'}: ${issue.message}`).join('; '));
      const placements = state.draft?.config.placements ?? [];
      if (JSON.stringify(parsed.data.placements) !== JSON.stringify(placements)) {
        throw new Error('Dashboard assignments are fixed. Keep the existing placements while editing the chart definition.');
      }
      return { ...state, draft: state.draft ? { ...state.draft, config: parsed.data as ChartDefinitionV1 } : null, advancedText: JSON.stringify(parsed.data, null, 2), advancedError: null, dirty: true, saveError: null };
    } catch (error) {
      return { ...state, advancedText: event.text, dirty: true, advancedError: { path: 'advanced', message: error instanceof Error ? error.message : 'Invalid JSON.' } };
    }
  }
  const draft = state.draft && (event.type === 'record' ? { ...state.draft, ...event.patch } : { ...state.draft, config: event.update(state.draft.config) });
  return { ...state, draft, dirty: true, saveError: null, advancedText: draft ? JSON.stringify(draft.config, null, 2) : state.advancedText, advancedError: event.type === 'definition' ? null : state.advancedError };
}

export function chartDraftProblems(state: ChartDraftState, manifests: ChartSourceManifest[]): ChartProblem[] {
  if (!state.draft) return [{ path: 'chart', message: 'Chart draft is not available.' }];
  let problems: ChartProblem[];
  try {
    const { config, slug } = state.draft;
    const parsed = ChartDefinitionV1Schema.safeParse(config);
    problems = parsed.success
      ? [...validateChartDefinitionAgainstSources(config, manifests), ...validateBundledRendererDefinition(slug, config)]
      : parsed.error.issues.map((issue) => ({ path: issue.path.join('.'), message: issue.message }));
  } catch (error) {
    problems = [{ path: 'config', message: error instanceof Error ? error.message : 'Unable to save chart.' }];
  }
  return state.advancedError ? [state.advancedError, ...problems] : problems;
}
