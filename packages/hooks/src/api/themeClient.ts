import type { ThemeCatalogResponse, ThemeMutationResponse, ThemePalette, ThemePreferencesResponse, ThemePreferencesV2, ThemeResource } from '@panda/types';
import { transport } from './transport';

type ThemeRequest = { method: string; path: string; body?: unknown; etag?: string; preferenceEtag?: string | undefined; matchRequired?: boolean };

async function themeResponse(request: ThemeRequest) {
  const headers: Record<string, string> = {};
  if (request.etag || request.matchRequired) headers['If-Match'] = request.etag ?? '';
  if (request.preferenceEtag) headers['X-Theme-Preferences-If-Match'] = request.preferenceEtag;
  if (!Object.keys(headers).length && request.body === undefined && !Object.hasOwn(request, 'etag')) return transport.requestResponse(request.method, request.path);
  return transport.requestResponse(request.method, request.path, request.body, undefined, true, true, Object.keys(headers).length ? headers : undefined);
}

async function resource<T>(request: ThemeRequest): Promise<T> {
  const response = await themeResponse(request);
  return response.json() as Promise<T>;
}

async function preferences(request: ThemeRequest): Promise<ThemePreferencesResponse> {
  const response = await themeResponse(request);
  const preferences = await response.json() as ThemePreferencesResponse['preferences'];
  return { preferences, etag: response.headers.get('etag') ?? '' };
}

export const themeClient = {
  getPreferences: () => preferences({ method: 'GET', path: '/v2/auth/preferences/theme' }),
  updatePreferences: (body: ThemePreferencesV2, etag: string) => preferences({ method: 'PUT', path: '/v2/auth/preferences/theme', body, etag }),
  getCatalog: () => resource<ThemeCatalogResponse>({ method: 'GET', path: '/v2/themes/catalog' }),
  create: (name: string, baseThemeId: ThemePalette) => resource<ThemeMutationResponse>({ method: 'POST', path: '/v2/themes', body: { name, baseThemeId } }),
  get: (themeId: string) => resource<ThemeResource>({ method: 'GET', path: `/v2/themes/${themeId}` }),
  saveRevision: (themeId: string, definition: unknown, etag: string) => resource<ThemeMutationResponse>({ method: 'POST', path: `/v2/themes/${themeId}/revisions`, body: { definition }, etag }),
  publishRevision: (themeId: string, revision: number, apply: boolean, etag: string, preferenceEtag?: string) => resource<ThemeMutationResponse>({ method: 'POST', path: `/v2/themes/${themeId}/revisions/${revision}/publish`, body: { apply }, etag, preferenceEtag, matchRequired: true }),
  rollback: (themeId: string, revision: number, etag: string, preferenceEtag: string) => preferences({ method: 'POST', path: `/v2/themes/${themeId}/rollback`, body: { revision }, etag, preferenceEtag, matchRequired: true }),
  retire: (themeId: string, etag: string) => resource<ThemeMutationResponse>({ method: 'DELETE', path: `/v2/themes/${themeId}`, etag }),
};
