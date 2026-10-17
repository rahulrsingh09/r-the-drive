import React from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import '../../../../packages/dashboards/src/widgets/sensor/SensorChipWidget';
import '../../../../packages/dashboards/src/widgets/table/TripStatWidget';
import { DashboardGrid } from '../../../../packages/dashboards/src/DashboardGrid';
import type { DashboardConfig } from '@panda/dashboards';

vi.mock('@panda/hooks', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@panda/hooks')>();
  return {
    ...actual,
    useMetricCatalog: () => ({ data: [] }),
  };
});

vi.mock('../../../../packages/dashboards/src/WidgetHost', () => ({
  WidgetHost: ({ instance }: { instance: { id: string; definitionId: string } }) => (
    <div data-testid={`widget-host-${instance.id}`}>{instance.definitionId}</div>
  ),
}));

const BASE_CTX = {
  vehicleId: 'vehicle-1',
  from: '2026-06-01T00:00:00Z',
  to: '2026-06-10T00:00:00Z',
};

const BASE_CONFIG: DashboardConfig = {
  schemaVersion: 2,
  id: '11111111-1111-1111-1111-111111111111',
  slug: 'editor-test',
  name: 'Editor Test',
  isDefault: false,
  isLocked: false,
  ownerId: null,
  controls: { dateRange: true },
  widgets: [
    {
      id: '22222222-2222-2222-2222-222222222222',
      componentType: 'sensor',
      definitionId: 'total_miles',
      title: 'Miles',
      layout: { x: 0, y: 0, w: 3, h: 2 },
      options: {},
    },
    {
      id: '33333333-3333-3333-3333-333333333333',
      componentType: 'custom',
      definitionId: 'trips.stat',
      title: 'Trips',
      layout: { x: 3, y: 0, w: 3, h: 2 },
      options: {},
    },
  ],
};

describe('Dashboard view chrome', () => {
  it('keeps edit-only overlays out of the view grid chrome', () => {
    render(<DashboardGrid widgets={BASE_CONFIG.widgets} ctx={BASE_CTX} />);

    expect(screen.getByTestId('widget-host-22222222-2222-2222-2222-222222222222')).toBeInTheDocument();
    expect(screen.queryByTestId('widget-overlay-left-22222222-2222-2222-2222-222222222222')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Edit widget settings' })).toBeNull();
  });

});
