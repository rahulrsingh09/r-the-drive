import React from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { ResponsiveDialog, SelectPicker } from '@panda/ui/primitives';

describe('SelectPicker', () => {
  it('opens a rich listbox and reports the selected option', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();

    render(
      <SelectPicker
        value="r1s"
        onChange={onChange}
        aria-label="Vehicle"
        options={[
          { value: 'r1s', label: 'R1S', description: 'Adventure SUV' },
          { value: 'r1t', label: 'R1T', description: 'Adventure truck' },
        ]}
      />,
    );

    await user.click(screen.getByRole('button', { name: 'Vehicle' }));

    expect(screen.getByRole('listbox')).toBeInTheDocument();
    expect(screen.getByRole('option', { name: /R1S Adventure SUV/i })).toHaveAttribute('aria-selected', 'true');

    await user.click(screen.getByRole('option', { name: /R1T Adventure truck/i }));
    expect(onChange).toHaveBeenCalledWith('r1t');
  });

  it('keeps a 44 px tall trigger at both sizes', () => {
    render(
      <>
        <SelectPicker value="15" onChange={vi.fn()} aria-label="Rows per page" size="sm" options={[{ value: '15', label: '15' }]} />
        <SelectPicker value="r1s" onChange={vi.fn()} aria-label="Select vehicle" options={[{ value: 'r1s', label: 'R1S' }]} />
      </>,
    );

    expect(screen.getByRole('button', { name: 'Rows per page' })).toHaveClass('h-11', 'text-xs');
    expect(screen.getByRole('button', { name: 'Select vehicle' })).toHaveClass('h-11', 'text-sm');
  });

  it('closes the picker before its parent dialog and restores keyboard focus', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(
      <ResponsiveDialog titleId="dialog-title" onClose={onClose}>
        <h2 id="dialog-title">Choose vehicle</h2>
        <SelectPicker value="r2" aria-label="Vehicle" onChange={vi.fn()}
          options={[{ value: 'r2', label: 'R2' }, { value: 'r1s', label: 'R1S' }]} />
      </ResponsiveDialog>,
    );
    const trigger = screen.getByRole('button', { name: 'Vehicle' });
    await user.click(trigger);
    expect(screen.getByRole('dialog')).toContainElement(screen.getByRole('listbox'));
    await user.keyboard('{Escape}');
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
    expect(trigger).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

});
