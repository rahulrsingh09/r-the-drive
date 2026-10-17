export type TelemetryTimeframe = '24h' | '7d' | '30d' | 'all';
export interface TelemetryExplorerState {
  vehicleId: string;
  timeframe: TelemetryTimeframe;
  search: string;
  selectedField: string;
  populatedOnly: boolean;
  page: number;
  selectedSampleTime: string | null;
  showEmptyFields: boolean;
  eventsOpen: boolean;
  selectedEventId: string | null;
  eventType: string;
  collectorOpen: boolean;
}
export const initialTelemetryExplorer: TelemetryExplorerState = {
  vehicleId: '', timeframe: '24h', search: '', selectedField: '', populatedOnly: true,
  page: 1, selectedSampleTime: null, showEmptyFields: false, eventsOpen: false,
  selectedEventId: null, eventType: '', collectorOpen: false,
};
type ExplorerChange = { [K in keyof TelemetryExplorerState]: { key: K; value: TelemetryExplorerState[K] } }[keyof TelemetryExplorerState];
export function telemetryExplorerTransition(state: TelemetryExplorerState, change: ExplorerChange): TelemetryExplorerState {
  const next = { ...state, [change.key]: change.value };
  switch (change.key) {
    case 'vehicleId':
    case 'timeframe':
    case 'search':
    case 'selectedField':
    case 'populatedOnly':
      next.page = 1;
      next.selectedSampleTime = null;
  }
  return next;
}
