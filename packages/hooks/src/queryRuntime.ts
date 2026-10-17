import { keepPreviousData, useQuery, type QueryKey, type UseQueryOptions } from '@tanstack/react-query';
import { api } from './api';
import { useAuthReady } from './useAuthState';

type Api = typeof api;
type Operation = { [K in keyof Api]: Api[K] extends (...args: never[]) => Promise<unknown> ? K : never }[keyof Api];
type Result<K extends Operation> = Awaited<ReturnType<Api[K]>>;
type Policy<T> = Omit<UseQueryOptions<T>, 'queryFn' | 'queryKey' | 'enabled' | 'placeholderData'> & { previous?: boolean };

export const queryPolicies = {
  history: { previous: true },
  detail: { previous: true, refetchOnWindowFocus: false, refetchOnReconnect: false, refetchOnMount: 'always' },
  schedule: { staleTime: 5 * 60 * 1000 },
} as const;

export function useResourceQuery<T>(
  key: QueryKey,
  load: () => Promise<T>,
  requirements: readonly unknown[],
  policy: Policy<T>,
) {
  const authenticated = useAuthReady();
  const { previous, ...options } = policy;
  return useQuery<T>({
    ...options,
    queryKey: key,
    queryFn: load,
    enabled: authenticated && requirements.every(Boolean),
    ...(previous ? { placeholderData: keepPreviousData } : {}),
  });
}

export function useApiQuery<K extends Operation>(
  key: QueryKey,
  operation: K,
  args: Parameters<Api[K]>,
  requirements: readonly unknown[],
  policy: Policy<Result<K>>,
) {
  return useResourceQuery(key, () => {
    const request = api[operation] as (...input: Parameters<Api[K]>) => Promise<Result<K>>;
    return request.apply(api, args);
  }, requirements, policy);
}

export function canonicalTagFilter(filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }) {
  const sorted = (filters?.tagIds ?? []).slice().sort();
  const tagIds = sorted.filter((value, index) => index === 0 || sorted[index - 1] !== value);
  return { tagIds, tagMatch: filters?.tagMatch === 'any' ? 'any' as const : 'all' as const, untagged: tagIds.length === 0 && Boolean(filters?.untagged) };
}
