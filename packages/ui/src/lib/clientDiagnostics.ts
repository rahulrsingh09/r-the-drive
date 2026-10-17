import { listen } from './browserRuntime';

export type ClientErrorSeverity = 'warn' | 'error';

export interface ClientErrorContext {
  event: string;
  area: string;
  operation: string;
  method?: string | undefined;
  path?: string | undefined;
  status?: number | undefined;
  code?: string | undefined;
  provider?: string | undefined;
  style?: string | undefined;
  resourceKind?: string | undefined;
  sourceId?: string | undefined;
  requestId?: string | undefined;
  url?: string | undefined;
  severity?: ClientErrorSeverity;
}

interface NormalizedError {
  name: string;
  message: string;
  stack?: string;
}

const REPORT_DEDUPE_WINDOW_MS = 5_000;
const MAX_TEXT_LENGTH = 240;
const reportedErrors = new WeakSet<object>();
const recentReports = new Map<string, number>();
const TOKEN_FIELDS = ['code', 'provider', 'style', 'resourceKind', 'sourceId', 'requestId'] as const;
const WARNING_AREAS = new Set(['api', 'query', 'mutation']);

export function isAbortError(error: unknown): boolean {
  return isRecord(error) && error.name === 'AbortError';
}

function claimReport(error: unknown, record: object): boolean {
  const now = Date.now();
  for (const [key, time] of recentReports) {
    if (time <= now - REPORT_DEDUPE_WINDOW_MS) recentReports.delete(key);
  }
  const key = JSON.stringify(record);
  if (recentReports.has(key)) return false;
  recentReports.set(key, now);
  if (isRecord(error)) reportedErrors.add(error);
  return true;
}

export function reportClientError(error: unknown, context: ClientErrorContext): void {
  if (isAbortError(error) || (isRecord(error) && reportedErrors.has(error))) return;
  const tokens = Object.fromEntries(TOKEN_FIELDS.map(field => [field, sanitizeToken(context[field])]));
  const record = {
    event: context.event, area: context.area, operation: context.operation,
    method: context.method, path: sanitizePath(context.path),
    status: typeof context.status === 'number' && Number.isFinite(context.status) ? context.status : undefined,
    ...tokens, url: sanitizePath(context.url), error: normalizeError(error),
  };
  if (!claimReport(error, record)) return;
  const severity = context.severity ?? (WARNING_AREAS.has(context.area) ? 'warn' : 'error');
  const summary = `[Panda client] ${record.event}: ${record.error.name}: ${record.error.message}`;
  if (severity === 'error') console.error(summary, record);
  else console.warn(summary, record);
}

export function installGlobalClientErrorHandlers(): () => void {
  if (typeof window === 'undefined') return () => undefined;
  const handle: EventListener = event => {
    let error: unknown;
    let context: ClientErrorContext;
    switch (event.type) {
      case 'unhandledrejection':
        error = (event as PromiseRejectionEvent).reason;
        context = { event: 'browser.unhandled_rejection', area: 'browser', operation: 'window.unhandledrejection' };
        break;
      case 'securitypolicyviolation': {
        const violation = event as SecurityPolicyViolationEvent;
        error = new Error(`Blocked ${violation.violatedDirective}`);
        context = { event: 'browser.csp_violation', area: 'browser', operation: violation.violatedDirective, url: violation.blockedURI };
        break;
      }
      default:
        if (event instanceof ErrorEvent || 'error' in event) {
          const failure = event as ErrorEvent;
          error = failure.error ?? failure.message;
          context = { event: 'browser.error', area: 'browser', operation: 'window.error', url: failure.filename };
        } else {
          const target = event.target as Element | null;
          error = new Error('Browser resource failed to load');
          context = { event: 'browser.resource_load_failed', area: 'asset', operation: target?.tagName?.toLowerCase() ?? 'unknown-resource', url: target?.getAttribute?.('src') ?? target?.getAttribute?.('href') ?? undefined };
        }
    }
    reportClientError(error, { ...context, severity: 'error' });
    event.preventDefault();
  };
  return listen([
    [window, 'error', handle, true],
    [window, 'unhandledrejection', handle],
    [window, 'securitypolicyviolation', handle],
  ]);
}

function normalizeError(error: unknown): NormalizedError {
  const source = typeof error === 'string' ? { message: error, name: 'Error' }
    : isRecord(error) ? error : null;
  const message = typeof source?.message === 'string' ? source.message : undefined;
  const name = typeof source?.name === 'string' ? source.name : undefined;
  if (!(error instanceof Error) && typeof error !== 'string' && !message && !name) {
    return { name: 'UnknownError', message: 'Unknown error' };
  }
  const normalized: NormalizedError = {
    name: sanitizeToken(name) ?? 'Error', message: sanitizeText(message) || 'Unknown error',
  };
  if (error instanceof Error && error.stack) normalized.stack = sanitizeText(error.stack);
  return normalized;
}

const PATH_REDACTIONS: ReadonlyArray<readonly [RegExp, string]> = [
  [/\/v[0-9]+\/external\/basemap\/openfreemap\/planet\/[^/]+\/[^/]+\/[^/]+\/[^/]+\.pbf$/i, '/v1/external/basemap/openfreemap/planet/<tile>.pbf'],
  [/\/[0-9a-f]{8}-[0-9a-f-]{27,}/gi, '/<id>'],
  [/\/[0-9]+\/[0-9]+\/[0-9]+\.(pbf|png)$/i, '/<tile>.$1'],
];
function sanitizePath(value: string | undefined): string | undefined {
  if (!value) return undefined;
  let pathname = value.split(/[?#]/, 1)[0] ?? value;
  try { pathname = new URL(value, 'http://localhost').pathname; } catch { /* Use the path before URL metadata. */ }
  for (const [pattern, replacement] of PATH_REDACTIONS) pathname = pathname.replace(pattern, replacement);
  return pathname.slice(0, MAX_TEXT_LENGTH);
}
function sanitizeText(value: string | undefined): string {
  const bearerRemoved = (value ?? '').replace(/Bearer\s+[A-Za-z0-9._~-]+/gi, 'Bearer <redacted>');
  return bearerRemoved.replace(/https?:\/\/[^\s)]+/gi, url => sanitizePath(url) ?? '<url>').slice(0, MAX_TEXT_LENGTH);
}
function sanitizeToken(value: string | undefined): string | undefined {
  return value ? sanitizeText(value).replace(/^Bearer\s+.+$/i, 'Bearer <redacted>') : undefined;
}
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}
