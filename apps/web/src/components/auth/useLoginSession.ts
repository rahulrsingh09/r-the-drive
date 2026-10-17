import { useEffect, useRef, useState } from 'react';
import { consumeExplicitLogoutIntent } from '@panda/hooks';

type SessionOptions = {
  authenticated: boolean;
  bootstrapping: boolean;
  resume: () => Promise<boolean>;
  enter: () => void;
  resumeAllowed: boolean;
  autoSsoAllowed: boolean;
  startSso: () => Promise<void>;
};

export function useLoginSession(options: SessionOptions) {
  const { authenticated, bootstrapping, resume, enter, resumeAllowed, autoSsoAllowed, startSso } = options;
  const attempts = useRef({ resume: null as Promise<boolean> | null, entered: false, sso: false, logout: null as boolean | null });
  const [restoring, setRestoring] = useState(false);
  useEffect(() => {
    const session = attempts.current;
    let active = true;
    const finish = (success: boolean) => {
      if (!active) return;
      if (success && !session.entered) {
        session.entered = true;
        enter();
      }
    };
    if (resumeAllowed) {
      if (authenticated) finish(true);
      else if (session.resume || bootstrapping) {
        session.resume ??= resume();
        setRestoring(true);
        void session.resume.then(finish).finally(() => { if (active) setRestoring(false); });
      }
    }
    return () => { active = false; };
  }, [authenticated, bootstrapping, resume, enter, resumeAllowed]);
  useEffect(() => {
    const session = attempts.current;
    session.logout ??= consumeExplicitLogoutIntent();
    const busy = authenticated || bootstrapping || restoring || session.sso;
    if (session.logout || busy || !autoSsoAllowed) return;
    session.sso = true;
    void startSso();
  }, [authenticated, bootstrapping, restoring, autoSsoAllowed, startSso]);
  return restoring;
}
