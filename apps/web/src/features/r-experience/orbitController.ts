import { createOrbitFrames, DEFAULT_FRAME, FRONT_FRAME, ORBIT_FRAMES, wrapFrame } from './orbitFrames';

const anglesByVehicle = new Map<string, number>();
type Drag = { id: number; originX: number; originY: number; originFrame: number; sampledAt: number; speed: number; moved: boolean };
export type OrbitMemory = { frame: number; interacted: boolean; failed: boolean; drag: Drag | null };
export type OrbitView = { angle: number; ready: boolean; loading: boolean; error: boolean };
type Pointer = {
  pointerId: number; isPrimary: boolean; button: number; clientX: number; clientY: number;
  currentTarget: HTMLDivElement;
};

export function initialOrbitMemory(vehicleId: string): OrbitMemory {
  return { frame: anglesByVehicle.get(vehicleId) ?? DEFAULT_FRAME, interacted: false, failed: false, drag: null };
}

export class OrbitController {
  private frames = createOrbitFrames();
  private media = window.matchMedia('(prefers-reduced-motion: reduce)');
  private observer: IntersectionObserver;
  private alive = true;
  private entered = false;
  private restored: boolean;
  private animation = 0;
  private painting = false;
  private wanted: number | null = null;

  constructor(
    private vehicleId: string,
    private memory: OrbitMemory,
    private canvas: HTMLCanvasElement | null,
    private notify: (patch: Partial<OrbitView>) => void,
  ) {
    this.restored = anglesByVehicle.has(vehicleId);
    this.changeMotion();
    this.media.addEventListener('change', this.changeMotion);
    this.observer = new IntersectionObserver(this.observe);
    if (canvas?.parentElement) this.observer.observe(canvas.parentElement);
    document.addEventListener('visibilitychange', this.visibility);
    if (this.restored) this.request(memory.frame);
  }

  private stop() {
    cancelAnimationFrame(this.animation);
  }

  private changeMotion = () => {
    this.memory.interacted ||= this.media.matches;
    this.stop();
    this.notify({ loading: false });
  };

  private visibility = () => {
    if (!document.hidden) return;
    this.memory.interacted = true;
    this.stop();
    this.notify({ loading: false });
  };

  private observe = (entries: IntersectionObserverEntry[]) => {
    const visible = entries.reduce((found, entry) => found || entry.isIntersecting, false);
    if (!visible) {
      if (this.entered) {
        this.memory.interacted = true;
        this.stop();
        this.notify({ loading: false });
      }
      return;
    }
    if (this.entered || this.restored || this.media.matches || this.memory.interacted) return;
    this.entered = true;
    this.notify({ loading: true });
    void this.warmup();
  };

  private async warmup() {
    try {
      await this.frames.get(this.memory.frame);
      if (!this.alive) return;
      if (this.memory.interacted || document.hidden) {
        this.notify({ loading: false });
        return;
      }
      this.timeline(this.memory.frame + ORBIT_FRAMES, 3600, true);
    } catch {
      if (!this.alive) return;
      this.memory.failed = true;
      this.notify({ error: true, loading: false });
    }
  }

  private request(frame: number) {
    if (!this.alive || this.memory.failed) return;
    this.memory.frame = frame;
    const angle = wrapFrame(frame);
    anglesByVehicle.set(this.vehicleId, angle);
    this.notify({ angle });
    this.wanted = angle;
    if (!this.painting) this.consume();
  }

  private consume() {
    if (!this.alive || this.wanted === null) return;
    const target = this.wanted;
    this.wanted = null;
    this.painting = true;
    this.notify({ loading: true });
    void this.frames.get(target).then(bitmap => {
      if (!this.alive) return;
      const context = this.canvas?.getContext('2d');
      if (context) {
        context.clearRect(0, 0, 840, 430);
        context.drawImage(bitmap, 0, 0, 840, 430);
        this.notify({ ready: true, error: false });
      }
    }).catch(() => {
      if (!this.alive) return;
      this.wanted = null;
      this.memory.failed = true;
      this.memory.drag = null;
      this.stop();
      this.notify({ error: true });
    }).finally(() => {
      this.painting = false;
      if (!this.alive) return;
      if (this.wanted !== null) this.consume();
      else this.notify({ loading: false });
    });
  }

  private timeline(target: number, duration: number, entrance = false) {
    const origin = this.memory.frame;
    const began = performance.now();
    const step: FrameRequestCallback = time => {
      if (!this.alive || entrance && (this.memory.interacted || this.media.matches)) return;
      const progress = Math.min(1, (time - began) / duration);
      const eased = 1 - (1 - progress) ** 3;
      this.request(origin + (target - origin) * eased);
      if (progress < 1) this.animation = requestAnimationFrame(step);
    };
    this.animation = requestAnimationFrame(step);
  }

  private takeControl() {
    this.memory.interacted = true;
    this.memory.failed = false;
    this.stop();
  }

  private animate(target: number, duration: number) {
    this.takeControl();
    if (this.media.matches) this.request(target);
    else this.timeline(target, duration);
  }

  control(command: 'front' | 'reset' | 'replay') {
    const { frame } = this.memory;
    if (command === 'replay') {
      this.animate(frame + ORBIT_FRAMES, 2600);
      return;
    }
    const target = command === 'front' ? FRONT_FRAME : DEFAULT_FRAME;
    const distance = (target - wrapFrame(frame) + ORBIT_FRAMES * 1.5) % ORBIT_FRAMES - ORBIT_FRAMES / 2;
    this.animate(frame + distance, command === 'front' ? 600 : 650);
  }

  key(event: { key: string; preventDefault: () => void }) {
    let target: number;
    switch (event.key) {
      case 'ArrowRight': target = this.memory.frame + 2; break;
      case 'ArrowLeft': target = this.memory.frame - 2; break;
      case 'Home': target = FRONT_FRAME; break;
      case 'End': target = DEFAULT_FRAME; break;
      default: return;
    }
    event.preventDefault();
    this.takeControl();
    this.request(target);
  }

  pointer(action: 'down' | 'move' | 'up' | 'cancel', event: Pointer) {
    if (action === 'down') {
      if (!event.isPrimary || event.button !== 0) return;
      this.takeControl();
      this.notify({ loading: false });
      this.memory.drag = {
        id: event.pointerId, originX: event.clientX, originY: event.clientY,
        originFrame: this.memory.frame, sampledAt: performance.now(), speed: 0, moved: false,
      };
      event.currentTarget.setPointerCapture(event.pointerId);
      return;
    }
    const drag = this.memory.drag;
    if (!drag || event.pointerId !== drag.id) return;
    if (action !== 'move') {
      this.release(event, action === 'cancel');
      return;
    }
    const horizontal = event.clientX - drag.originX;
    const vertical = event.clientY - drag.originY;
    if (!drag.moved) {
      if (Math.abs(vertical) > Math.abs(horizontal) + 6) {
        this.release(event, true);
        return;
      }
      if (Math.abs(horizontal) < 6) return;
      drag.moved = true;
    }
    const frame = drag.originFrame + horizontal / Math.max(5, event.currentTarget.clientWidth / ORBIT_FRAMES);
    const sampledAt = performance.now();
    drag.speed = (frame - this.memory.frame) / Math.max(1, sampledAt - drag.sampledAt);
    drag.sampledAt = sampledAt;
    this.request(frame);
  }

  private release(event: Pointer, cancelled: boolean) {
    const drag = this.memory.drag!;
    this.memory.drag = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    if (cancelled || !drag.moved || this.media.matches) return;
    const coast = Math.max(-12, Math.min(12, drag.speed * 160));
    this.animate(this.memory.frame + coast, 500);
  }

  close() {
    this.alive = false;
    this.stop();
    this.observer.disconnect();
    document.removeEventListener('visibilitychange', this.visibility);
    this.media.removeEventListener('change', this.changeMotion);
    this.frames.close();
  }
}
