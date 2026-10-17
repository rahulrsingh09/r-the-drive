import React from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { ChartPicker, ColorPicker, DateRangePicker, ResponsiveDialog, SelectPicker, Sidebar, Tooltip } from '@panda/ui/primitives';
import { DataTable } from '@panda/ui/tables';

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.style.overflow = ''; });

describe('foundation interaction contracts', () => {
  it('positions a wide picker above a low mobile trigger and navigates disabled options', async () => {
    const change = vi.fn();
    const viewport = Object.assign(new EventTarget(), { width: 360, height: 600, offsetTop: 0, offsetLeft: 0 });
    vi.stubGlobal('visualViewport', viewport);
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ left: 20, right: 220, top: 540, bottom: 580, width: 200, height: 40, x: 20, y: 540, toJSON: () => ({}) });
    vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockReturnValue(300);
    vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockReturnValue(400);
    render(<SelectPicker aria-label="Choice" value="b" onChange={change} options={[{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta', disabled: true }, { value: 'c', label: 'Gamma' }]} />);
    const trigger = screen.getByRole('button', { name: 'Choice' });
    fireEvent.keyDown(trigger, { key: 'ArrowDown' });
    const menu = screen.getByRole('listbox');
    expect(menu).toHaveStyle({ left: '12px', top: '246px', width: '336px', maxHeight: '288px' });
    await waitFor(() => expect(screen.getByRole('option', { name: 'Gamma' })).toHaveFocus());
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    await waitFor(() => expect(screen.getByRole('option', { name: 'Alpha' })).toHaveFocus());
    fireEvent.keyDown(document.activeElement!, { key: 'End' });
    await waitFor(() => expect(screen.getByRole('option', { name: 'Gamma' })).toHaveFocus());
    fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
    expect(change).toHaveBeenCalledWith('c');
    expect(trigger).toHaveFocus();
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('traps dialog tab edges and restores the trigger and prior scroll policy', () => {
    document.body.style.overflow = 'auto';
    const trigger = document.createElement('button'); document.body.append(trigger); trigger.focus();
    const close = vi.fn();
    const view = render(<ResponsiveDialog titleId="title" onClose={close}><h2 id="title">Dialog</h2><button>First</button><button>Last</button></ResponsiveDialog>);
    expect(screen.getByRole('button', { name: 'First' })).toHaveFocus();
    fireEvent.keyDown(document, { key: 'Tab', shiftKey: true });
    expect(screen.getByRole('button', { name: 'Last' })).toHaveFocus();
    fireEvent.keyDown(document, { key: 'Tab' });
    expect(screen.getByRole('button', { name: 'First' })).toHaveFocus();
    fireEvent.mouseDown(screen.getByRole('dialog'));
    expect(close).not.toHaveBeenCalled();
    fireEvent.mouseDown(screen.getByRole('dialog').parentElement!);
    expect(close).toHaveBeenCalledTimes(1);
    expect(document.body.style.overflow).toBe('hidden');
    view.unmount(); expect(trigger).toHaveFocus(); expect(document.body.style.overflow).toBe('auto'); trigger.remove();
  });

  it('restores mobile navigation focus on dismissal and reports route selection', async () => {
    const navigate = vi.fn();
    render(<Sidebar activeKey="dashboard" onNavigate={navigate} />);
    fireEvent.click(screen.getByRole('button', { name: 'Toggle navigation' }));
    const dialog = screen.getByRole('dialog', { name: 'Navigation' });
    expect(dialog).toHaveClass('min-h-[100dvh]', 'lg:hidden');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Close navigation' })).toHaveFocus());
    expect(document.body.style.overflow).toBe('hidden');
    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Toggle navigation' })).toHaveFocus());
    expect(document.body.style.overflow).toBe('');
    fireEvent.click(screen.getByRole('button', { name: 'Toggle navigation' }));
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Battery' }));
    expect(navigate).toHaveBeenCalledWith('/battery');
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('retains the selected chart during search and isolates the default action', () => {
    const change = vi.fn(), setDefault = vi.fn();
    render(<ChartPicker value="one" searchValue="two" onSearchChange={vi.fn()} options={[{ value: 'one', label: 'One' }, { value: 'two', label: 'Two' }]} onChange={change} onSetDefault={setDefault} />);
    fireEvent.focus(screen.getByPlaceholderText('Search charts'));
    expect(screen.getAllByRole('option').map(item => item.textContent)).toEqual(['One', 'Two']);
    fireEvent.click(screen.getByRole('button', { name: 'Set Two as default' }));
    expect(setDefault).toHaveBeenCalledWith('two'); expect(change).not.toHaveBeenCalled();
    expect(screen.getByRole('listbox')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('option', { name: /Two/ }));
    expect(change).toHaveBeenCalledWith('two'); expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('preserves tooltip description composition, viewport flip and outside dismissal', () => {
    vi.stubGlobal('innerWidth', 320); vi.stubGlobal('innerHeight', 300);
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
      const popup = this.getAttribute('role') === 'tooltip';
      return { x: 0, y: 0, left: 0, right: popup ? 224 : 30, top: 2, bottom: 22, width: popup ? 224 : 30, height: popup ? 60 : 20, toJSON: () => ({}) };
    });
    render(<Tooltip content="Help"><button aria-describedby="prior">Trigger</button></Tooltip>);
    const trigger = screen.getByRole('button'); fireEvent.focus(trigger);
    const popup = screen.getByRole('tooltip');
    expect(popup).toHaveStyle({ top: '30px', left: '8px', visibility: 'visible' });
    expect(trigger.getAttribute('aria-describedby')).toBe(`prior ${popup.id}`);
    fireEvent.pointerDown(document.body); expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('shares sorting between mobile cards and desktop cells and preserves column visibility', async () => {
    render(<DataTable data={[{ value: 3 }, { value: 1 }, { value: 2 }]} columns={[{ accessorKey: 'value', header: 'Value' }]} columnVisibilityMenu renderMobileRow={row => <div data-testid="item">{row.original.value}</div>} />);
    fireEvent.click(screen.getByRole('button', { name: 'Sort rows by' }));
    fireEvent.click(screen.getByRole('option', { name: 'Value' }));
    expect(screen.getAllByTestId('item').map(item => item.textContent)).toEqual(['1', '2', '3']);
    fireEvent.click(screen.getByRole('button', { name: 'Sort descending' }));
    expect(screen.getAllByTestId('item').map(item => item.textContent)).toEqual(['3', '2', '1']);
    fireEvent.contextMenu(screen.getByRole('columnheader'), { clientX: 30, clientY: 40 });
    const checkbox = screen.getByRole('checkbox', { name: 'Value' }); fireEvent.click(checkbox);
    expect(checkbox).not.toBeChecked(); expect(screen.queryByRole('columnheader')).toBeNull();
    fireEvent.keyDown(document, { key: 'Escape' }); expect(screen.queryByRole('checkbox')).toBeNull();
  });

  it('keeps color drafts separate until apply, including validation and cross-mode copy', () => {
    const apply = vi.fn(), close = vi.fn();
    render(<ColorPicker open value={{ light: '#112233', dark: '#445566' }} swatches={[]} onApply={apply} onOpenChange={close} />);
    fireEvent.change(screen.getByLabelText('HEX color'), { target: { value: 'bad' } });
    fireEvent.click(screen.getByRole('button', { name: 'Set' }));
    expect(screen.getByRole('alert')).toHaveTextContent('Enter a valid HEX color.');
    expect(apply).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Light → dark' }));
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
    expect(apply).toHaveBeenCalledWith({ light: '#112233', dark: '#112233' }); expect(close).toHaveBeenCalledWith(false);
  });

  it('orders reversed typed date endpoints and rejects invalid drafts', () => {
    const change = vi.fn();
    render(<DateRangePicker timeframe={{ kind: 'custom', from: new Date('2026-06-03T00:00:00Z'), to: new Date('2026-06-07T00:00:00Z') }} onChange={change} />);
    fireEvent.click(screen.getByRole('button', { name: /Jun 3/ }));
    const from = screen.getByDisplayValue('6/3/26'), to = screen.getByDisplayValue('6/7/26');
    fireEvent.change(from, { target: { value: 'invalid' } });
    fireEvent.click(screen.getByRole('button', { name: 'Apply custom range' }));
    expect(change).not.toHaveBeenCalled(); expect(screen.getByText(/Enter a valid From/)).toBeInTheDocument();
    fireEvent.change(from, { target: { value: '7/9/26 18:30' } });
    fireEvent.change(to, { target: { value: '7/5/26 06:00' } });
    fireEvent.click(screen.getByRole('button', { name: 'Apply custom range' }));
    expect(change).toHaveBeenCalledWith({ kind: 'custom', from: new Date('2026-07-05T06:00:00Z'), to: new Date('2026-07-09T18:30:00Z') });
  });
});
