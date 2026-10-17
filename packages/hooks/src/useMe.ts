import { useApiQuery, queryPolicies } from './queryRuntime';

const ONE_HOUR_MS = 60 * 60 * 1000;

export function useMe() {

  return useApiQuery(['me'], 'me', [], [], {
    ...queryPolicies.history,
    staleTime: ONE_HOUR_MS,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false
  });
}
