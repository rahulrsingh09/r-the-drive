interface SessionPorts {
  resume: () => Promise<boolean>;
  clear: () => void;
  redirect: () => void;
  hydrate: (userId: string) => void;
  clearCache: (userId: string) => void;
}

export interface AuthObservation {
  authenticated: boolean;
  bootstrapping: boolean;
  token: string | null;
}

function jwtUserId(token: string): string | null {
  const encoded = /^[^.]*\.([^.]+)/.exec(token)?.[1];
  if (encoded === undefined) return null;
  try {
    return (JSON.parse(atob(encoded)) as { sub?: string }).sub ?? null;
  } catch {
    return null;
  }
}

export function createAuthSessionLifecycle(ports: SessionPorts) {
  const state = {
    mounted: false,
    bootstrapStarted: false,
    redirectStarted: false,
    recovery: false,
    owner: null as string | null,
  };
  const discardCache = () => {
    if (state.owner === null) return;
    const previous = state.owner;
    state.owner = null;
    ports.clearCache(previous);
  };
  const redirect = () => {
    if (state.redirectStarted) return;
    state.redirectStarted = true;
    ports.redirect();
  };
  const finishRecovery = (resumed: boolean) => {
    if (!resumed && state.mounted) {
      discardCache();
      ports.clear();
      redirect();
    }
    state.recovery = false;
  };
  return {
    mount(authenticated: boolean) {
      state.mounted = true;
      let active = true;
      if (!authenticated && !state.bootstrapStarted) {
        state.bootstrapStarted = true;
        void ports.resume().then(ok => { if (!ok && active) ports.clear(); });
      }
      return () => { active = false; state.mounted = false; };
    },
    observe({ authenticated, bootstrapping, token }: AuthObservation) {
      if (authenticated && token) {
        const owner = jwtUserId(token);
        if (owner && state.owner !== owner) {
          state.owner = owner;
          ports.hydrate(owner);
        }
      }
      if (!authenticated && !bootstrapping && !state.recovery) {
        discardCache();
        redirect();
      }
    },
    recover() {
      if (!state.mounted || state.recovery) return;
      state.recovery = true;
      void Promise.resolve().then(ports.resume).then(finishRecovery, () => finishRecovery(false));
    },
  };
}
