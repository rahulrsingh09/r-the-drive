type Sample = { ts?: string; value: number | null | undefined };

export function sensorSamples(series: Sample[]) {
  const finite = series.filter(
    (point) => typeof point.value === 'number' && Number.isFinite(point.value)
  );
  return {
    available: finite.length > 0,
    sprite(current: number | null | undefined, timestamp: string) {
      const fallback: Sample[] = Number.isFinite(current) && typeof current === 'number'
        ? [{ ts: timestamp, value: current }]
        : [];
      return finite.length ? finite : fallback;
    },
  };
}

export function sensorSourceLoading(
  source: string,
  shared: boolean,
  refreshing: boolean,
  pending: { health: boolean; charging: boolean; status: boolean; needsStatus: boolean }
) {
  const requests: Record<string, boolean[]> = {
    batteryHealth: [pending.health, pending.needsStatus && pending.status],
    chargingSummary: [pending.charging],
    vehicleStatus: [pending.status],
  };
  const active = requests[source];
  return active ? shared ? refreshing : active.includes(true) : false;
}
