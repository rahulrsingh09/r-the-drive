import { useApiQuery } from './queryRuntime';
import { useCacheMutation } from './cacheRuntime';
import { api } from './api';
import { useAuth } from './useAuth';

function queryKey(userId: string | null) {
  return ['auth', 'dashboard-chart-favorites', userId] as const;
}

export function useDashboardChartFavorites() {
  const userId = useAuth((state) => state.userId);
  return useApiQuery(queryKey(userId), 'getDashboardChartFavorites', [], [userId], {
    staleTime: 5 * 60 * 1000,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false
  });
}

export function useUpdateDashboardChartFavorite() {
  const userId = useAuth((state) => state.userId);
  return useCacheMutation(
    ({ key, chartId }: { key: string; chartId: string }) => api.updateDashboardChartFavorite(key, chartId),
    (data) => [{ key: queryKey(userId), value: data }],
  );
}
