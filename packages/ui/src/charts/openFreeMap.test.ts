import { describe, expect, it, vi } from 'vitest';
import { basemapRequest, darkLiberty, DARK_LIBERTY_COLORS, libertyStyleUrl, loadLibertyStyle } from './openFreeMap';

const liberty = {
  version: 8,
  layers: [
    { id: 'background', type: 'background', paint: { 'background-color': '#fff' } },
    { id: 'water', type: 'fill', paint: { 'fill-color': '#00f', 'fill-pattern': 'x' } },
    { id: 'highway_motorway', type: 'line', paint: { 'line-color': '#f00' } },
    { id: 'label_city', type: 'symbol', paint: {} },
  ],
};

describe('OpenFreeMap Liberty', () => {
  it('recolors Liberty into the dark theme without changing the source', () => {
    const dark = darkLiberty(liberty);
    expect(dark.layers[0]!.paint!['background-color']).toBe(DARK_LIBERTY_COLORS.bg);
    expect(dark.layers[1]!.paint).toEqual({ 'fill-color': DARK_LIBERTY_COLORS.water, 'fill-outline-color': DARK_LIBERTY_COLORS.water });
    expect(dark.layers[2]!.paint!['line-color']).toBe(DARK_LIBERTY_COLORS.mway);
    expect(dark.layers[3]!.paint).toMatchObject({ 'text-color': DARK_LIBERTY_COLORS.city, 'text-halo-color': DARK_LIBERTY_COLORS.halo });
    expect(liberty.layers[0]!.paint['background-color']).toBe('#fff');
  });

  it('uses the proxied Liberty style from the basemap config', () => {
    const config = { enabled: true, revision: '1', styles: [{ id: 'liberty', label: 'Liberty', kind: 'style', light_url: '/v1/external/basemap/openfreemap/styles/liberty?v=1', dark_url: '', perspective_3d: false }] };
    expect(libertyStyleUrl(config as never)).toBe('/v1/external/basemap/openfreemap/styles/liberty?v=1');
    expect(libertyStyleUrl({ ...config, enabled: false } as never)).toBeNull();
  });

  it('sends proxy requests to this origin with the token and leaves other URLs alone', () => {
    expect(basemapRequest('https://panda.invalid/v1/external/basemap/openfreemap/tiles/1.pbf', 't', 'https://app.test')).toEqual({
      url: 'https://app.test/v1/external/basemap/openfreemap/tiles/1.pbf', headers: { Authorization: 'Bearer t' }, credentials: 'same-origin',
    });
    expect(basemapRequest('https://elsewhere.test/a.png', 't', 'https://app.test')).toEqual({ url: 'https://elsewhere.test/a.png' });
  });

  it('loads the dark style once and recolors it', async () => {
    const fetcher = vi.fn(async () => new Response(JSON.stringify(liberty)));
    const first = await loadLibertyStyle('/v1/external/basemap/openfreemap/styles/liberty?v=test', 'dark', 't', fetcher as never);
    await loadLibertyStyle('/v1/external/basemap/openfreemap/styles/liberty?v=test', 'dark', 't', fetcher as never);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(first.layers[0]!.paint!['background-color']).toBe(DARK_LIBERTY_COLORS.bg);
  });
});
