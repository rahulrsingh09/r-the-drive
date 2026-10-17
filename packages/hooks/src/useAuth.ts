import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import type { AuthTokens } from '@panda/types';
import { api } from './api';

const EXPLICIT_LOGOUT_INTENT_KEY = 'rm-explicit-logout-intent';
let pendingLogout = false;
let sessionRevision = 0;

export function recordExplicitLogoutIntent(): void {
  pendingLogout = true;
  try {
    if (typeof window !== 'undefined') {
      window.sessionStorage.setItem(EXPLICIT_LOGOUT_INTENT_KEY, '1');
      pendingLogout = false;
    }
  } catch { /* The in-memory intent survives blocked browser storage. */ }
}

export function consumeExplicitLogoutIntent(): boolean {
  const memoryIntent = pendingLogout;
  pendingLogout = false;
  if (memoryIntent) return true;
  try {
    const storage = typeof window === 'undefined' ? null : window.sessionStorage;
    const recorded = storage?.getItem(EXPLICIT_LOGOUT_INTENT_KEY) === '1';
    if (recorded) storage!.removeItem(EXPLICIT_LOGOUT_INTENT_KEY);
    return recorded;
  } catch { return false; }
}

function userIdFromToken(token: string): string | null {
  try {
    const encoded = token.split('.')[1];
    if (!encoded) return null;
    const base64 = encoded.replace(/[-_]/g, (character) => character === '-' ? '+' : '/');
    const payload: unknown = JSON.parse(atob(base64.padEnd(Math.ceil(base64.length / 4) * 4, '=')));
    if (payload === null || typeof payload !== 'object' || !('sub' in payload)) return null;
    return typeof payload.sub === 'string' ? payload.sub : null;
  } catch { return null; }
}

interface AuthState {
  // Access token is kept in-memory only — never written to localStorage.
  // This prevents XSS from exfiltrating long-lived credentials.
  accessToken: string | null;
  userId: string | null;
  defaultVehicleId: string | null;
  activeVehicleId: string | null;
  isAuthenticated: boolean;
  // True while an initial bootstrap session resume is in flight on page load.
  isBootstrapping: boolean;

  login: (email: string, password: string) => Promise<void>;
  register: (email: string, password: string, setupToken?: string) => Promise<void>;
  acceptAccountInvitation: (token: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  resumeSession: () => Promise<boolean>;
  setTokens: (accessToken: string, defaultVehicleId: string | null) => void;
  setDefaultVehicleId: (vehicleId: string | null) => void;
  setActiveVehicleId: (vehicleId: string | null) => void;
  clearSession: () => void;
}

const emptySession = {
  accessToken: null,
  userId: null,
  defaultVehicleId: null,
  activeVehicleId: null,
  isAuthenticated: false,
  isBootstrapping: false,
} as const;

function sessionState(tokens: AuthTokens, activeVehicleId: string | null) {
  return {
    accessToken: tokens.access_token,
    userId: userIdFromToken(tokens.access_token),
    defaultVehicleId: tokens.default_vehicle_id ?? null,
    activeVehicleId,
    isAuthenticated: true,
    isBootstrapping: false,
  };
}

export const useAuth = create<AuthState>()(persist((set, get) => {
  const adopt = (tokens: AuthTokens, preserveSelection: boolean) => {
    api.setToken(tokens.access_token);
    set(sessionState(tokens, preserveSelection ? get().activeVehicleId : null));
  };
  const authenticate = async (request: () => Promise<AuthTokens>) => {
    const revision = ++sessionRevision;
    const tokens = await request();
    if (revision !== sessionRevision) throw Object.assign(new Error('The session changed before the request completed.'), { code: 'SESSION_CHANGED' });
    adopt(tokens, false);
  };
  return {
    ...emptySession,
    isBootstrapping: true,
    setTokens(accessToken, defaultVehicleId) {
      sessionRevision++;
      adopt({ access_token: accessToken, default_vehicle_id: defaultVehicleId, expires_in: 0 }, false);
    },
    setDefaultVehicleId: (defaultVehicleId) => set({ defaultVehicleId }),
    setActiveVehicleId: (activeVehicleId) => set({ activeVehicleId }),
    clearSession() {
      sessionRevision++;
      api.setToken(null);
      set(emptySession);
      if (typeof window !== 'undefined') window.dispatchEvent(new Event('panda:session-cleared'));
    },
    login: (email, password) => authenticate(() => api.login(email, password)),
    register: (email, password, setupToken) => authenticate(() => api.register(email, password, setupToken)),
    acceptAccountInvitation: (token, password) => authenticate(() => api.acceptAccountInvitation(token, password)),
    async logout() {
      recordExplicitLogoutIntent();
      get().clearSession();
      await api.logout().catch(() => undefined);
    },
    async resumeSession() {
      const revision = sessionRevision;
      try {
        const tokens = await api.resumeSession();
        if (revision !== sessionRevision) return false;
        if (tokens) adopt(tokens, true);
        return Boolean(tokens);
      } catch { return false; }
      finally { if (revision === sessionRevision) set({ isBootstrapping: false }); }
    },
  };
}, {
  name: 'rm-auth',
  partialize: ({ defaultVehicleId, activeVehicleId, userId }) => ({ defaultVehicleId, activeVehicleId, userId }),
}));

api.onAuthChange((tokens) => {
  useAuth.setState(tokens ? sessionState(tokens, useAuth.getState().activeVehicleId) : emptySession);
});
