import type React from 'react';
import type { DashboardConfig, WidgetCtx } from '@panda/dashboards';
import type { DashboardTimeframe, DateRange } from '../../lib/dates';

export interface DashboardPageShellRenderState {
  activeConfig: DashboardConfig | undefined;
  savedConfig: DashboardConfig | undefined;
  isEditMode: false;
  isLoading: boolean;
  vehicleId: string | null;
  ctx: WidgetCtx;
  timeframe: DashboardTimeframe;
  range: DateRange | null;
  chargeSessionDayLocal?: string | null;
  setChargeSessionDayLocal?: (dayLocal: string | null) => void;
  setTimeframe: React.Dispatch<React.SetStateAction<DashboardTimeframe>>;
}
