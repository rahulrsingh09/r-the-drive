import React from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { colors, controlBorderClassName } from '../../../../packages/ui/src/tokens/colors';
import { DateRangePicker, Input, MetricTabs, SelectPicker } from '@panda/ui/primitives';

describe('control border token', () => {
  it('uses --rm-border-control and falls back to border-strong outside R themes', () => {
    expect(colors.border.control).toBe('var(--rm-border-control, var(--rm-border-strong))');
    expect(controlBorderClassName).toBe('border-[color:var(--rm-border-control,var(--rm-border-strong))]');
  });

  it('draws the edge of Input with the control border, and danger on error', () => {
    const { rerender } = render(<Input aria-label="Name" />);
    const input = screen.getByRole('textbox', { name: 'Name' });
    expect(input).toHaveClass(controlBorderClassName);
    expect(input).not.toHaveClass('border-border');

    rerender(<Input aria-label="Name" error="Required" />);
    expect(screen.getByRole('textbox', { name: 'Name' })).not.toHaveClass(controlBorderClassName);
    expect(screen.getByRole('textbox', { name: 'Name' })).toHaveClass('border-status-danger/50');
  });

  it('draws the edge of the SelectPicker trigger with the control border', () => {
    render(
      <SelectPicker
        aria-label="Unit"
        value="mi"
        onChange={vi.fn()}
        options={[{ value: 'mi', label: 'Miles' }, { value: 'km', label: 'Kilometers' }]}
      />
    );
    const trigger = screen.getByRole('button', { name: 'Unit' });
    expect(trigger).toHaveClass(controlBorderClassName);
    expect(trigger).not.toHaveClass('border-border');
  });

  it('draws the edge of the DateRangePicker trigger with the control border', () => {
    render(<DateRangePicker timeframe={{ kind: 'preset', preset: '30d' }} onChange={vi.fn()} />);
    const trigger = screen.getByRole('button', { name: /last 30 days/i });
    expect(trigger).toHaveClass(controlBorderClassName);
    expect(trigger).not.toHaveClass('border-border');
  });
});

describe('MetricTabs', () => {
  it('uses a surface token for tab hover, not the dialog scrim', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <MetricTabs
        tabs={[{ key: 'a', label: 'Battery' }, { key: 'b', label: 'Range' }]}
        active="a"
        onChange={onChange}
      >
        <div />
      </MetricTabs>
    );
    const inactive = screen.getByRole('button', { name: 'Range' });
    expect(inactive.className).toContain('hover:bg-bg-surface');
    expect(inactive.className).not.toContain('bg-bg-overlay');
    await user.click(inactive);
    expect(onChange).toHaveBeenCalledWith('b');
  });
});
