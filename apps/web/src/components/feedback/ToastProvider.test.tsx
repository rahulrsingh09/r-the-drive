import React from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ToastProvider } from './ToastProvider';
import { TOAST_DISMISS_MS, addToast, emitToast, toastRole, type ToastEntry } from './toast';

function renderProvider() {
  return render(
    <ToastProvider>
      <p>Page</p>
    </ToastProvider>,
  );
}

function emit(title: string, variant: 'error' | 'info' | 'success' | 'warning' = 'error', message = `${title} message`) {
  act(() => emitToast({ title, message, variant }));
}

describe('toast helpers', () => {
  it('maps errors to alert and other variants to status', () => {
    expect(toastRole('error')).toBe('alert');
    expect(toastRole('info')).toBe('status');
    expect(toastRole('success')).toBe('status');
    expect(toastRole('warning')).toBe('status');
    expect(toastRole(undefined)).toBe('status');
  });

  it('merges duplicates into one counted entry and keeps at most three', () => {
    let id = 0;
    const next = () => `t${++id}`;
    let stack: ToastEntry[] = [];
    stack = addToast(stack, { title: 'A', message: 'a', variant: 'error' }, next);
    stack = addToast(stack, { title: 'B', message: 'b', variant: 'error' }, next);
    stack = addToast(stack, { title: 'A', message: 'a', variant: 'error' }, next);
    expect(stack.map((t) => [t.title, t.count, t.id])).toEqual([['B', 1, 't2'], ['A', 2, 't1']]);
    stack = addToast(stack, { title: 'A', message: 'a', variant: 'info' }, next);
    stack = addToast(stack, { title: 'C', message: 'c' }, next);
    expect(stack).toHaveLength(3);
    expect(stack.map((t) => t.title)).toEqual(['A', 'A', 'C']);
  });
});

describe('ToastProvider', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('uses role="alert" for errors and role="status" for info and success', () => {
    renderProvider();
    emit('Save failed', 'error');
    emit('Saved', 'success');
    emit('Heads up', 'info');
    expect(screen.getByRole('alert')).toHaveTextContent('Save failed');
    const statuses = screen.getAllByRole('status');
    expect(statuses.map((node) => node.textContent)).toEqual([
      expect.stringContaining('Saved'),
      expect.stringContaining('Heads up'),
    ]);
  });

  it('keeps one permanent polite live region, before and after toasts', () => {
    renderProvider();
    const region = screen.getByRole('region', { name: 'Notifications' });
    expect(region).toHaveAttribute('aria-live', 'polite');
    expect(region).toBeEmptyDOMElement();
    emit('Saved', 'success');
    emit('Heads up', 'info');
    expect(screen.getByRole('region', { name: 'Notifications' })).toBe(region);
    expect(region).toHaveTextContent('Saved');
    expect(region).toHaveTextContent('Heads up');
    expect(document.querySelectorAll('[aria-live="polite"]')).toHaveLength(1);
    act(() => vi.advanceTimersByTime(TOAST_DISMISS_MS + 1));
    expect(screen.getByRole('region', { name: 'Notifications' })).toBe(region);
  });

  it('shows at most three toasts, newest last', () => {
    renderProvider();
    for (const title of ['One', 'Two', 'Three', 'Four']) emit(title);
    const alerts = screen.getAllByRole('alert');
    expect(alerts).toHaveLength(3);
    expect(alerts.map((node) => node.textContent)).toEqual([
      expect.stringContaining('Two'),
      expect.stringContaining('Three'),
      expect.stringContaining('Four'),
    ]);
  });

  it('merges duplicate toasts and shows a count', () => {
    renderProvider();
    emit('Could not save the charge cost');
    emit('Could not save the charge cost');
    const alerts = screen.getAllByRole('alert');
    expect(alerts).toHaveLength(1);
    expect(alerts[0]).toHaveAttribute('data-toast-count', '2');
    expect(alerts[0]).toHaveTextContent('2×');
    expect(alerts[0]).toHaveTextContent('shown 2 times');
  });

  it('dismisses after the timeout', () => {
    renderProvider();
    emit('Saved', 'success');
    act(() => vi.advanceTimersByTime(TOAST_DISMISS_MS - 1));
    expect(screen.getByRole('status')).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('pauses auto-dismiss on hover and resumes with the remaining time', () => {
    renderProvider();
    emit('Save failed');
    act(() => vi.advanceTimersByTime(3000));
    fireEvent.mouseEnter(screen.getByRole('alert'));
    act(() => vi.advanceTimersByTime(TOAST_DISMISS_MS * 2));
    expect(screen.getByRole('alert')).toBeInTheDocument();
    fireEvent.mouseLeave(screen.getByRole('alert'));
    act(() => vi.advanceTimersByTime(TOAST_DISMISS_MS - 3000 - 1));
    expect(screen.getByRole('alert')).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('pauses auto-dismiss while keyboard focus is inside the toast', () => {
    renderProvider();
    emit('Save failed');
    const close = screen.getByRole('button', { name: 'Dismiss notification' });
    act(() => close.focus());
    act(() => vi.advanceTimersByTime(TOAST_DISMISS_MS * 2));
    expect(screen.getByRole('alert')).toBeInTheDocument();
    act(() => close.blur());
    act(() => vi.advanceTimersByTime(TOAST_DISMISS_MS));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('restarts the display time when a duplicate merges', () => {
    renderProvider();
    emit('Save failed');
    act(() => vi.advanceTimersByTime(TOAST_DISMISS_MS - 1000));
    emit('Save failed');
    act(() => vi.advanceTimersByTime(TOAST_DISMISS_MS - 1));
    expect(screen.getByRole('alert')).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('closes a toast from the Close button', () => {
    renderProvider();
    emit('Save failed');
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss notification' }));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});
