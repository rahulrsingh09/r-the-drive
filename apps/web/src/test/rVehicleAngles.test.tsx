import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const artwork = vi.hoisted(() => ({ failedFetch: new Set<string>() }));

vi.mock('@panda/hooks', () => ({
  useVehicleArtwork: (source: string | null | undefined) => ({
    src: source && !artwork.failedFetch.has(source) ? source : null,
    restoring: false,
    isLoading: false,
    isError: Boolean(source && artwork.failedFetch.has(source)),
  }),
  AuthenticatedVehicleArtwork: ({ source, fallbackSource, fallbackProps: _fallbackProps, onFallbackChange: _onFallbackChange, ...props }: React.ImgHTMLAttributes<HTMLImageElement> & {
    source?: string | null; fallbackSource?: string | null; fallbackProps?: unknown; onFallbackChange?: unknown;
  }) => {
    const src = source && !artwork.failedFetch.has(source) ? source : fallbackSource;
    return src ? <img {...props} src={src} /> : null;
  },
}));

import { RVehicleAngles } from '../features/r-experience/RVehicleOrbit';

const angle = (name: string, label: string) => ({ angle: name, label, url: `/v1/vehicle-image-cache/${name}.png` });
const angles = [
  angle('front', 'Front'),
  angle('front-three-quarter', 'Front three-quarter'),
  angle('side', 'Side'),
  angle('rear', 'Rear'),
] as unknown as Parameters<typeof RVehicleAngles>[0]['angles'];
const label = 'Your R2, Launch Green. Use arrow keys to turn.';

function stops(container: HTMLElement) {
  return [...container.querySelectorAll('img.r-orbit-angle')].map((image) => image.getAttribute('data-angle'));
}

describe('RVehicleAngles failed owner images', () => {
  beforeEach(() => {
    artwork.failedFetch = new Set();
    vi.spyOn(window, 'matchMedia').mockReturnValue({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() } as unknown as MediaQueryList);
  });

  it('drops an angle whose protected image cannot be fetched from the turn set', () => {
    artwork.failedFetch = new Set(['/v1/vehicle-image-cache/front.png']);
    const { container } = render(<RVehicleAngles label={label} angles={angles} fallbackSource="/fallback/side.webp" />);

    expect(stops(container)).toEqual(['front-three-quarter', 'side', 'rear']);
    const slider = screen.getByRole('slider', { name: label });
    expect(slider).toHaveAttribute('aria-valuemax', '2');
    expect(slider).toHaveAttribute('aria-valuetext', 'Side');
    fireEvent.keyDown(slider, { key: 'ArrowRight' });
    fireEvent.keyDown(slider, { key: 'ArrowRight' });
    // The turn wraps from Rear to Front three-quarter: Front is not a stop.
    expect(slider).toHaveAttribute('aria-valuetext', 'Front three-quarter');
  });

  it('drops an angle whose image fails in the browser', () => {
    const { container } = render(<RVehicleAngles label={label} angles={angles} fallbackSource="/fallback/side.webp" />);
    expect(stops(container)).toEqual(['front', 'front-three-quarter', 'side', 'rear']);

    fireEvent.error(container.querySelector('img[data-angle="rear"]')!);
    expect(stops(container)).toEqual(['front', 'front-three-quarter', 'side']);
    expect(screen.getByRole('slider')).toHaveAttribute('aria-valuemax', '2');
  });

  it('shows the side image alone, with no turn control, when fewer than 2 angles load', () => {
    artwork.failedFetch = new Set([
      '/v1/vehicle-image-cache/front.png',
      '/v1/vehicle-image-cache/front-three-quarter.png',
      '/v1/vehicle-image-cache/rear.png',
    ]);
    const { container } = render(<RVehicleAngles label={label} angles={angles} fallbackSource="/fallback/side.webp" />);

    expect(screen.queryByRole('slider')).toBeNull();
    expect(stops(container)).toEqual(['side']);
    expect(screen.getByRole('img', { name: 'Your R2, Launch Green' })).toHaveAttribute('src', '/v1/vehicle-image-cache/side.png');
  });

  it('keeps the side stop with the packaged fallback when the side image fails', () => {
    artwork.failedFetch = new Set(['/v1/vehicle-image-cache/side.png']);
    const { container } = render(<RVehicleAngles label={label} angles={angles} fallbackSource="/fallback/side.webp" />);

    expect(stops(container)).toEqual(['front', 'front-three-quarter', 'side', 'rear']);
    expect(container.querySelector('img[data-angle="side"]')).toHaveAttribute('src', '/fallback/side.webp');
  });
});
