export interface DailyChargeSessionsDay {
  day_local: string;
  day_start: string;
  total_energy_kwh: number;
  session_count: number;
}

export interface DailyChargeSessionsSession {
  session_id: string;
  day_local: string;
  day_start: string;
  started_at: string;
  energy_added_kwh: number | null;
  cost_usd: number | null;
  charger_type: string | null;
  location_name: string | null;
}
