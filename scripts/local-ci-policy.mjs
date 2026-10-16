export function selectHookGate(env = {}) {
  if (env.SKIP_LOCAL_CI === '1') return 'bypass';
  if (env.PANDA_FULL_LOCAL_CI === '1') return 'full';
  return 'local';
}
