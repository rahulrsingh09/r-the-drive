import type { ChargeSession } from '@panda/types';
import { numeric, project, textual } from './rowProjection';

type LiveChargeFields = Pick<ChargeSession, 'live_soc_pct' | 'live_time_elapsed_seconds' | 'live_time_remaining_min' | 'live_charger_state' | 'live_charger_status' | 'live_started_at'>;

const decodeLive = project({
  live_soc_pct: numeric(null, 'live_soc_pct', 'current_soc', 'soc_pct'),
  live_time_elapsed_seconds: numeric(null, 'live_time_elapsed_seconds', 'time_elapsed_seconds'),
  live_time_remaining_min: numeric(null, 'live_time_remaining_min', 'live_time_remaining_minutes', 'time_remaining_min'),
  live_charger_state: textual(null, 'live_charger_state', 'current_charger_state', 'vehicle_charger_state'),
  live_charger_status: textual(null, 'live_charger_status', 'parallax_charger_status'),
  live_started_at: (row) => {
    const raw = row.live_started_at ?? row.live_session_started_at;
    return raw == null ? null : String(raw);
  },
});

export function liveFields(row: Record<string, unknown>): LiveChargeFields { return decodeLive(row); }
