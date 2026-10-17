import type { RestoreJob, UploadBackupResponse } from '@panda/types';
import { isRecord } from './normalization';

type UploadOutcome = { value: UploadBackupResponse } | { error: Error };
function uploadOutcome(status: number, text: string): UploadOutcome {
  let decoded: unknown = null;
  try { decoded = JSON.parse(text); } catch { /* Invalid successful payloads fail the shape check below. */ }
  const success = status >= 200 && status < 300;
  if (success && isRecord(decoded) && isRecord(decoded.artifact)) return { value: decoded as unknown as UploadBackupResponse };
  const envelope = isRecord(decoded) && isRecord(decoded.error) ? decoded.error : {};
  const description = typeof envelope.message === 'string' ? envelope.message : '';
  return { error: Object.assign(new Error(success ? 'Backup upload returned an invalid response.' : description || `Backup upload failed (${status}).`), { status, ...(success ? { code: 'INVALID_RESPONSE' } : {}) }) };
}

export function uploadBackup(base: string, token: string | null, file: File, progress?: (loaded: number, total: number) => void): Promise<UploadBackupResponse> {
  return new Promise((resolve, reject) => {
    const transfer = new XMLHttpRequest();
    const settle = (outcome: UploadOutcome) => {
      transfer.onload = transfer.onerror = transfer.onabort = null;
      transfer.upload.onprogress = null;
      if ('error' in outcome) reject(outcome.error); else resolve(outcome.value);
    };
    transfer.open('POST', `${base}/v1/admin/backups/imports`);
    transfer.withCredentials = true;
    const headers = new Map([
      ['Content-Type', file.type || 'application/gzip'],
      ['X-Panda-File-Name', file.name.replace(/[^a-zA-Z0-9._-]/g, '_')],
    ]);
    if (token) headers.set('Authorization', `Bearer ${token}`);
    headers.forEach((value, name) => transfer.setRequestHeader(name, value));
    transfer.upload.onprogress = ({ loaded, lengthComputable, total }) => progress?.(loaded, lengthComputable ? total : file.size);
    transfer.onload = () => settle(uploadOutcome(transfer.status, transfer.responseText));
    const interrupted = () => settle({ error: new Error('Backup upload was interrupted.') });
    transfer.onerror = interrupted;
    transfer.onabort = interrupted;
    transfer.send(file);
  });
}

export function restoreJobStatus(base: string, jobId: string, capabilityToken: string): Promise<RestoreJob> {
  const request: RequestInit = { headers: { 'X-Panda-Restore-Token': capabilityToken }, credentials: 'same-origin' };
  return fetch(`${base}/v1/restore-runtime/jobs/${jobId}`, request).then((response) => {
    if (response.ok) return response.json() as Promise<RestoreJob>;
    throw new Error(`Restore status is unavailable (${response.status}).`);
  });
}
