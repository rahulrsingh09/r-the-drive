export type VehicleGateCapability = 'liftgate' | 'tailgate';

const gateModels: readonly {
  capability: VehicleGateCapability;
  identifiers: readonly string[];
}[] = [
  { capability: 'tailgate', identifiers: ['R1T'] },
  { capability: 'liftgate', identifiers: ['R1S', 'R2'] },
];

/**
 * Resolve the single rear-closure capability that a known Rivian model should
 * display. Unknown models intentionally return null so observed telemetry can
 * decide which field is available instead of assuming both gates exist.
 */
export function resolveVehicleGateCapability(
  model: string | null | undefined
): VehicleGateCapability | null {
  const normalized = model?.trim().toUpperCase() ?? '';
  const match = gateModels.find(({ identifiers }) =>
    identifiers.some((identifier) => normalized.includes(identifier))
  );
  return match?.capability ?? null;
}
