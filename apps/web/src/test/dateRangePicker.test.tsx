import React from 'react';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { DateRangePicker } from '@panda/ui/primitives';

describe('DateRangePicker', () => {
  it('toggles the custom editor from the preset list', async () => {
    const user = userEvent.setup();

    render(<DateRangePicker timeframe={{ kind: 'preset', preset: '30d' }} onChange={vi.fn()} />);

    expect(screen.queryByText('Custom range')).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: /last 30 days/i }));
    const dialog = screen.getByRole('dialog', { name: 'Select timeframe' });
    const modalShell = dialog.parentElement as HTMLElement;
    expect(modalShell).toHaveClass(
      'fixed',
      'inset-0',
      'items-center',
      'justify-center',
      'sm:absolute',
      'sm:right-0',
    );
    expect(dialog).toHaveClass(
      'max-h-[calc(100dvh-2rem)]',
      'w-full',
      'sm:min-w-[380px]',
      'sm:max-w-[calc(100vw-1rem)]'
    );
    await user.click(screen.getByRole('button', { name: 'Custom Range' }));
    expect(screen.getByText('Custom range')).toBeInTheDocument();
    expect(screen.getAllByText('mm/dd/yyyy')).toHaveLength(2);

    await user.click(screen.getByRole('button', { name: 'Custom Range' }));
    expect(screen.queryByText('Custom range')).not.toBeInTheDocument();
  });

  it('applies typed custom dates and keeps the calendar synced', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();

    render(
      <DateRangePicker
        timeframe={{
          kind: 'custom',
          from: new Date(2026, 5, 3, 10, 0),
          to: new Date(2026, 5, 7, 18, 30),
        }}
        onChange={onChange}
      />
    );

    await user.click(screen.getByRole('button', { name: /jun 3, 2026/i }));

    const fromInput = screen.getByDisplayValue('6/3/26') as HTMLInputElement;
    const toInput = screen.getByDisplayValue('6/7/26') as HTMLInputElement;

    expect(fromInput).toHaveClass('w-full');
    expect(fromInput).toHaveClass('min-w-0');

    const minutePickers = [
      screen.getByRole('button', { name: 'From minute' }),
      screen.getByRole('button', { name: 'To minute' }),
    ];
    expect(minutePickers).toHaveLength(2);
    await user.click(minutePickers[0]!);
    expect(screen.getAllByRole('option').map((option) => option.textContent)).toEqual([
      '00',
      '15',
      '30',
      '45',
    ]);
    await user.click(screen.getByRole('option', { name: '00' }));

    await user.clear(fromInput);
    await user.type(fromInput, '7/5/26');

    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Month' })).toHaveTextContent('July')
    );
    expect(fromInput).toHaveValue('7/5/26');

    await user.clear(toInput);
    await user.type(toInput, '7/8/26');
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Month' })).toHaveTextContent('July')
    );
    expect(toInput).toHaveValue('7/8/26');

    await user.click(screen.getByRole('button', { name: /apply custom range/i }));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: 'custom',
        from: expect.any(Date),
        to: expect.any(Date),
      })
    );
  });

  it('applies lifetime directly from the preset list', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();

    render(<DateRangePicker timeframe={{ kind: 'preset', preset: '30d' }} onChange={onChange} />);

    await user.click(screen.getByRole('button', { name: /last 30 days/i }));
    await user.click(screen.getByRole('button', { name: 'Lifetime' }));

    expect(onChange).toHaveBeenCalledWith({ kind: 'lifetime' });
  });
  it('gives the trigger, presets and custom-range controls a 44 px target', async () => {
    const user = userEvent.setup();

    render(<DateRangePicker timeframe={{ kind: 'preset', preset: '30d' }} onChange={vi.fn()} />);

    const trigger = screen.getByRole('button', { name: /last 30 days/i });
    expect(trigger).toHaveClass('h-11');
    await user.click(trigger);
    const dialog = screen.getByRole('dialog', { name: 'Select timeframe' });
    for (const preset of within(dialog).getAllByRole('button')) expect(preset).toHaveClass('min-h-11');

    await user.click(within(dialog).getByRole('button', { name: 'Custom Range' }));
    for (const name of ['From hour', 'From minute', 'To hour', 'To minute']) {
      expect(within(dialog).getByRole('button', { name })).toHaveClass('h-11');
      expect(within(dialog).getByRole('button', { name })).not.toHaveClass('h-7');
    }
    for (const input of within(dialog).getAllByPlaceholderText('6/3/26')) expect(input).toHaveClass('h-11');
    expect(within(dialog).getByRole('button', { name: 'Apply custom range' })).toHaveClass('h-11');
  });

  it('labels the month buttons and gives them and the day cells a 44 px hit area', async () => {
    const user = userEvent.setup();

    render(
      <DateRangePicker
        timeframe={{ kind: 'custom', from: new Date(2026, 5, 3, 10, 0), to: new Date(2026, 5, 7, 18, 30) }}
        onChange={vi.fn()}
      />
    );
    await user.click(screen.getByRole('button', { name: /jun 3, 2026/i }));

    const previous = screen.getByRole('button', { name: 'Previous month' });
    const next = screen.getByRole('button', { name: 'Next month' });
    for (const button of [previous, next]) expect(button).toHaveClass('h-11', 'w-11');

    await user.click(next);
    expect(screen.getByRole('button', { name: 'Month' })).toHaveTextContent('July');
    await user.click(previous);
    expect(screen.getByRole('button', { name: 'Month' })).toHaveTextContent('June');

    const grid = document.querySelector('[data-calendar-grid]') as HTMLElement;
    // No column gap below the sm breakpoint keeps 7 x 44 px inside a 390 px screen.
    expect(grid).toHaveClass('gap-x-0', 'sm:gap-x-1');
    const days = Array.from(grid.querySelectorAll('button'));
    expect(days).toHaveLength(30);
    for (const day of days) {
      expect(day).toHaveClass('h-11');
      // Low-alpha text fails 3:1; every day uses a full-strength text token.
      expect(day.className).not.toMatch(/text-[a-z-]+\/\d+/);
    }
  });
});
