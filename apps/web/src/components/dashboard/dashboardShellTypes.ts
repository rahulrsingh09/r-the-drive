import type React from 'react';
import type { DashboardConfig, WidgetCtx } from '@panda/dashboards';
import type { DashboardTimeframe, DateRange } from '../../lib/dates';

export interface DashboardPageShellRenderState {
  activeConfig: DashboardConfig | undefined;
  savedConfig: DashboardConfig | undefined;
  localConfig: DashboardConfig | null;
  setLocalConfig: React.Dispatch<React.SetStateAction<DashboardConfig | null>>;
  isEditMode: boolean;
  isDirty: boolean;
  isLoading: boolean;
  saveError: string | null;
  setSaveError: React.Dispatch<React.SetStateAction<string | null>>;
  vehicleId: string | null;
  ctx: WidgetCtx;
  timeframe: DashboardTimeframe;
  range: DateRange | null;
  chargeSessionDayLocal?: string | null;
  setChargeSessionDayLocal?: (dayLocal: string | null) => void;
  setTimeframe: React.Dispatch<React.SetStateAction<DashboardTimeframe>>;
  enterEdit: () => void;
  exitEdit: () => void;
}
