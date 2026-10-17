export function activeDestination(activeKey: string, visibleKeys: readonly string[]): string {
  const requested = activeKey.startsWith('battery') ? 'battery' : activeKey;
  return requested === 'battery' || visibleKeys.includes(requested) ? requested : 'explore';
}

export function canManagePeople(role: string | undefined) {
  return new Set(['admin', 'super_user']).has(role ?? '');
}

export function lockPresentation(locked: boolean | null) {
  const state = locked === null ? 'unknown' : String(locked) as 'true' | 'false';
  const labels = {
    unknown: 'Vehicle lock status unavailable', true: 'Vehicle locked', false: 'Vehicle unlocked',
  };
  return { label: labels[state], value: locked ?? 'unknown' };
}
