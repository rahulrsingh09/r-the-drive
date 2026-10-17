import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import type { DashboardPageShellProps } from '../components/dashboard/DashboardPageShell';
import type { DashboardPageShellRenderState } from '../components/dashboard/dashboardShellTypes';

vi.mock('../components/dashboard/DashboardPageShell', () => ({
  DashboardPageShell: (props: DashboardPageShellProps) => (
    <div>
      {props.renderLeadingActions?.({ vehicleId: 'v' } as DashboardPageShellRenderState)}
      {props.renderBeforeDashboard?.({ vehicleId: 'v' } as DashboardPageShellRenderState)}
    </div>
  ),
}));
vi.mock('@panda/dashboards', () => ({
  TripTagPicker: ({ onChange }: { onChange: (ids: string[]) => void }) =>
    <button onClick={() => onChange(['one', 'two'])}>Select tags</button>,
}));
vi.mock('@panda/ui/primitives', () => ({
  Tooltip: ({ children }: { children: React.ReactNode }) => children,
}));
import { EfficiencyDashboardPage } from '../components/dashboard/EfficiencyDashboardPage';

it('preserves manual visibility across filter updates and permission changes', () => {
  const filter = { tagIds: ['one'], untagged: false, tagMatch: 'all' as const, setFilter: vi.fn() };
  const page = (editable = true) => <EfficiencyDashboardPage navKey="efficiency" slug="efficiency" widgetCtx={{
    tripTagFilter: editable ? filter : { tagIds: filter.tagIds, untagged: false, tagMatch: 'all' },
  }} />;
  const { rerender } = render(page());
  fireEvent.click(screen.getByRole('button', { name: 'Hide efficiency filters' }));
  expect(screen.queryByRole('region', { name: 'Efficiency tag filters' })).toBeNull();
  expect(screen.getByRole('button', { name: 'Show efficiency filters' })).toHaveClass('text-accent');
  rerender(page(false));
  expect(screen.queryByRole('button', { name: 'Show efficiency filters' })).toBeNull();
  rerender(page());
  expect(screen.getByRole('button', { name: 'Show efficiency filters' })).toHaveAttribute('aria-expanded', 'false');
  filter.tagIds = [];
  rerender(page());
  expect(screen.getByRole('button', { name: 'Show efficiency filters' })).toHaveClass('text-fg-secondary');
});

it('retains filter dispatch, summaries and disabled match controls', () => {
  const setFilter = vi.fn();
  const filter = { tagIds: [] as string[], untagged: false, tagMatch: 'all' as const, setFilter };
  const page = () => <EfficiencyDashboardPage navKey="efficiency" slug="efficiency" widgetCtx={{ tripTagFilter: filter }} />;
  const { rerender } = render(page());
  fireEvent.click(screen.getByRole('button', { name: 'Show efficiency filters' }));
  expect(screen.getByText('All trips')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'any' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: 'Select tags' }));
  expect(setFilter).toHaveBeenLastCalledWith({ tagIds: ['one', 'two'], tagMatch: 'all', untagged: false });
  filter.tagIds = ['one', 'two'];
  rerender(page());
  expect(screen.getByText('2 tags selected')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'any' }));
  expect(setFilter).toHaveBeenLastCalledWith({ ...filter, tagMatch: 'any' });
  fireEvent.click(screen.getByRole('button', { name: 'Untagged' }));
  expect(setFilter).toHaveBeenLastCalledWith({ tagIds: [], tagMatch: 'all', untagged: true });
  filter.untagged = true;
  rerender(page());
  expect(screen.getByText('Untagged trips')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'any' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
  expect(setFilter).toHaveBeenLastCalledWith({ tagIds: [], tagMatch: 'all', untagged: false });
});
