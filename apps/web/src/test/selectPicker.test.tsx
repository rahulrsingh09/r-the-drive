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
