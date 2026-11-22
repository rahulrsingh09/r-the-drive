import React from 'react';
import { render, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { RouteReplayMap, routePointAt } from '../../../../packages/ui/src/charts/RouteReplayMap';
import type { BasemapConfig } from '../../../../packages/ui/src/charts/TripMapChart';

function fakeMapLibre() {
  const handlers: Record<string, () => void> = {};
  const sources: Record<string, { setData: ReturnType<typeof vi.fn> }> = {};
  const marker = { setLngLat: vi.fn(() => marker), addTo: vi.fn(() => marker), remove: vi.fn() };
  const map = {
    options: null as unknown as Record<string, unknown>,
    on: vi.fn((event: string, cb: () => void) => { handlers[event] = cb; }),
    addControl: vi.fn(), addLayer: vi.fn(), resize: vi.fn(), remove: vi.fn(), setPaintProperty: vi.fn(),
    addSource: vi.fn((id: string) => { sources[id] = { setData: vi.fn() }; }),
    getSource: (id: string) => sources[id], getLayer: () => true,
    touchZoomRotate: { disableRotation: vi.fn() },
  };
  const lib = {
    Map: vi.fn(function MapCtor(options: Record<string, unknown>) { map.options = options; return map; }),
    NavigationControl: vi.fn(), Marker: vi.fn(function MarkerCtor() { return marker; }),
  };
  return { lib, map, marker, sources, load: () => handlers.load?.() };
}

const route: Array<[number, number]> = [[-122.2, 47.6], [-122.2, 47.61], [-122.2, 47.62]];

describe('RouteReplayMap', () => {
  it('draws the route with cooperative gestures and its own attribution, then follows the replay', async () => {
    const fake = fakeMapLibre();
    const styleLoader = vi.fn(async () => ({ version: 8, layers: [] }));
    const mapLoader = async () => fake.lib as never;
    const config = { enabled: true, revision: '1', styles: [{ id: 'liberty', label: 'Liberty', kind: 'style', light_url: '/v1/external/basemap/openfreemap/styles/liberty', dark_url: '', perspective_3d: false }] } as unknown as BasemapConfig;
    const { rerender, getByText } = render(
      <RouteReplayMap route={route} progress={2} mode="dark" basemapConfig={config} mapLoader={mapLoader} styleLoader={styleLoader} />,
    );
    await waitFor(() => expect(fake.lib.Map).toHaveBeenCalled());
    expect(styleLoader).toHaveBeenCalledWith('/v1/external/basemap/openfreemap/styles/liberty', 'dark', null);
    expect(fake.map.options).toMatchObject({ cooperativeGestures: true, attributionControl: false, dragRotate: false });
    expect(getByText('OpenFreeMap')).toHaveAttribute('href', 'https://openfreemap.org');
    fake.load();
    expect(fake.map.addLayer).toHaveBeenCalledWith(expect.objectContaining({ id: 'replay-route-line' }));
    expect(fake.marker.addTo).not.toHaveBeenCalled();
    rerender(<RouteReplayMap route={route} progress={1.5} mode="dark" basemapConfig={config} mapLoader={mapLoader} styleLoader={styleLoader} />);
    expect(fake.marker.addTo).toHaveBeenCalled();
    expect(fake.sources['replay-done']!.setData).toHaveBeenLastCalledWith(expect.objectContaining({
      geometry: expect.objectContaining({ type: 'LineString' }),
    }));
    const done = fake.sources['replay-done']!.setData.mock.lastCall![0].geometry.coordinates;
    expect(done).toHaveLength(3);
    expect(done[2][1]).toBeCloseTo(47.615);
  });

  it('shows a message when the map cannot load', async () => {
    const { findByText } = render(<RouteReplayMap route={route} mapLoader={async () => { throw new Error('no webgl'); }} />);
    expect(await findByText('The map could not load.')).toBeInTheDocument();
  });

  it('interpolates points along the route', () => {
    const [lng, lat] = routePointAt(route, 0.5);
    expect(lng).toBe(-122.2);
    expect(lat).toBeCloseTo(47.605);
  });
});
