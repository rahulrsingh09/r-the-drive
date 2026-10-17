import React from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { snapshotDom } from './fullReplacementRExperienceDom';
const loader = vi.hoisted(() => ({ get: vi.fn(), close: vi.fn() }));
vi.mock('../features/r-experience/orbitFrames', async importOriginal => ({
  ...await importOriginal<typeof import('../features/r-experience/orbitFrames')>(),
  createOrbitFrames: () => loader,
}));
import { RVehicleOrbit } from '../features/r-experience/RVehicleOrbit';

let observer: IntersectionObserverCallback;
let reduced = false;
let clock = 1000;
let nextAnimation = 0;
let animations: Map<number, FrameRequestCallback>;
beforeEach(() => {
  clock = 1000;
  animations = new Map();
  loader.get.mockReset().mockResolvedValue({});
  loader.close.mockClear();
  vi.spyOn(performance, 'now').mockImplementation(() => clock);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { animations.set(++nextAnimation, callback); return nextAnimation; });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => animations.delete(id));
  vi.stubGlobal('IntersectionObserver', class {
    constructor(callback: IntersectionObserverCallback) { observer = callback; }
    observe() {}
    disconnect() {}
  });
  vi.spyOn(window, 'matchMedia').mockImplementation(query => ({
    matches: reduced, media: query, addEventListener: vi.fn(), removeEventListener: vi.fn(),
  }) as unknown as MediaQueryList);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); reduced = false; });
const flush = () => act(async () => { await Promise.resolve(); });
const pointer = (element: HTMLElement, type: string, props: object) => {
  const event = new Event(type, { bubbles: true });
  Object.assign(event, { pointerId: 1, isPrimary: true, button: 0, clientX: 0, clientY: 0 }, props);
  fireEvent(element, event);
};
function stage() {
  const element = screen.getByRole('slider');
  Object.assign(element, { setPointerCapture: vi.fn(), hasPointerCapture: () => true, releasePointerCapture: vi.fn() });
  return element;
}
function tick(time: number) {
  clock = time;
  const callbacks = [...animations.values()];
  animations.clear();
  act(() => { for (const callback of callbacks) callback(time); });
}

it('keeps lazy poster, keyboard angles, reduced-motion controls and remembered vehicle view', async () => {
  reduced = true;
  const view = render(<RVehicleOrbit vehicleId="keyboard-contract" />);
  const slider = stage();
  expect(loader.get).not.toHaveBeenCalled();
  expect(slider).toHaveAttribute('aria-valuenow', '280');
  expect(snapshotDom(view.container)).toMatchSnapshot();
  fireEvent.keyDown(slider, { key: 'ArrowRight' });
  await flush();
  expect(slider).toHaveAttribute('aria-valuenow', '290');
  fireEvent.keyDown(slider, { key: 'Home' });
  await flush();
  expect(slider).toHaveAttribute('aria-valuenow', '325');
  fireEvent.click(screen.getByLabelText('Reset vehicle view'));
  await flush();
  expect(slider).toHaveAttribute('aria-valuenow', '280');
  fireEvent.keyDown(slider, { key: 'ArrowLeft' });
  await flush();
  view.unmount();
  expect(loader.close).toHaveBeenCalledOnce();
  render(<RVehicleOrbit vehicleId="keyboard-contract" />);
  await flush();
  expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '270');
});

it('keeps six-pixel drag threshold, vertical scroll cancellation and pointer ownership', async () => {
  reduced = true;
  render(<RVehicleOrbit vehicleId="pointer-contract" />);
  const slider = stage();
  pointer(slider, 'pointerdown', { clientX: 20, clientY: 20 });
  pointer(slider, 'pointermove', { pointerId: 2, clientX: 100 });
  pointer(slider, 'pointermove', { clientX: 25, clientY: 20 });
  expect(loader.get).not.toHaveBeenCalled();
  pointer(slider, 'pointermove', { clientX: 50, clientY: 20 });
  await flush();
  expect(slider).toHaveAttribute('aria-valuenow', '310');
  pointer(slider, 'pointerup', {});
  expect(slider.releasePointerCapture).toHaveBeenCalledWith(1);
  pointer(slider, 'pointerdown', { clientX: 0, clientY: 0 });
  pointer(slider, 'pointermove', { clientX: 1, clientY: 20 });
  pointer(slider, 'pointermove', { clientX: 100, clientY: 20 });
  expect(loader.get).toHaveBeenCalledTimes(1);
});

it('shows decode failure and permits retry through explicit interaction', async () => {
  reduced = true;
  loader.get.mockRejectedValueOnce(new Error('Synthetic decode failure'));
  render(<RVehicleOrbit vehicleId="retry-contract" />);
  fireEvent.keyDown(stage(), { key: 'ArrowRight' });
  await waitFor(() => expect(screen.getByText('Rotation unavailable. Try again.')).toBeInTheDocument());
  fireEvent.click(screen.getByText('Front'));
  await waitFor(() => expect(screen.getByText('Drag to turn · 360°')).toBeInTheDocument());
  expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '325');
});

it('plays one lazy entrance tour and cancels it when leaving the viewport', async () => {
  render(<RVehicleOrbit vehicleId="entrance-contract" />);
  act(() => observer([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver));
  await flush();
  expect(loader.get).toHaveBeenCalledWith(56);
  tick(2800);
  await flush();
  expect(screen.getByRole('slider')).toHaveAttribute('aria-valuenow', '235');
  act(() => observer([{ isIntersecting: false } as IntersectionObserverEntry], {} as IntersectionObserver));
  expect(animations.size).toBe(0);
  act(() => observer([{ isIntersecting: true } as IntersectionObserverEntry], {} as IntersectionObserver));
  expect(animations.size).toBe(0);
});

it('coalesces pending paint requests and ignores completion after unmount', async () => {
  reduced = true;
  let finish!: (value: object) => void;
  loader.get.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  const view = render(<RVehicleOrbit vehicleId="coalesce-contract" />);
  const slider = stage();
  fireEvent.keyDown(slider, { key: 'ArrowRight' });
  fireEvent.keyDown(slider, { key: 'ArrowRight' });
  fireEvent.keyDown(slider, { key: 'Home' });
  expect(loader.get).toHaveBeenCalledTimes(1);
  expect(loader.get).toHaveBeenLastCalledWith(58);
  await act(async () => finish({}));
  await waitFor(() => expect(loader.get).toHaveBeenLastCalledWith(65));
  expect(loader.get).toHaveBeenCalledTimes(2);
  loader.get.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  fireEvent.keyDown(slider, { key: 'End' });
  view.unmount();
  await act(async () => finish({}));
  expect(loader.close).toHaveBeenCalledOnce();
  expect(animations.size).toBe(0);
});
