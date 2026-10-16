import { expect, it, vi } from 'vitest';

const { setFetch } = vi.hoisted(() => ({ setFetch: vi.fn() }));
vi.mock('@iconify/react', () => ({ _api: { setFetch } }));
import '../../../../packages/dashboards/src/iconPolicy';

it('resolves missing icons without making network requests', async () => {
  const network = vi.spyOn(globalThis, 'fetch');
  const configured = setFetch.mock.calls[0];
  if (!configured) throw new Error('Icon network policy was not installed');
  const fetchIcon = configured[0] as typeof fetch;
  const response = await fetchIcon('https://api.iconify.design/lucide.json?icons=route');
  expect(response.status).toBe(404);
  expect(network).not.toHaveBeenCalled();
  network.mockRestore();
});
