import type { RestoreJob } from '@panda/types';

export interface RestoreObserver {
  read: () => Promise<RestoreJob>;
  job: (job: RestoreJob) => void;
  unavailable: (value: boolean) => void;
  completed: () => void;
}

export function observeRestore(observer: RestoreObserver): () => void {
  let disposed = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const schedule = (callback: () => void, delay: number) => {
    if (!disposed) timer = setTimeout(callback, delay);
  };
  const poll = async () => {
    let terminal = false;
    try {
      const job = await observer.read();
      if (disposed) return;
      observer.unavailable(false);
      observer.job(job);
      terminal = job.phase === 'completed' || job.phase === 'failed';
      if (job.phase === 'completed') schedule(observer.completed, 1200);
    } catch {
      if (!disposed) observer.unavailable(true);
    } finally {
      // Restore replaces the API process; temporary failures remain retryable.
      if (!terminal && !disposed) schedule(() => { void poll(); }, 1000);
    }
  };
  void poll();
  return () => {
    disposed = true;
    clearTimeout(timer);
  };
}
