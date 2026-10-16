import React from 'react';
import { RotateCcw, Play } from 'lucide-react';
import { createOrbitFrames, DEFAULT_FRAME, FRONT_FRAME, ORBIT_FRAMES, wrapFrame } from './orbitFrames';
import heroUrl from './assets/r2-hero.webp';

const rememberedAngles = new Map<string, number>();

export function RVehicleOrbit({ vehicleId }: { vehicleId: string }) {
  const canvas = React.useRef<HTMLCanvasElement>(null);
  const frame = React.useRef(rememberedAngles.get(vehicleId) ?? DEFAULT_FRAME);
  const [angle, setAngle] = React.useState(frame.current);
  const [ready, setReady] = React.useState(false);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState(false);
  const motion = React.useRef(0);
  const failed = React.useRef(false);
  const draw = React.useRef<(next: number) => void>(() => {});
  const reduced = React.useRef(false);
  const interacted = React.useRef(false);
  const drag = React.useRef<{ id: number; x: number; y: number; frame: number; time: number; velocity: number; moved: boolean } | null>(null);

  React.useEffect(() => {
    const media = window.matchMedia('(prefers-reduced-motion: reduce)');
    const change = () => {
      reduced.current = media.matches;
      if (media.matches) interacted.current = true;
      cancelAnimationFrame(motion.current);
      setLoading(false);
    };
    change();
    media.addEventListener('change', change);
    const frames = createOrbitFrames();
    let sequence = 0;
    let active = true;
    let rendering = false;
    let requested: number | null = null;
    draw.current = (next) => {
      if (failed.current) return;
      frame.current = next;
      const index = wrapFrame(next);
      rememberedAngles.set(vehicleId, index);
      setAngle(index);
      requested = index;
      if (rendering) return;
      const render = async () => {
        rendering = true;
        setLoading(true);
        const generation = ++sequence;
        try {
          while (requested !== null && active) {
            const target = requested;
            requested = null;
            const image = await frames.get(target);
            if (!active || generation !== sequence) return;
            const context = canvas.current?.getContext('2d');
            if (context) {
              context.clearRect(0, 0, 840, 430);
              context.drawImage(image, 0, 0, 840, 430);
              setReady(true);
              setError(false);
            }
          }
        } catch {
          if (active) {
            requested = null;
            failed.current = true;
            drag.current = null;
            cancelAnimationFrame(motion.current);
            setError(true);
          }
        } finally {
          if (active) setLoading(false);
          rendering = false;
        }
      };
      void render();
    };
    const restore = rememberedAngles.has(vehicleId);
    if (restore) draw.current(frame.current);
    let entered = false;
    const observer = new IntersectionObserver(entries => {
      const visible = entries.some(entry => entry.isIntersecting);
      if (!visible) {
        if (entered) { interacted.current = true; cancelAnimationFrame(motion.current); setLoading(false); }
        return;
      }
      if (entered || restore || reduced.current || interacted.current) return;
      entered = true;
      setLoading(true);
      void frames.get(frame.current).then(() => {
        if (!active || interacted.current || document.hidden) {
          if (active) setLoading(false);
          return;
        }
        const start = frame.current;
        const began = performance.now();
        const tick = (time: number) => {
          if (!active || interacted.current || reduced.current) return;
          const progress = Math.min(1, (time - began) / 3600);
          draw.current(start + ORBIT_FRAMES * (1 - Math.pow(1 - progress, 3)));
          if (progress < 1) motion.current = requestAnimationFrame(tick);
        };
        motion.current = requestAnimationFrame(tick);
      }).catch(() => {
        if (active) { failed.current = true; setError(true); setLoading(false); }
      });
    });
    if (canvas.current) observer.observe(canvas.current.parentElement!);
    const visibility = () => {
      if (document.hidden) { interacted.current = true; cancelAnimationFrame(motion.current); setLoading(false); }
    };
    document.addEventListener('visibilitychange', visibility);
    return () => {
      active = false;
      sequence++;
      cancelAnimationFrame(motion.current);
      observer.disconnect();
      document.removeEventListener('visibilitychange', visibility);
      media.removeEventListener('change', change);
      frames.close();
    };
  }, [vehicleId]);

  function animate(target: number, duration: number) {
    interacted.current = true;
    cancelAnimationFrame(motion.current);
    failed.current = false;
    if (reduced.current) { draw.current(target); return; }
    const start = frame.current;
    const began = performance.now();
    const tick = (time: number) => {
      const progress = Math.min(1, (time - began) / duration);
      draw.current(start + (target - start) * (1 - Math.pow(1 - progress, 3)));
      if (progress < 1) motion.current = requestAnimationFrame(tick);
    };
    motion.current = requestAnimationFrame(tick);
  }

  function finish(event: React.PointerEvent<HTMLDivElement>, cancelled = false) {
    const previous = drag.current;
    if (!previous || previous.id !== event.pointerId) return;
    drag.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    if (!cancelled && previous.moved && !reduced.current) {
      animate(frame.current + Math.max(-12, Math.min(12, previous.velocity * 160)), 500);
    }
  }

  return (
    <div className="r-orbit">
      <div className="r-orbit-stage" role="slider" tabIndex={0} aria-label="Rotate vehicle"
        aria-valuemin={0} aria-valuemax={355} aria-valuenow={angle * 5}
        aria-valuetext={`${angle * 5} degrees`} aria-describedby="r-orbit-hint"
        onKeyDown={event => {
          const next = event.key === 'ArrowRight' ? frame.current + 2
            : event.key === 'ArrowLeft' ? frame.current - 2
            : event.key === 'Home' ? FRONT_FRAME : event.key === 'End' ? DEFAULT_FRAME : null;
          if (next === null) return;
          event.preventDefault(); interacted.current = true; cancelAnimationFrame(motion.current); failed.current = false; draw.current(next);
        }}
        onPointerDown={event => {
          if (!event.isPrimary || event.button !== 0) return;
          interacted.current = true;
          cancelAnimationFrame(motion.current);
          setLoading(false);
          failed.current = false;
          drag.current = { id: event.pointerId, x: event.clientX, y: event.clientY, frame: frame.current, time: performance.now(), velocity: 0, moved: false };
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={event => {
          const previous = drag.current;
          if (!previous || previous.id !== event.pointerId) return;
          const dx = event.clientX - previous.x;
          const dy = event.clientY - previous.y;
          if (!previous.moved && Math.abs(dy) > Math.abs(dx) + 6) { finish(event, true); return; }
          if (!previous.moved && Math.abs(dx) < 6) return;
          previous.moved = true;
          const next = previous.frame + dx / Math.max(5, event.currentTarget.clientWidth / ORBIT_FRAMES);
          const now = performance.now();
          previous.velocity = (next - frame.current) / Math.max(1, now - previous.time);
          previous.time = now;
          draw.current(next);
        }}
        onPointerUp={event => finish(event)} onPointerCancel={event => finish(event, true)}>
        <img src={heroUrl} alt="Rivian R2 Performance in Catalina Cove with 21-inch wheels" draggable={false} className={ready ? 'r-orbit-poster r-orbit-hidden' : 'r-orbit-poster'} />
        <canvas ref={canvas} width={840} height={430} aria-hidden="true" className={ready ? '' : 'r-orbit-hidden'} />
      </div>
      <div className="r-orbit-controls">
        <span id="r-orbit-hint">{error ? 'Rotation unavailable. Try again.' : loading && !ready ? 'Loading rotation…' : 'Drag to turn · 360°'}</span>
        <button type="button" onClick={() => animate(frame.current + ((FRONT_FRAME - wrapFrame(frame.current) + 108) % 72 - 36), 600)}>Front</button>
        <button type="button" aria-label="Reset vehicle view" onClick={() => animate(frame.current + ((DEFAULT_FRAME - wrapFrame(frame.current) + 108) % 72 - 36), 650)}><RotateCcw /></button>
        <button type="button" aria-label="Replay vehicle rotation" onClick={() => animate(frame.current + ORBIT_FRAMES, 2600)}><Play /></button>
      </div>
    </div>
  );
}
