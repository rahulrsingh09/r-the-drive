import { describe, expect, it } from 'vitest';
import {
  dashboardVisibilityStateFromStatus,
  exportDashboardYaml,
  getDashboardVisibilityOptionLabel,
  getDashboardVisibilityRuleTypes,
  getWidgetVisibilityRules,
  hasDashboardVisibilityRules,
  importDashboardYaml,
  isWidgetVisible,
  migrateLegacyWidgetVisibility,
  normalizeDashboardConfig,
  setVehicleConnectionVisibility,
  type DashboardConfig,
  type WidgetInstance,
} from '@panda/dashboards';
import type { VehicleStatus } from '@panda/types';

const conditionalWidget: WidgetInstance = {
  id: '11111111-1111-4111-8111-111111111111',
  componentType: 'sensor',
  definitionId: 'charging_avg_session',
  layout: { x: 0, y: 0, w: 3, h: 2 },
  visibility: [{ type: 'vehicle-connection', value: 'plugged' }],
};

const config: DashboardConfig = {
  schemaVersion: 2,
  id: '22222222-2222-4222-8222-222222222222',
  slug: 'charging',
  name: 'Charging',
  isDefault: false,
  isLocked: false,
  ownerId: null,
  controls: { dateRange: true },
  widgets: [conditionalWidget],
};

describe('dashboard visibility rules', () => {
  it('keeps unconditional widgets visible and applies multiple rules with AND semantics', () => {
    const unconditional = { ...conditionalWidget, visibility: undefined };
    const contradictory = {
      ...conditionalWidget,
      visibility: [
        { type: 'vehicle-connection' as const, value: 'plugged' as const },
        { type: 'vehicle-connection' as const, value: 'unplugged' as const },
      ],
    };

    expect(isWidgetVisible(unconditional, { 'vehicle-connection': 'unplugged' })).toBe(true);
    expect(isWidgetVisible(conditionalWidget, { 'vehicle-connection': 'plugged' })).toBe(true);
    expect(isWidgetVisible(conditionalWidget, { 'vehicle-connection': 'unplugged' })).toBe(false);
    expect(isWidgetVisible(contradictory, { 'vehicle-connection': 'plugged' })).toBe(false);
  });

  it('treats connected standby as plugged and unknown status as unplugged', () => {
    const standby = { charger_state: 'standby' } as unknown as VehicleStatus;

    expect(dashboardVisibilityStateFromStatus(standby)).toEqual({ 'vehicle-connection': 'plugged' });
    expect(dashboardVisibilityStateFromStatus(undefined)).toEqual({ 'vehicle-connection': 'unplugged' });
  });

  it.each([
    [null, 'unplugged'],
    [{ charger_state: 'STANDBY', charger_status: 'chrgr_sts_not_connected' }, 'plugged'],
    [{ charger_state: 'disconnected', charger_status: 'chrgr_sts_not_connected' }, 'unplugged'],
    [{ charger_state: 'unknown', charger_status: 'chrgr_sts_connected_charging' }, 'plugged'],
    [{ charger_state: null, charger_status: 'chrgr_sts_connected_no_charging' }, 'plugged'],
    [{ charger_state: null, charger_status: null }, 'unplugged'],
  ] as const)('resolves connection precedence for %j', (status, expected) => {
    expect(dashboardVisibilityStateFromStatus(status as VehicleStatus | null))
      .toEqual({ 'vehicle-connection': expected });
  });

  it('keeps nonempty typed rules authoritative over conflicting legacy options', () => {
    const mixed: WidgetInstance = {
      ...conditionalWidget,
      options: { chargingConnectionVisibility: 'unplugged', accent: true },
    };
    const original = structuredClone(mixed);
    Object.freeze(mixed.options);
    Object.freeze(mixed.visibility);
    Object.freeze(mixed);
    const migrated = migrateLegacyWidgetVisibility(mixed);
    expect(getWidgetVisibilityRules(mixed)).toEqual(conditionalWidget.visibility);
    expect(migrated.visibility).toEqual(conditionalWidget.visibility);
    expect(migrated.options).toEqual({ accent: true });
    expect(mixed).toEqual(original);
    expect(migrateLegacyWidgetVisibility(migrated)).toBe(migrated);
  });

  it('replaces or clears typed connection rules without mutating inputs', () => {
    const original = structuredClone(conditionalWidget);
    const input: WidgetInstance = {
      ...conditionalWidget,
      visibility: [
        { type: 'vehicle-connection', value: 'plugged' },
        { type: 'vehicle-connection', value: 'plugged' },
      ],
    };
    Object.freeze(input.visibility);
    Object.freeze(input);
    const changed = setVehicleConnectionVisibility(input, 'unplugged');
    expect(changed.visibility).toEqual([{ type: 'vehicle-connection', value: 'unplugged' }]);
    const cleared = setVehicleConnectionVisibility(changed, null);
    expect(cleared).not.toHaveProperty('visibility');
    expect(isWidgetVisible(cleared, { 'vehicle-connection': 'plugged' })).toBe(true);
    expect(isWidgetVisible(cleared, { 'vehicle-connection': 'unplugged' })).toBe(true);
    expect(input.visibility).toHaveLength(2);
    expect(conditionalWidget).toEqual(original);
  });

  it('discovers effective rule types once, including legacy rules', () => {
    const unconditional = { ...conditionalWidget, visibility: undefined };
    const legacy = { ...unconditional, options: { chargingConnectionVisibility: 'unplugged' } };
    expect(hasDashboardVisibilityRules([])).toBe(false);
    expect(hasDashboardVisibilityRules([unconditional])).toBe(false);
    expect(hasDashboardVisibilityRules([legacy])).toBe(true);
    expect(getDashboardVisibilityRuleTypes([unconditional, legacy, conditionalWidget]))
      .toEqual(['vehicle-connection']);
    expect(getDashboardVisibilityOptionLabel('vehicle-connection', 'plugged')).toBe('Plugged in');
    expect(getDashboardVisibilityOptionLabel('vehicle-connection', 'unplugged')).toBe('Unplugged');
  });

  it('migrates legacy widget options to typed rules without changing other options', () => {
    const legacy = {
      ...config,
      widgets: [{
        ...conditionalWidget,
        visibility: undefined,
        options: { chargingConnectionVisibility: 'unplugged', accent: true },
      }],
    };
    const normalized = normalizeDashboardConfig(legacy);

    expect(normalized.widgets[0]?.visibility).toEqual([
      { type: 'vehicle-connection', value: 'unplugged' },
    ]);
    expect(normalized.widgets[0]?.options).toEqual({ accent: true });
  });

  it('round-trips typed rules through dashboard YAML', () => {
    const imported = importDashboardYaml(exportDashboardYaml(config));

    expect(imported.widgets[0]?.visibility).toEqual(conditionalWidget.visibility);
  });
});
