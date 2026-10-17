import React from 'react';

const STORAGE_PREFIX = 'rm-show-dashboard-edit-button';
const CHANGE_EVENT = 'rm-dashboard-edit-button-preference-change';

function storageKey(userId: string | null | undefined) {
  return `${STORAGE_PREFIX}:${userId ?? 'anonymous'}`;
}

function subscribePreference(notify: () => void) {
  window.addEventListener('storage', notify);
  window.addEventListener(CHANGE_EVENT, notify);
  return () => {
    window.removeEventListener('storage', notify);
    window.removeEventListener(CHANGE_EVENT, notify);
  };
}

function serverPreference() { return false; }

export function useDashboardEditButtonPreference(userId: string | null | undefined) {
  const key = storageKey(userId);
  const read = React.useCallback(() => window.localStorage.getItem(key) === 'true', [key]);
  const showEditButton = React.useSyncExternalStore(subscribePreference, read, serverPreference);
  const updateShowEditButton = React.useCallback((next: boolean) => {
    if (typeof window === 'undefined') return;
    window.localStorage.setItem(key, String(next));
    window.dispatchEvent(new Event(CHANGE_EVENT));
  }, [key]);
  return [showEditButton, updateShowEditButton] as const;
}
