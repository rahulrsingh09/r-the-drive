const loopbackHosts = new Set(['localhost', '127.0.0.1', '::1']);

export function configuredTarget(value: string, origin: string): { url?: URL; loopback: boolean } {
  try {
    const url = new URL(value, origin);
    return { url, loopback: loopbackHosts.has(url.hostname) };
  } catch { return { loopback: false }; }
}

export function websocketProtocol(value: string): string {
  const prefix = value.startsWith('https') ? ['https', 'wss'] : value.startsWith('http') ? ['http', 'ws'] : ['', ''];
  return prefix[1] + value.slice(prefix[0]!.length);
}
