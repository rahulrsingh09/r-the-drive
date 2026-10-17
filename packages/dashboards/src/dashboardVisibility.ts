import type { VehicleStatus } from '@panda/types';
import type {
  DashboardVisibilityRule,
  DashboardVisibilityRuleType,
  VehicleConnectionVisibilityValue,
  WidgetInstance,
} from './schema';

export interface DashboardVisibilityState {
  'vehicle-connection': VehicleConnectionVisibilityValue;
}

/**
 * Legacy caller type retained for compatibility. DashboardRenderer ignores
 * scenario overrides and derives visibility from live vehicle status.
 */
export type DashboardPreviewState = DashboardVisibilityState;

interface VisibilityValueDefinition<T extends string> {
  value: T;
  label: string;
}

interface VisibilityConditionDefinition<T extends DashboardVisibilityRuleType, V extends string> {
  type: T;
  label: string;
  values: readonly VisibilityValueDefinition<V>[];
  resolve: (status: VehicleStatus | null | undefined) => V;
}

export const DASHBOARD_VISIBILITY_CONDITIONS = {
  'vehicle-connection': {
    type: 'vehicle-connection',
    label: 'Vehicle connection',
    values: [
      { value: 'plugged', label: 'Plugged in' },
      { value: 'unplugged', label: 'Unplugged' },
    ],
    resolve: (status) => isVehiclePluggedIn(status) ? 'plugged' : 'unplugged',
  } satisfies VisibilityConditionDefinition<'vehicle-connection', VehicleConnectionVisibilityValue>,
} as const;

export const DEFAULT_DASHBOARD_VISIBILITY_STATE: DashboardVisibilityState = {
  'vehicle-connection': 'unplugged',
};

export function dashboardVisibilityStateFromStatus(
  status: VehicleStatus | null | undefined,
): DashboardVisibilityState {
  return {
    'vehicle-connection': DASHBOARD_VISIBILITY_CONDITIONS['vehicle-connection'].resolve(status),
  };
}

export function getDashboardVisibilityOptionLabel(
  type: DashboardVisibilityRuleType,
  value: DashboardVisibilityState[DashboardVisibilityRuleType],
) {
  const option = DASHBOARD_VISIBILITY_CONDITIONS[type].values.find(
    (candidate) => candidate.value === value,
  );
  return option?.label ?? String(value);
}

export function isVehiclePluggedIn(status: VehicleStatus | null | undefined) {
  // A Rivian can remain connected while in standby without continuously
  // refreshing charger_state, so connection is semantic rather than timestamp-based.
  if (!status) return false;
  switch (status.charger_state?.toLowerCase()) {
    case undefined:
    case '':
    case 'unknown':
    case 'disconnected':
      return Boolean(status.charger_status && status.charger_status !== 'chrgr_sts_not_connected');
    default:
      return true;
  }
}

function legacyConnectionRule(widget: WidgetInstance): DashboardVisibilityRule | undefined {
  const value = widget.options?.['chargingConnectionVisibility'];
  if (value === 'plugged' || value === 'unplugged') return { type: 'vehicle-connection', value };
  return undefined;
}

export function getWidgetVisibilityRules(widget: WidgetInstance): DashboardVisibilityRule[] {
  // A nonempty typed list is authoritative. Empty lists still support older
  // saved/imported v2 configurations until the normalization boundary migrates them.
  if (widget.visibility?.length) return widget.visibility;
  const legacy = legacyConnectionRule(widget);
  return legacy ? [legacy] : [];
}

export function hasDashboardVisibilityRules(widgets: readonly WidgetInstance[]) {
  return widgets.some((widget) => getWidgetVisibilityRules(widget).length > 0);
}

export function getDashboardVisibilityRuleTypes(
  widgets: readonly WidgetInstance[],
): DashboardVisibilityRuleType[] {
  const types = new Set<DashboardVisibilityRuleType>();
  for (const widget of widgets) {
    for (const rule of getWidgetVisibilityRules(widget)) types.add(rule.type);
  }
  return Array.from(types);
}

export function isWidgetVisible(
  widget: WidgetInstance,
  state: DashboardVisibilityState,
) {
  return getWidgetVisibilityRules(widget).every((rule) => state[rule.type] === rule.value);
}

export function setVehicleConnectionVisibility(
  widget: WidgetInstance,
  value: VehicleConnectionVisibilityValue | null,
): WidgetInstance {
  const visibility = getWidgetVisibilityRules(widget)
    .filter(rule => rule.type !== 'vehicle-connection');
  if (value !== null) visibility.push({ type: 'vehicle-connection', value });
  const result: WidgetInstance = { ...widget, visibility };
  if (visibility.length === 0) delete result.visibility;
  return result;
}

export function migrateLegacyWidgetVisibility(widget: WidgetInstance): WidgetInstance {
  if (!legacyConnectionRule(widget)) return widget;
  const options = { ...widget.options };
  delete options['chargingConnectionVisibility'];
  return {
    ...widget,
    visibility: getWidgetVisibilityRules(widget),
    options,
  };
}
