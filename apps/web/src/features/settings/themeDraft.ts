import type { ThemeColorPair, ThemeOverride } from '@panda/themes';
import type { ThemeResource } from '@panda/types';

export interface ThemeDraftState {
  owner: string;
  draft: ThemeOverride | null;
  etag: string;
  revision: number | null;
}
export type ThemeDraftEvent =
  | { type: 'load'; owner: string; resource: ThemeResource }
  | { type: 'paint'; kind: 'token' | 'series'; key: string; value: ThemeColorPair }
  | { type: 'saved'; etag: string; revision?: number };

export function themeDraftTransition(state: ThemeDraftState, event: ThemeDraftEvent): ThemeDraftState {
  switch (event.type) {
    case 'load': {
      if (state.owner === event.owner && state.draft) return state;
      const revision = event.resource.revisions[0];
      return { owner: event.owner, draft: structuredClone(revision?.definition ?? { theme: event.resource.baseThemeId }), etag: event.resource.etag, revision: revision?.revision ?? null };
    }
    case 'paint': {
      if (!state.draft) return state;
      const collection = event.kind === 'token' ? 'tokens' : 'series';
      return { ...state, draft: { ...state.draft, [collection]: { ...state.draft[collection], [event.key]: event.value } } };
    }
    case 'saved': return { ...state, etag: event.etag, revision: event.revision || state.revision };
  }
}
