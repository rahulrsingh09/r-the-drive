import type { Vehicle } from '@panda/types';
import { DEFAULT_TARGET_TIRE_PRESSURE_PSI } from '@panda/ui/lib/vehicleTires';
import { ALL_PRESETS, R2_PRESET, RIVIAN_BATTERY_PRESETS, type BatteryGen } from './batteryPresets';

export interface VehicleSettingsDraft {
  editingBatteryVehicleId: string | null;
  batteryGen: BatteryGen;
  batteryPreset: string;
  customKwh: string;
  editTargetTirePressure: string;
  editNameValue: string;
}
export const initialVehicleSettings: VehicleSettingsDraft = {
  editingBatteryVehicleId: null, batteryGen: 'gen1', batteryPreset: 'r1_large_g1', customKwh: '',
  editTargetTirePressure: String(DEFAULT_TARGET_TIRE_PRESSURE_PSI), editNameValue: '',
};
export function openVehicleSettings(previous: VehicleSettingsDraft, vehicleId: string, vehicle: Vehicle | undefined, capacity: number | null | undefined): VehicleSettingsDraft {
  const next = { ...previous, editingBatteryVehicleId: vehicleId, editNameValue: vehicle?.display_name ?? '', editTargetTirePressure: String(vehicle?.target_tire_pressure_psi ?? DEFAULT_TARGET_TIRE_PRESSURE_PSI), customKwh: '' };
  if (capacity == null) {
    const model = vehicle?.model?.toLowerCase() ?? '';
    next.batteryGen = model.includes('gen 2') || model.includes('2nd') || (vehicle?.year ?? 0) >= 2025 ? 'gen2' : 'gen1';
    next.batteryPreset = next.batteryGen === 'gen2' ? 'r1_large_g2' : 'r1_large_g1';
    return next;
  }
  const candidates = (['gen1', 'gen2'] as const).flatMap((generation) => RIVIAN_BATTERY_PRESETS[generation].map((preset) => ({ generation, preset })));
  const chosen = candidates.find(({ preset }) => preset.key !== 'custom' && preset.kwh === capacity);
  if (chosen) return { ...next, batteryGen: chosen.generation, batteryPreset: chosen.preset.key };
  return capacity === R2_PRESET.kwh ? { ...next, batteryPreset: R2_PRESET.key } : { ...next, batteryPreset: 'custom', customKwh: String(capacity) };
}

export function vehicleSettingsPayload(draft: VehicleSettingsDraft) {
  const preset = [...RIVIAN_BATTERY_PRESETS[draft.batteryGen], ...ALL_PRESETS].find((candidate) => candidate.key === draft.batteryPreset);
  if (!preset) return null;
  const kwh = preset.key === 'custom' ? parseFloat(draft.customKwh) : preset.kwh!;
  const targetTirePressurePsi = parseFloat(draft.editTargetTirePressure);
  if (!Number.isFinite(kwh) || kwh <= 0 || !Number.isFinite(targetTirePressurePsi) || targetTirePressurePsi < 20 || targetTirePressurePsi > 80) return null;
  return { kwh, targetTirePressurePsi, config: preset.key === 'custom' ? `Custom (${kwh} kWh)` : preset.label };
}

type VehicleEditEvent = { type: 'generation'; value: BatteryGen } | { type: 'open'; vehicleId: string; vehicle: Vehicle | undefined; capacity: number | null | undefined } | { type: 'patch'; patch: Partial<VehicleSettingsDraft> };
export function vehicleSettingsTransition(state: VehicleSettingsDraft, event: VehicleEditEvent) {
  if (event.type === 'generation') return { ...state, batteryGen: event.value, batteryPreset: { gen1: 'r1_large_g1', gen2: 'r1_large_g2' }[event.value], customKwh: '' };
  return event.type === 'open' ? openVehicleSettings(state, event.vehicleId, event.vehicle, event.capacity) : { ...state, ...event.patch };
}
