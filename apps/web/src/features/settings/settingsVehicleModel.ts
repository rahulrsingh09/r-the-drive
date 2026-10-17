import type { Vehicle } from '@panda/types';
import { formatRivianRenewalDate, getRivianCredentialRenewalNotice } from '../../lib/rivianCredentialRenewal';

export function vehicleSettingsCapabilities(vehicle: Pick<Vehicle, 'membership_role' | 'is_demo' | 'rivian_vehicle_id'>) {
  const membershipRole = vehicle.membership_role ?? 'viewer';
  const capabilities = new Set({
    owner: ['manage', 'members'], manager: ['manage'], viewer: [],
  }[membershipRole]);
  const isDemo = vehicle.is_demo ?? Boolean(vehicle.rivian_vehicle_id?.startsWith('demo-'));
  return {
    membershipRole, isDemo,
    canManageVehicle: capabilities.has('manage'),
    canManageMembers: capabilities.has('members'),
    canCaptureIngestion: capabilities.has('manage') && !isDemo,
  };
}

export function settingsVehicleModel(vehicle: Vehicle, activeId: string | null | undefined) {
  const isActive = vehicle.id === activeId;
  const capabilities = vehicleSettingsCapabilities(vehicle);
  const { isDemo } = capabilities;
  const needsReauth = !isDemo && vehicle.auth_state === 'needs_reauth';
  const collectorStates: Record<string, { color: string; text: string }> = {
    ok: { color: 'var(--rm-status-positive)', text: isActive ? 'Active' : 'Connected' },
    connected: { color: 'var(--rm-status-positive)', text: isActive ? 'Active' : 'Connected' },
    passive: { color: 'var(--rm-border-default)', text: 'Standby' },
  };
  let status = { color: 'var(--rm-border-default)', text: isActive ? 'Active' : 'Offline' };
  if (vehicle.worker_health != null) {
    status = collectorStates[vehicle.worker_health] ?? { color: 'var(--rm-status-danger)', text: 'Error' };
  }
  if (needsReauth) status = { color: 'var(--rm-status-warning)', text: 'Login required' };
  if (isDemo) status = { color: 'var(--rm-status-info)', text: 'Demo data' };
  return {
    ...capabilities, isActive, needsReauth,
    renewalNotice: isDemo || needsReauth ? null : getRivianCredentialRenewalNotice(vehicle),
    expectedRenewalDate: isDemo ? null : formatRivianRenewalDate(vehicle.expected_renewal_at),
    healthColor: status.color, healthText: status.text,
    hasGlow: isDemo || needsReauth || vehicle.worker_health != null,
    modelLine: [vehicle.model, vehicle.year, vehicle.trim].filter(Boolean).join(' · ') || 'Vehicle details pending',
    batteryLabel: vehicle.battery_capacity_kwh == null ? '' : ` · ${vehicle.battery_capacity_kwh} kWh`,
  };
}
