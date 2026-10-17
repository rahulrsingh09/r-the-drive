/** Serialize rotating-cookie requests across tabs without persisting tokens. */
const LOCK_NAME = 'panda-session-renewal';
const LEASE_MS = 120_000;
let tail = Promise.resolve();

class RenewalLease {
  constructor(readonly database: IDBDatabase, readonly owner: string) {}

  static open(owner: string): Promise<RenewalLease> {
    return new Promise((resolve, reject) => {
      const opening = indexedDB.open('panda-session-coordination', 1);
      const handlers = {
        upgradeneeded: () => { opening.result.createObjectStore('leases'); },
        success: () => resolve(new RenewalLease(opening.result, owner)),
        error: () => reject(opening.error),
        blocked: () => reject(new Error('Session coordination database is blocked')),
      };
      Object.assign(opening, { onupgradeneeded: handlers.upgradeneeded, onsuccess: handlers.success, onerror: handlers.error, onblocked: handlers.blocked });
    });
  }

  change(action: 'claim' | 'release'): Promise<boolean> {
    return new Promise((resolve, reject) => {
      const transaction = this.database.transaction('leases', 'readwrite');
      const records = transaction.objectStore('leases');
      const read = records.get(LOCK_NAME);
      let claimed = false;
      transaction.oncomplete = () => resolve(claimed);
      transaction.onerror = () => reject(transaction.error);
      transaction.onabort = () => reject(transaction.error ?? new Error('Session coordination transaction aborted'));
      read.onsuccess = () => {
        const current = read.result as { owner: string; expires: number } | undefined;
        switch (action) {
          case 'release':
            if (current?.owner === this.owner) records.delete(LOCK_NAME);
            break;
          case 'claim':
            if (current && !(current.expires <= Date.now())) break;
            records.put({ owner: this.owner, expires: Date.now() + LEASE_MS }, LOCK_NAME);
            claimed = true;
        }
      };
    });
  }

  async run<T>(operation: () => Promise<T>): Promise<T> {
    const deadline = Date.now() + LEASE_MS;
    for (;;) {
      if (await this.change('claim')) break;
      if (Date.now() >= deadline) throw new Error('Session renewal is busy. Please retry.');
      await new Promise((resume) => setTimeout(resume, 250));
    }
    try { return await operation(); }
    finally { await this.change('release'); }
  }
}

export async function withSessionLock<T>(operation: () => Promise<T>): Promise<T> {
  if (typeof navigator !== 'undefined' && navigator.locks?.request) return navigator.locks.request(LOCK_NAME, { mode: 'exclusive' }, operation);
  if (typeof indexedDB !== 'undefined') {
    const owner = crypto.randomUUID();
    let lease: RenewalLease | undefined;
    try {
      lease = await RenewalLease.open(owner);
      return await lease.run(operation);
    } catch (cause) {
      if (cause instanceof Error && 'detail' in cause) throw cause;
      throw Object.assign(new Error('Unable to coordinate session renewal. Please try again.'), { code: 'NETWORK_ERROR', cause });
    } finally { lease?.database.close(); }
  }
  const browser = typeof window !== 'undefined' && typeof window.document !== 'undefined';
  if (browser && !/jsdom/i.test(navigator.userAgent)) throw Object.assign(new Error('Session coordination is unavailable. Please try again.'), { code: 'NETWORK_ERROR' });
  const previous = tail;
  let release!: () => void;
  tail = new Promise<void>((resolve) => { release = resolve; });
  await previous;
  try { return await operation(); }
  finally { release(); }
}
