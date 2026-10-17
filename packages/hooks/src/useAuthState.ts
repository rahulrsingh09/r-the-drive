import { useAuth } from './useAuth';

export function useAuthReady() {
  return useAuth(({ accessToken, isAuthenticated, isBootstrapping }) => Boolean(accessToken) && isAuthenticated && !isBootstrapping);
}
