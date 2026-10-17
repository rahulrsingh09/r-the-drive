import type { VehicleStatus } from '@panda/types';
import { reportClientError } from '@panda/ui/lib/clientDiagnostics';

export type VehicleConnectionState = 'idle' | 'connecting' | 'online' | 'failed';
export const VEHICLE_CREDENTIALS_REFRESHED_EVENT = 'panda:vehicle-credentials-refreshed';
export interface ConnectionHistory { removed: string | null; refreshAttempted: boolean; }
interface ConnectionSink {
  transition: (state: VehicleConnectionState) => void;
  connected: (connected: boolean) => void;
  status: (patch: Partial<VehicleStatus>) => void;
  current: () => VehicleStatus | undefined;
  revoke: () => void;
  refresh: () => Promise<unknown>;
  clearSession: () => void;
}

export class LiveVehicleConnection {
  private socket: WebSocket | null = null;
  private stopped = false;
  private mode: 'transport' | 'renewing' | 'revoked' = 'transport';
  private delay = 1000;
  private attempts = 0;
  private timestamps = { created: null as number | null, opened: null as number | null, message: null as number | null };
  private timers = new Map<string, ReturnType<typeof setTimeout>>();
  private detach: Array<() => void> = [];

  constructor(private vehicleId: string, private token: string, private url: string, private history: ConnectionHistory, private sink: ConnectionSink) {}

  start() {
    const listen = (target: EventTarget, type: string, handler: EventListener) => {
      target.addEventListener(type, handler);
      this.detach.push(() => target.removeEventListener(type, handler));
    };
    listen(window, 'online', this.wake);
    listen(window, 'focus', this.wake);
    listen(window, 'offline', this.offline);
    listen(document, 'visibilitychange', () => { if (document.visibilityState === 'visible') this.wake(); });
    listen(window, VEHICLE_CREDENTIALS_REFRESHED_EVENT, (event) => {
      const vehicleId = (event as CustomEvent<{ vehicleId?: string }>).detail?.vehicleId;
      if (vehicleId && vehicleId !== this.vehicleId) return;
      this.resetBackoff();
      this.reconnect();
    });
    const interval = setInterval(() => this.checkLiveness(), 30_000);
    this.detach.push(() => clearInterval(interval));
    this.connect();
  }

  stop() {
    this.stopped = true;
    this.detach.splice(0).forEach((remove) => remove());
    for (const timer of this.timers.values()) clearTimeout(timer);
    this.timers.clear();
    const socket = this.socket;
    this.socket = null;
    if (socket) {
      Object.assign(socket, { onopen: null, onmessage: null, onclose: null, onerror: null });
      socket.close();
    }
  }

  private cancel(name: string) { clearTimeout(this.timers.get(name)); this.timers.delete(name); }
  private schedule(name: string, delay: number, action: () => void) {
    this.cancel(name);
    this.timers.set(name, setTimeout(() => {
      this.timers.delete(name);
      if (!this.stopped) action();
    }, delay));
  }
  private resetBackoff() { this.delay = 1000; this.attempts = 0; }
  private transition(state: VehicleConnectionState) {
    this.sink.connected(state === 'online');
    this.sink.transition(state);
  }
  private available() { return !this.stopped && this.mode === 'transport' && this.history.removed !== this.vehicleId; }

  private connect() {
    if (!this.available() || this.socket && (this.socket.readyState === WebSocket.CONNECTING || this.socket.readyState === WebSocket.OPEN)) return;
    this.cancel('reconnect');
    const socket = new WebSocket(this.url, ['bearer', `bearer.${this.token}`]);
    this.socket = socket;
    this.timestamps = { created: Date.now(), opened: null, message: null };
    this.sink.transition('connecting');
    const current = () => !this.stopped && this.socket === socket;
    socket.onopen = () => {
      if (!current()) return;
      this.history.refreshAttempted = false;
      this.resetBackoff();
      this.timestamps.opened = Date.now();
      this.timestamps.message = null;
      this.transition('connecting');
      this.probe('initial-probe');
    };
    socket.onmessage = (event) => {
      if (!current()) return;
      this.timestamps.message = Date.now();
      this.cancel('probe');
      this.transition('online');
      try {
        const message = JSON.parse(event.data as string) as Record<string, unknown>;
        let kind: 'partial' | 'snapshot' | 'ignored' = 'ignored';
        let patch: Partial<VehicleStatus> | null = null;
        if ('data' in message && message.data) {
          kind = 'partial';
          const values = Object.fromEntries(Object.entries(message.data as object).filter(([, value]) => value != null));
          if (values.range_miles === undefined && values.distance_to_empty_mi != null) values.range_miles = values.distance_to_empty_mi;
          patch = { vehicle_id: this.vehicleId, is_online: true, last_updated: (message.ts ?? new Date().toISOString()) as string, ...values };
        } else if ('vehicle_id' in message) { kind = 'snapshot'; patch = message as unknown as VehicleStatus; }
        debugFrame(this.vehicleId, kind, message, patch, this.sink.current());
        if (patch) this.sink.status(patch);
      } catch (error) { this.report(error, 'live.websocket_message_invalid', 'message-parse'); }
    };
    socket.onclose = (event) => { if (current()) this.closed(event.code); };
    socket.onerror = () => {
      if (!current()) return;
      this.report(new Error('Live status websocket failed'), 'live.websocket_error', 'live-status');
      socket.close();
    };
  }

  private closed(code: number) {
    this.socket = null;
    this.cancel('probe');
    this.timestamps = { created: null, opened: null, message: null };
    this.sink.connected(false);
    switch (code) {
      case 4403:
        this.mode = 'revoked';
        this.history.removed = this.vehicleId;
        this.sink.revoke();
        this.sink.transition('idle');
        return;
      case 4401:
        this.mode = 'renewing';
        this.sink.transition('connecting');
        if (this.history.refreshAttempted) { this.sink.clearSession(); return; }
        this.history.refreshAttempted = true;
        this.renew();
        return;
    }
    if (!this.available()) { this.sink.transition('idle'); return; }
    if (navigator.onLine === false) { this.sink.transition('connecting'); return; }
    this.sink.transition(++this.attempts > 5 ? 'failed' : 'connecting');
    this.schedule('reconnect', this.delay, () => {
      this.delay = Math.min(this.delay * 2, 60_000);
      this.connect();
    });
  }

  private renew() {
    void this.sink.refresh().catch((error: unknown) => {
      if (this.stopped) return;
      const status = error && typeof error === 'object' && 'status' in error ? error.status : null;
      if (status === 401 || status === 403) { this.sink.clearSession(); return; }
      this.sink.transition('failed');
      this.schedule('reconnect', this.delay, () => this.renew());
      this.delay = Math.min(this.delay * 2, 60_000);
    });
  }

  private reconnect() {
    if (!this.available()) return;
    this.cancel('probe');
    if (!this.socket || this.socket.readyState === WebSocket.CLOSED) { this.socket = null; this.connect(); return; }
    if (this.socket.readyState === WebSocket.CLOSING) return;
    this.transition('connecting');
    this.socket.close();
  }

  private probe(operation: 'initial-probe' | 'liveness-probe') {
    const socket = this.socket;
    if (!socket || operation !== 'initial-probe' && socket.readyState !== WebSocket.OPEN) return;
    this.cancel('probe');
    try {
      socket.send('{"type":"probe"}');
      this.schedule('probe', 10_000, () => {
        if (this.socket !== socket) return;
        this.transition('connecting');
        socket.close();
      });
    } catch (error) {
      this.report(error, 'live.websocket_send_failed', operation);
      if (operation === 'initial-probe') socket.close(); else this.reconnect();
    }
  }

  private wake = () => {
    if (!this.available() || navigator.onLine === false) return;
    this.resetBackoff();
    if (!this.socket) { this.connect(); return; }
    if (this.socket.readyState === WebSocket.OPEN) {
      if (this.timestamps.message === null || Date.now() - this.timestamps.message >= 90_000) this.reconnect();
      else this.probe('liveness-probe');
    } else if (this.socket.readyState === WebSocket.CONNECTING && this.timestamps.created !== null && Date.now() - this.timestamps.created >= 10_000) this.reconnect();
  };

  private offline = () => {
    this.cancel('reconnect');
    this.transition('connecting');
    if (this.socket && (this.socket.readyState !== WebSocket.CLOSED && this.socket.readyState !== WebSocket.CLOSING)) this.socket.close();
  };

  private checkLiveness() {
    if (!this.available() || document.visibilityState !== 'visible') return;
    if (!this.socket) { if (!this.timers.has('reconnect')) this.connect(); return; }
    const last = this.timestamps.message ?? this.timestamps.opened;
    if (last !== null && Date.now() - last >= 90_000) this.reconnect();
  }

  private report(error: unknown, event: string, operation: string) {
    reportClientError(error, { event, area: 'websocket', operation, severity: 'warn' });
  }
}

function debugFrame(vehicleId: string, kind: string, raw: unknown, patch: Partial<VehicleStatus> | null, existing: VehicleStatus | undefined) {
  let enabled = false;
  try { enabled = typeof localStorage !== 'undefined' && localStorage.getItem('rm-ws-debug') === '1'; } catch { /* Optional diagnostics cannot interrupt telemetry. */ }
  if (!enabled) return;
  const prior = existing as unknown as Record<string, unknown> | undefined;
  const dropouts = !prior || !patch ? [] : Object.entries(patch).flatMap(([key, value]) => prior[key] != null && value == null ? [`${key}: ${JSON.stringify(prior[key])} → ${String(value)}`] : []);
  console.group(`[WS ${vehicleId}] ${kind} @ ${new Date().toISOString()}`);
  console.log('raw   :', raw);
  console.log('patch :', patch);
  if (dropouts.length) console.warn('would-dropout:', dropouts);
  console.groupEnd();
}
