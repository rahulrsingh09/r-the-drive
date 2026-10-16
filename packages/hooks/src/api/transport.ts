/** Stable transport facade; endpoint contracts and transport stages are independently testable. */
export { AuthenticatedTransport, transport } from './clients';
export { resolveApiBaseUrl, setApiBaseUrl } from './apiClientTransport';
export type {
  ChargingSchedule,
  ChargingScheduleInput,
  DepartureOccurrence,
  DepartureComfortSettings,
  DepartureSchedule,
  DepartureScheduleInput,
  LiveSession,
  BackfillStatus,
} from './apiClientTransport';
