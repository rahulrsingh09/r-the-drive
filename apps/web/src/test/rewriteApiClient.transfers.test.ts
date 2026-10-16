import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  AuthenticatedTransport,
  resolveApiBaseUrl,
  setApiBaseUrl,
} from '../../../../packages/hooks/src/api/transport';
import { vehicleClient, transport } from '../../../../packages/hooks/src/api/clients';

let client: AuthenticatedTransport;
beforeEach(() => {
  client = new AuthenticatedTransport();
  client.setToken('synthetic-access');
  setApiBaseUrl('');
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  setApiBaseUrl(undefined);
});

describe('specialized transport boundaries', () => {
  it.each([
    ['/v1/external/basemap/tiles/1', true],
    ['/v1/external/iconify/icons', true],
    ['/v1/external/basemap-elsewhere', false],
    ['/v1/example', false],
    ['https://other.example.invalid/v1/external/basemap/tiles/1', false],
  ])(
    'limits proxy bearer injection to the original first-party allowlist: %s',
    async (url, authorized) => {
      const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('synthetic'));
      await client.proxyFetch(url, { headers: { 'X-Synthetic': 'preserved' } });
      const init = fetchMock.mock.calls[0]?.[1];
      expect(new Headers(init?.headers).get('Authorization')).toBe(
        authorized ? 'Bearer synthetic-access' : null
      );
      expect(new Headers(init?.headers).get('X-Synthetic')).toBe('preserved');
      expect(init?.credentials).toBe(authorized ? 'same-origin' : undefined);
    }
  );

  it('passes through non-first-party proxy errors and responses', async () => {
    const response = new Response('synthetic', { status: 503 });
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValueOnce(response);
    expect(await client.proxyFetch('https://other.example.invalid/tile')).toBe(response);
    const failure = new TypeError('Synthetic network failure');
    fetchMock.mockRejectedValueOnce(failure);
    await expect(client.proxyFetch('https://other.example.invalid/tile')).rejects.toBe(failure);
  });

  it('rejects non-artwork paths before authenticated fetch', async () => {
    const fetchMock = vi.spyOn(globalThis, 'fetch');
    await expect(client.authenticatedAsset('/v1/vehicles')).rejects.toThrow('only accepts');
    await expect(
      client.authenticatedAsset('https://other.example.invalid/v1/vehicle-image-cache/a')
    ).rejects.toThrow('only accepts');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('retains download bytes and fallback filenames', async () => {
    vi.spyOn(globalThis, 'fetch').mockImplementation(async () => new Response('synthetic-bytes'));
    const backup = await client.downloadBackupArtifact('abc');
    const capture = await client.downloadVehicleIngestionCapture('vehicle');
    expect(backup.fileName).toBe('backup-abc');
    expect(capture.fileName).toBe('panda-capture.jsonl');
    expect(await backup.blob.text()).toBe('synthetic-bytes');
  });

  it('keeps restore status capability auth, same-origin cookies, and no renewal', async () => {
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValue(new Response('denied', { status: 401 }));
    await expect(client.getRestoreJob('job', 'synthetic-capability')).rejects.toThrow(
      'Restore status is unavailable (401).'
    );
    expect(fetchMock).toHaveBeenCalledExactlyOnceWith('/v1/restore-runtime/jobs/job', {
      headers: { 'X-Panda-Restore-Token': 'synthetic-capability' },
      credentials: 'same-origin',
    });
  });

  it('keeps detached domain methods bound to the shared facade session', async () => {
    transport.setToken('synthetic-shared');
    const { vehicleStatus } = vehicleClient;
    const fetchMock = vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('{}'));
    await vehicleStatus('v');
    expect(fetchMock.mock.calls[0]?.[1]?.headers).toMatchObject({
      Authorization: 'Bearer synthetic-shared',
    });
    transport.setToken(null);
  });

  it('preserves loopback proxy routing, configured origins, and SSR overrides', () => {
    const location = { hostname: 'app.example.invalid', origin: 'https://app.example.invalid' };
    expect(resolveApiBaseUrl('http://localhost:3001', location)).toBe('');
    expect(resolveApiBaseUrl('http://127.0.0.1:3001', location)).toBe('');
    expect(resolveApiBaseUrl('https://api.example.invalid', location)).toBe(
      'https://api.example.invalid'
    );
    expect(resolveApiBaseUrl('/api', location)).toBe('/api');
  });
});

class SyntheticUpload {
  static current: SyntheticUpload;
  constructor() {
    SyntheticUpload.current = this;
  }
  open = vi.fn();
  setRequestHeader = vi.fn();
  send = vi.fn();
  withCredentials = false;
  upload = {
    onprogress: undefined as
      ((event: { loaded: number; total: number; lengthComputable: boolean }) => void) | undefined,
  };
  onerror?: () => void;
  onabort?: () => void;
  onload?: () => void;
  status = 200;
  responseText = '{"artifact":{"id":"synthetic-artifact"}}';
}

describe('XHR backup upload contract', () => {
  beforeEach(() => vi.stubGlobal('XMLHttpRequest', SyntheticUpload));

  it('sends raw bytes with sanitized filename, bearer, cookies, and progress', async () => {
    const file = new File(['synthetic'], 'name /☃.gz', { type: 'application/gzip' });
    const progress = vi.fn();
    const result = client.uploadBackupArtifact(file, progress);
    const xhr = SyntheticUpload.current;
    expect(xhr.open).toHaveBeenCalledWith('POST', '/v1/admin/backups/imports');
    expect(xhr.withCredentials).toBe(true);
    expect(xhr.setRequestHeader.mock.calls).toEqual([
      ['Content-Type', 'application/gzip'],
      ['X-Panda-File-Name', 'name___.gz'],
      ['Authorization', 'Bearer synthetic-access'],
    ]);
    expect(xhr.send).toHaveBeenCalledWith(file);
    xhr.upload.onprogress?.({ loaded: 3, total: 12, lengthComputable: true });
    xhr.upload.onprogress?.({ loaded: 4, total: 0, lengthComputable: false });
    expect(progress.mock.calls).toEqual([
      [3, 12],
      [4, file.size],
    ]);
    xhr.onload?.();
    expect(await result).toEqual({ artifact: { id: 'synthetic-artifact' } });
  });

  it.each(['{', '', 'null', '[]', '{"artifact":null}'])(
    'rejects a malformed successful upload response: %s',
    async (body) => {
      const result = client.uploadBackupArtifact(new File([], 'empty'));
      SyntheticUpload.current.responseText = body;
      SyntheticUpload.current.onload?.();
      await expect(result).rejects.toMatchObject({ code: 'INVALID_RESPONSE', status: 200 });
    }
  );

  it.each(['onerror', 'onabort'] as const)('settles interrupted uploads: %s', async (event) => {
    const result = client.uploadBackupArtifact(new File([], 'empty'));
    SyntheticUpload.current[event]?.();
    await expect(result).rejects.toThrow('Backup upload was interrupted.');
  });

  it('keeps upload failure status and message without retrying', async () => {
    const result = client.uploadBackupArtifact(new File([], 'empty'));
    const xhr = SyntheticUpload.current;
    xhr.status = 401;
    xhr.responseText = '{"error":{"message":"Synthetic denied"}}';
    xhr.onload?.();
    await expect(result).rejects.toMatchObject({ status: 401, message: 'Synthetic denied' });
    expect(xhr.send).toHaveBeenCalledTimes(1);
  });
});
