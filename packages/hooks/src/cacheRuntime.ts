import { useMutation, useQueryClient, type QueryClient, type QueryKey } from '@tanstack/react-query';

type CacheCommand =
  | { key: QueryKey; invalidate: true }
  | { key: QueryKey; value: unknown }
  | { key: QueryKey; patch: Record<string, unknown> };

export function invalidateKeys(client: QueryClient, keys: readonly QueryKey[]): void {
  executeCacheCommands(client, keys.map((key) => ({ key, invalidate: true })));
}

function executeCacheCommands(client: QueryClient, commands: readonly CacheCommand[]) {
  for (const command of commands) {
    if ('invalidate' in command) { void client.invalidateQueries({ queryKey: command.key }); continue; }
    if ('value' in command) { client.setQueryData(command.key, command.value); continue; }
    client.setQueryData(command.key, (current: unknown) => current ? { ...current as object, ...command.patch } : current);
  }
}

export function useCacheMutation<T, V>(
  send: (variables: V) => Promise<T>,
  success: (data: T, variables: V) => readonly CacheCommand[],
  optimistic?: (variables: V) => { key: QueryKey; patch: Record<string, unknown>; restoreMissing?: boolean },
  settledKeys: readonly QueryKey[] = [],
) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: send,
    ...(optimistic ? {
      onMutate: async (variables: V) => {
        const plan = optimistic(variables);
        await client.cancelQueries({ queryKey: plan.key });
        const previous = client.getQueryData(plan.key);
        executeCacheCommands(client, [plan]);
        return { ...plan, previous };
      },
      onError: (_error: Error, _variables: V, context: { key: QueryKey; previous: unknown; restoreMissing?: boolean } | undefined) => {
        if (context && (context.restoreMissing || context.previous)) executeCacheCommands(client, [{ key: context.key, value: context.previous }]);
      },
    } : {}),
    onSuccess: (data, variables) => executeCacheCommands(client, success(data, variables)),
    ...(settledKeys.length ? { onSettled: () => invalidateKeys(client, settledKeys) } : {}),
  });
}

export function cacheInvalidations(keys: readonly QueryKey[]): CacheCommand[] {
  return keys.map((key) => ({ key, invalidate: true }));
}
