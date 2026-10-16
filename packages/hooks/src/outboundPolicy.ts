// Mirrors the server's deployment policy; saved preferences cannot enable it.
export const VEHICLE_CONTROLS_ENABLED = false;
export const OPTIONAL_EXTERNAL_TRAFFIC_ENABLED = false;

export function isLocalArtworkSource(source: string | null | undefined): source is string {
  return Boolean(source?.startsWith('/') && !source.startsWith('//') && !/[\\\s]/.test(source));
}
