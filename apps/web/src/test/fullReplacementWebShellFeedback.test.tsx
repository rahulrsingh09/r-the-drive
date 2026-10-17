import React from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { ToastProvider } from '../components/feedback/ToastProvider';
import { emitToast } from '../components/feedback/toast';

afterEach(() => vi.useRealTimers());

it('retains the newest three notifications, independent expiry, and manual dismissal', () => {
  vi.useFakeTimers();
  const { container } = render(<ToastProvider><p>Child surface</p></ToastProvider>);
  act(() => {
    emitToast({ title: 'A', message: 'first' });
    emitToast({ title: 'B', message: 'second', variant: 'error', code: 'E_SYNTHETIC' });
    emitToast({ title: 'C', message: 'third', variant: 'success' });
    emitToast({ title: 'D', message: 'fourth' });
  });
  expect(screen.queryByText('first')).not.toBeInTheDocument();
  expect(screen.getByText('second')).toBeInTheDocument();
  expect(screen.getByText('E_SYNTHETIC')).toBeInTheDocument();
  expect(container.querySelector('[aria-live]')).toHaveClass('w-[min(420px,calc(100vw-2rem))]');
  fireEvent.click(screen.getAllByRole('button', { name: 'Dismiss notification' })[1]!);
  expect(screen.queryByText('third')).not.toBeInTheDocument();
  act(() => { vi.advanceTimersByTime(6000); emitToast({ title: 'E', message: 'fifth' }); });
  act(() => { vi.advanceTimersByTime(1000); });
  expect(screen.queryByText('second')).not.toBeInTheDocument();
  expect(screen.queryByText('fourth')).not.toBeInTheDocument();
  expect(screen.getByText('fifth')).toBeInTheDocument();
  expect(screen.getByText('Child surface')).toBeInTheDocument();
});

it.each([
  [undefined, 'text-fg', 'border-border-strong'],
  ['error', 'text-status-danger', 'border-status-danger/70'],
  ['warning', 'text-status-warning', 'border-status-warning/70'],
  ['success', 'text-status-positive', 'border-status-positive/70'],
  ['info', 'text-status-info', 'border-status-info/70'],
] as const)('preserves toast paragraphs and visual classes for %j', (variant, titleClass, borderClass) => {
  const { container, unmount } = render(<ToastProvider><p>Children</p></ToastProvider>);
  act(() => emitToast({ title: 'Notice', message: 'Details', code: '', ...(variant ? { variant } : {}) }));
  expect(screen.getByText('Notice')).toHaveClass(titleClass);
  expect(container.querySelector('[aria-live] > div')).toHaveClass(borderClass);
  expect(container.querySelectorAll('[aria-live] p')).toHaveLength(2);
  act(() => emitToast({ title: 'Coded', message: 'More details', code: 'E_SYNTHETIC', ...(variant ? { variant } : {}) }));
  expect([...container.querySelectorAll('[aria-live] > div:last-child p')].map(node => node.textContent)).toEqual(['Coded', 'E_SYNTHETIC', 'More details']);
  unmount();
});
