export interface ChargingSchedule {
  id: string;
  enabled: boolean;
  start_time_minutes: number | null;
  duration_minutes: number | null;
  amperage: number | null;
  location_lat: number | null;
  location_lng: number | null;
  week_days: string[] | null;
  rivian_updated_at: string | null;
  updated_at: string;
}

export interface ChargingScheduleInput {
  enabled: boolean;
  start_time_minutes?: number | null;
  duration_minutes?: number | null;
  amperage?: number | null;
  location_lat?: number | null;
  location_lng?: number | null;
  week_days?: string[] | null;
}

export interface DepartureOccurrence {
  type: 'RepeatsWeekly' | 'Once';
  days?: string[];
  time_minutes?: number;
}

export interface DepartureComfortSettings {
  seat_fl_heat?: number;
  seat_fr_heat?: number;
  seat_rl_heat?: number;
  seat_rr_heat?: number;
  cabin_temp_c?: number;
  defrost?: boolean;
}

export interface DepartureSchedule {
  id: string;
  rivian_schedule_id: string;
  name: string | null;
  enabled: boolean;
  occurrence: DepartureOccurrence | null;
  comfort_settings: DepartureComfortSettings | null;
  updated_at: string;
}

export interface DepartureScheduleInput {
  name?: string | null;
  enabled: boolean;
  occurrence?: DepartureOccurrence | null;
  comfort_settings?: DepartureComfortSettings | null;
}

export interface LiveSession {
  soc_pct: number | null;
  power_kw: number | null;
  energy_kwh: number | null;
  range_added_km: number | null;
  time_remaining_min: number | null;
  charger_type: string | null;
  ts: string;
  charge_rate_kph?: number | null;
  time_elapsed_seconds?: number | null;
  price?: number | null;
  currency?: string | null;
  is_free_session?: boolean | null;
  vehicle_charger_state?: string | null;
  started_at?: string | null;
  pack_energy_kwh?: number | null;
  thermal_energy_kwh?: number | null;
  provenance?: Partial<
    Record<
      'power_kw' | 'energy_kwh' | 'pack_energy_kwh' | 'thermal_energy_kwh' | 'time_remaining_min',
      { source: 'canonical' | 'parallax' | 'legacy_charging_session'; observed_at: string }
    >
  >;
}

export interface BackfillStatus {
  vehicle_id: string;
  history_backfilled_at: string | null;
  status: string | null;
  rivian_session_count: number | null;
  local_session_count: number;
  missing_source_count?: number | null;
}
