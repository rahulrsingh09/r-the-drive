import type React from 'react';
import type { DashboardTimeframe } from '@panda/types';
import type { TripTagMatch } from '@panda/types';
import type { DashboardVisibilityState } from './dashboardVisibility';
import type { DashboardComponentType, WidgetInstance } from './schema';

export type { WidgetInstance };

export interface DashboardDataRequirements {
  metrics?: import('@panda/types').MetricBatchMetricRequest[];
  status?: boolean;
  batteryHealth?: boolean;
  chargingSummary?: boolean;
  efficiencySummary?: boolean;
}

export interface WidgetCtx {
  vehicleId: string | null;
  dashboardSlug?: string;
  dashboardConfigId?: string;
  timeframe?: DashboardTimeframe;
  from: string | null;
  to: string | null;
  chargeSessionDayLocal?: string | null;
  setChargeSessionDayLocal?: (dayLocal: string | null) => void;
  chargeSessionId?: string | null;
  chargeSessionActive?: boolean;
  chargeSessionEnergyKwh?: number | null;
  updateWidgetOptions?: (widgetId: string, patch: Record<string, unknown>) => void;
  updateWidgetLayout?: (widgetId: string, nextHeight: number) => void;
  /** Temporary editor preview state; absent in normal dashboard view mode. */
  visibilityState?: DashboardVisibilityState;
  /** Trip-page-only server filter; omitted by every other dashboard. */
  tripTagFilter?: {
    tagIds: string[];
    tagMatch: TripTagMatch;
    untagged: boolean;
    setFilter?: (next: { tagIds: string[]; tagMatch: TripTagMatch; untagged: boolean }) => void;
  };
  /** Vehicle membership derived by the Trips route, never from account role. */
  canManageTripTags?: boolean;
}

export interface WidgetEditorMeta {
  category?: string;
  description?: string;
  deprecated: boolean;
  fixedSize: boolean;
  movable: boolean;
  resizable: boolean;
  maxSize?: { w: number; h: number };
}

export interface WidgetDef {
  componentType: DashboardComponentType;
  definitionId: string;
  title: string;
  defaultSize: { w: number; h: number };
  minSize: { w: number; h: number };
  defaultOptions?: Record<string, unknown>;
  /** Declares the compact shared data this widget needs in dashboard view. */
  dataRequirements?: (instance: WidgetInstance) => DashboardDataRequirements | undefined;
  editor?: Partial<WidgetEditorMeta>;
  component: React.ComponentType<{ instance: WidgetInstance; ctx: WidgetCtx }>;
}

interface RegisteredWidget {
  definition: WidgetDef;
  editor: WidgetEditorMeta;
}

const registry = new Map<string, RegisteredWidget>();
const DEFAULT_EDITOR_META: WidgetEditorMeta = {
  deprecated: false,
  fixedSize: false,
  movable: true,
  resizable: true,
};

export function widgetKey(componentType: DashboardComponentType, definitionId: string) {
  return `${componentType}:${definitionId}`;
}

export function registerWidget(def: WidgetDef) {
  const editor = Object.assign({}, DEFAULT_EDITOR_META, def.editor);
  editor.resizable = editor.fixedSize ? false : editor.resizable;
  registry.set(widgetKey(def.componentType, def.definitionId), { definition: def, editor });
}

export function getWidget(componentType: DashboardComponentType, definitionId: string): WidgetDef | undefined {
  return registry.get(widgetKey(componentType, definitionId))?.definition;
}

export function getWidgetForInstance(instance: WidgetInstance): WidgetDef | undefined {
  return getWidget(instance.componentType, instance.definitionId);
}

export function getAllWidgets(): WidgetDef[] {
  return Array.from(registry.values(), entry => entry.definition);
}

export function getWidgetKeys(): string[] {
  return Array.from(registry.keys());
}

export function getWidgetEditorMeta(def: WidgetDef | undefined): WidgetEditorMeta {
  const registered = def && registry.get(widgetKey(def.componentType, def.definitionId));
  return registered ? registered.editor : DEFAULT_EDITOR_META;
}
