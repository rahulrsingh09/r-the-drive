import { describe, expect, it } from 'vitest';
import {
  applyWidgetLayout,
  createWidgetInstance,
  dashboardKey,
  findOwnedDashboardBySlug,
  getDashboardOwnership,
  getDefaultBySlug,
  getWidget,
  isSystemDefaultDashboard,
  isUserOwnedDashboard,
  materializeSystemDashboardDraft,
  materializeUserDashboardDraft,
  resolveDashboardViewWidgets,
  widgetKeyForInstance,
  type DashboardConfig,
  type WidgetInstance,
} from '@panda/dashboards';

const systemDefault: DashboardConfig = {
  schemaVersion: 2,
  id: '00000000-0000-0000-0000-000000000002',
  slug: 'battery',
  name: 'Battery',
  isDefault: true,
  isLocked: true,
  ownerId: null,
  controls: { dateRange: true },
  widgets: [],
};

const userCopy: DashboardConfig = {
  ...systemDefault,
  id: '11111111-1111-1111-1111-111111111111',
  isDefault: false,
  isLocked: false,
  ownerId: '22222222-2222-2222-2222-222222222222',
};

function sensorWidget(
  id: string,
  definitionId: string,
  options: Record<string, unknown>,
  layout: WidgetInstance['layout'],
): WidgetInstance {
  return {
    id,
    componentType: 'sensor',
    definitionId,
    title: definitionId,
    options,
    layout,
  };
}

describe('dashboard model helpers', () => {
  it('keeps exact identity and ownership distinct from display slugs', () => {
    expect(dashboardKey(systemDefault, 'ignored')).toBe(`${systemDefault.id}:battery`);
    expect(dashboardKey(userCopy, 'ignored')).toBe(`${userCopy.id}:battery`);
    expect(dashboardKey(undefined, 'battery')).toBe('pending:battery');
    expect(dashboardKey(null, 'charging')).toBe('pending:charging');
    expect(getDashboardOwnership(systemDefault)).toBe('system-default');
    expect(getDashboardOwnership(userCopy)).toBe('user-owned');
    expect(isSystemDefaultDashboard(systemDefault)).toBe(true);
    expect(isSystemDefaultDashboard({ ...systemDefault, isDefault: false })).toBe(false);
    expect(isSystemDefaultDashboard({ ...userCopy, isDefault: true })).toBe(false);
    expect(isUserOwnedDashboard(userCopy)).toBe(true);
    expect(findOwnedDashboardBySlug(undefined, 'battery')).toBeUndefined();
    expect(findOwnedDashboardBySlug([systemDefault], 'battery')).toBeUndefined();
  });

  it('finds and materializes user-owned dashboard drafts without leaking default metadata', () => {
    expect(findOwnedDashboardBySlug([systemDefault, userCopy], 'battery')).toBe(userCopy);

    const materialized = materializeUserDashboardDraft(systemDefault, userCopy);

    expect(materialized).toMatchObject({
      id: userCopy.id,
      ownerId: userCopy.ownerId,
      isDefault: false,
      isLocked: false,
      slug: 'battery',
    });
  });

  it('materializes admin system-default drafts with the system row identity', () => {
    const materialized = materializeSystemDashboardDraft(
      { ...systemDefault, id: userCopy.id, ownerId: userCopy.ownerId, isDefault: false },
      systemDefault,
    );

    expect(materialized).toMatchObject({
      id: systemDefault.id,
      ownerId: null,
      isDefault: true,
      isLocked: true,
    });
  });

  it('allocates new IDs only for first customizations and preserves source metadata', () => {
    const saved = structuredClone(systemDefault);
    const original = structuredClone(saved);
    Object.freeze(saved);
    Object.freeze(saved.controls);
    Object.freeze(saved.widgets);
    const first = materializeUserDashboardDraft(saved);
    const second = materializeUserDashboardDraft(saved);
    expect(first.id).not.toBe(saved.id);
    expect(second.id).not.toBe(first.id);
    expect(first).toMatchObject({ ownerId: null, isDefault: false, isLocked: false });
    expect(materializeUserDashboardDraft(userCopy).id).toBe(userCopy.id);
    expect(saved).toEqual(original);
    expect(userCopy.isDefault).toBe(false);
  });

  it('creates independent widget identities and honors default and requested dimensions', () => {
    const definition = getWidget('sensor', 'total_miles')!;
    const original = structuredClone({
      defaultOptions: definition.defaultOptions,
      defaultSize: definition.defaultSize,
    });
    const first = createWidgetInstance(definition, { x: 0, y: 4 });
    const second = createWidgetInstance(definition, { x: 0, y: 8, w: 6, h: 3 });
    expect(first).toMatchObject({
      componentType: 'sensor',
      definitionId: 'total_miles',
      title: definition.title,
      layout: { ...definition.defaultSize, x: 0, y: 4 },
    });
    expect(second.id).not.toBe(first.id);
    expect(second.layout).toEqual({ x: 0, y: 8, w: 6, h: 3 });
    expect(widgetKeyForInstance(first)).toBe(`${first.id}:sensor:total_miles`);
    expect({ defaultOptions: definition.defaultOptions, defaultSize: definition.defaultSize })
      .toEqual(original);
  });

  it('preserves widget array identity when a layout callback makes no change', () => {
    const widgets = [
      sensorWidget(
        '33333333-3333-3333-3333-333333333333',
        'total_miles',
        {},
        { x: 0, y: 0, w: 3, h: 2 },
      ),
    ];

    const next = applyWidgetLayout(widgets, [{ i: widgets[0]!.id, x: 0, y: 0, w: 3, h: 2 }]);

    expect(next).toBe(widgets);
  });

  it('applies the last layout patch per ID without mutating hidden or unchanged widgets', () => {
    const widgets = [
      sensorWidget('33333333-3333-4333-8333-333333333331', 'total_miles', {}, { x: 0, y: 0, w: 3, h: 2 }),
      sensorWidget('33333333-3333-4333-8333-333333333332', 'total_miles', {}, { x: 3, y: 0, w: 3, h: 2 }),
    ];
    const original = structuredClone(widgets);
    for (const widget of widgets) {
      Object.freeze(widget.layout);
      Object.freeze(widget);
    }
    Object.freeze(widgets);
    const patches = Object.freeze([
      { i: widgets[0]!.id, x: 1, y: 2, w: 4, h: 3 },
      { i: 'not-in-this-dashboard', x: 0, y: 0, w: 12, h: 2 },
      { i: widgets[0]!.id, x: 6, y: 4, w: 6, h: 3 },
    ]);
    const next = applyWidgetLayout(widgets, patches);
    expect(next.map(widget => widget.id)).toEqual(original.map(widget => widget.id));
    expect(next[0]?.layout).toEqual({ x: 6, y: 4, w: 6, h: 3 });
    expect(next[1]).toBe(widgets[1]);
    expect(widgets).toEqual(original);
    expect(applyWidgetLayout(widgets, [])).toBe(widgets);
    expect(applyWidgetLayout(next, patches)).toBe(next);
  });

  it('sanitizes layout bounds and preserves fixed-size widget dimensions', () => {
    const fixed = getDefaultBySlug('charging')!.widgets.find(
      widget => widget.definitionId === 'charging.connection',
    )!;
    const unknown: WidgetInstance = {
      id: '33333333-3333-4333-8333-333333333333',
      componentType: 'custom',
      definitionId: 'contract.unknown',
      layout: { x: 0, y: 0, w: 3, h: 2 },
    };
    const result = applyWidgetLayout([unknown, fixed], [
      { i: unknown.id, x: 99, y: -10, w: 7.7, h: NaN },
      { i: fixed.id, x: 11, y: 7, w: 1, h: 1 },
    ]);
    expect(result[0]?.layout).toEqual({ x: 4, y: 0, w: 8, h: 1 });
    expect(result[1]?.layout).toMatchObject({ w: fixed.layout.w, h: fixed.layout.h, y: 7 });
    expect(result[1]!.layout.x + result[1]!.layout.w).toBeLessThanOrEqual(12);
  });

  it('resolves plugged and unplugged dashboard visibility from widget options', () => {
    const widgets = [
      sensorWidget(
        '44444444-4444-4444-4444-444444444444',
        'charging_home_share',
        {},
        { x: 0, y: 4, w: 3, h: 2 },
      ),
      sensorWidget(
        '55555555-5555-5555-5555-555555555555',
        'charging_dc_share',
        {},
        { x: 3, y: 4, w: 3, h: 2 },
      ),
      sensorWidget(
        '66666666-6666-6666-6666-666666666666',
        'charging_avg_session',
        { chargingConnectionVisibility: 'unplugged' },
        { x: 9, y: 0, w: 3, h: 2 },
      ),
      {
        id: '77777777-7777-7777-7777-777777777777',
        componentType: 'custom' as const,
        definitionId: 'charging.connection',
        title: 'Charging Connection',
        options: { chargingConnectionVisibility: 'plugged' },
        layout: { x: 6, y: 0, w: 6, h: 6 },
      },
    ];

    const unplugged = resolveDashboardViewWidgets(widgets, { 'vehicle-connection': 'unplugged' });
    expect(unplugged.map((widget) => widget.definitionId)).toEqual([
      'charging_home_share',
      'charging_dc_share',
      'charging_avg_session',
    ]);
    expect(unplugged[0]?.layout).toMatchObject({ x: 0, w: 6 });
    expect(unplugged[1]?.layout).toMatchObject({ x: 6, w: 6 });

    const plugged = resolveDashboardViewWidgets(widgets, { 'vehicle-connection': 'plugged' });
    expect(plugged.map((widget) => widget.definitionId)).toEqual([
      'charging_home_share',
      'charging_dc_share',
      'charging.connection',
    ]);
  });

  it('projects unplugged mix widths without changing the saved layout or hidden widgets', () => {
    const home = sensorWidget(
      '44444444-4444-4444-8444-444444444441', 'charging_home_share', {}, { x: 0, y: 4, w: 3, h: 2 },
    );
    const dc: WidgetInstance = {
      ...sensorWidget(
        '44444444-4444-4444-8444-444444444442', 'dc_share', {}, { x: 3, y: 4, w: 3, h: 2 },
      ),
      componentType: 'charging',
    };
    const hidden: WidgetInstance = {
      ...home,
      id: '44444444-4444-4444-8444-444444444443',
      visibility: [{ type: 'vehicle-connection', value: 'plugged' }],
    };
    const widgets = [home, dc, hidden];
    const original = structuredClone(widgets);
    for (const widget of widgets) {
      Object.freeze(widget.layout);
      Object.freeze(widget);
    }
    Object.freeze(widgets);
    const projected = resolveDashboardViewWidgets(widgets, { 'vehicle-connection': 'unplugged' });
    expect(projected.map(widget => widget.id)).toEqual([home.id, dc.id]);
    expect(projected.map(widget => widget.layout)).toEqual([
      { x: 0, y: 4, w: 6, h: 2 },
      { x: 6, y: 4, w: 6, h: 2 },
    ]);
    const plugged = resolveDashboardViewWidgets(widgets, { 'vehicle-connection': 'plugged' });
    expect(plugged).toEqual(original);
    expect(plugged[0]).toBe(home);
    expect(widgets).toEqual(original);
  });

  it('keeps both charging connection states compact before the chart', () => {
    const charging = getDefaultBySlug('charging');
    expect(charging).toBeDefined();

    const chart = charging!.widgets.find((widget) => widget.definitionId === 'catalog');
    const sessions = charging!.widgets.find(
      (widget) => widget.definitionId === 'charging.sessions.table',
    );
    expect(chart?.layout).toMatchObject({ x: 0, y: 6, w: 12, h: 9 });
    const charts = charging!.widgets.filter((widget) => widget.definitionId === 'catalog');
    expect(charts).toHaveLength(5);
    expect(sessions?.layout).toMatchObject({
      x: 0, y: Math.max(...charts.map((widget) => widget.layout.y + widget.layout.h)), w: 12, h: 12,
    });

    for (const connection of ['plugged', 'unplugged'] as const) {
      const visible = resolveDashboardViewWidgets(charging!.widgets, {
        'vehicle-connection': connection,
      });
      const summaryWidgets = visible.filter((widget) => widget.layout.y < 6);
      expect(
        Math.max(...summaryWidgets.map((widget) => widget.layout.y + widget.layout.h)),
      ).toBe(6);
      expect(visible.some((widget) => widget.definitionId === 'catalog')).toBe(true);
    }
  });
});
