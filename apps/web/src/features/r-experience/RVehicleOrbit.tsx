import React from 'react';
import { useQuery } from '@tanstack/react-query';
import type { Vehicle, VehicleImages } from '@panda/types';
import {
  api, AuthenticatedVehicleArtwork, normalizeVehicleArtworkModel, queryKeys, readStoredVehicleImages,
  resolveVehicleArtwork, storeVehicleImages, useAuthReady, useVehicleArtwork, type ResolvedVehicleArtwork,
} from '@panda/hooks';
import { useDocumentTheme } from '@panda/ui/hooks';
import {
  angleForKey, angleStepWidth, createOrbitFrames, DEFAULT_FRAME, FRONT_FRAME, ORBIT_FRAMES,
  restAngleIndex, settleAngle, wrapAngleIndex, wrapFrame,
} from './orbitFrames';
import heroUrl from './assets/r2-hero.webp';
import './r-orbit.css';

const DOUBLE_TAP_MS = 320;
const DOUBLE_TAP_PX = 24;

type VehicleAngleImage = NonNullable<ResolvedVehicleArtwork['angles']>['light'][number];

export interface RVehicleAnglesProps {
  /** Accessible name of the turn control, for example "Your R2, Catalina Cove. Use arrow keys to turn." */
  label: string;
  /** The owner's own Rivian images in turning order (at least two). */
  angles: readonly VehicleAngleImage[];
  /** Packaged model artwork for the side image when the owner image cannot load. */
  fallbackSource?: string | null;
}

/** The accessible name of the still image: the turn label without the key hint. */
function stillLabel(label: string) {
  return label.replace(/\.?\s*Use arrow keys to turn\.?\s*$/, '').trim() || label;
}

/**
 * Drops the owner angles whose image fails to load. The side image is kept: it has
 * the packaged model artwork as its fallback.
 */
function useUsableAngles(angles: readonly VehicleAngleImage[]) {
  const [failed, setFailed] = React.useState<ReadonlySet<string>>(() => new Set());
  const markFailed = React.useCallback((url: string) => {
    setFailed(current => (current.has(url) ? current : new Set(current).add(url)));
  }, []);
  const usable = React.useMemo(
    () => angles.filter(angle => angle.angle === 'side' || !failed.has(angle.url)),
    [angles, failed],
  );
  return { usable, markFailed };
}

/** One angle of the turn. A protected image that cannot be fetched, or a browser image error, reports the angle as failed. */
function TurnAngle({ angle, active, fallbackSource, onFailed }: {
  angle: VehicleAngleImage;
  active: boolean;
  fallbackSource: string | null;
  onFailed: (url: string) => void;
}) {
  // Same query as AuthenticatedVehicleArtwork (shared cache): no second request.
  const { isError } = useVehicleArtwork(angle.url);
  React.useEffect(() => {
    if (isError && !fallbackSource) onFailed(angle.url);
  }, [angle.url, fallbackSource, isError, onFailed]);
  return (
    <AuthenticatedVehicleArtwork source={angle.url} fallbackSource={fallbackSource}
      className="r-orbit-angle" data-angle={angle.angle} data-active={active || undefined}
      onError={fallbackSource ? undefined : () => onFailed(angle.url)}
      alt="" aria-hidden="true" draggable={false} />
  );
}

/**
 * The owner's own Rivian images as a turn control. The car rests on the side image.
 * A drag or the arrow keys step through the angles with a short cross-fade; a released
 * drag rounds to the nearest angle and the set wraps around. A double-tap or Home
 * returns to the side image. Reduced motion shows each angle at once (no fade, no inertia).
 */
export function RVehicleAngles({ label, angles: ownerAngles, fallbackSource }: RVehicleAnglesProps) {
  // An angle whose image fails is dropped from the turn set, so no stop is blank.
  const { usable: angles, markFailed } = useUsableAngles(ownerAngles);
  const count = angles.length;
  const rest = restAngleIndex(angles);
  const [index, setIndex] = React.useState(rest);
  const current = React.useRef(rest);
  const lastTap = React.useRef<{ time: number; x: number; y: number } | null>(null);
  const drag = React.useRef<{ id: number; x: number; y: number; start: number; position: number; time: number; velocity: number; moved: boolean } | null>(null);
  const reduced = React.useRef(false);

  React.useEffect(() => {
    const media = window.matchMedia('(prefers-reduced-motion: reduce)');
    const change = () => { reduced.current = media.matches; };
    change();
    media.addEventListener('change', change);
    return () => media.removeEventListener('change', change);
  }, []);

  const show = React.useCallback((next: number) => {
    const wrapped = wrapAngleIndex(next, count);
    current.current = wrapped;
    setIndex(wrapped);
  }, [count]);

  // A new angle set (theme or vehicle change) rests on its side image again.
  const signature = angles.map(angle => angle.url).join('|');
  React.useEffect(() => { show(rest); }, [signature, rest, show]);

  function finish(event: React.PointerEvent<HTMLDivElement>, cancelled = false) {
    const previous = drag.current;
    if (!previous || previous.id !== event.pointerId) return;
    drag.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    if (previous.moved) {
      show(settleAngle(previous.position, cancelled || reduced.current ? 0 : previous.velocity, count));
      return;
    }
    if (cancelled) return;
    const tap = lastTap.current;
    const now = performance.now();
    if (tap && now - tap.time < DOUBLE_TAP_MS && Math.hypot(event.clientX - tap.x, event.clientY - tap.y) < DOUBLE_TAP_PX) {
      lastTap.current = null;
      show(rest);
    } else {
      lastTap.current = { time: now, x: event.clientX, y: event.clientY };
    }
  }

  const active = angles[index] ?? angles[0];
  // With fewer than 2 usable angles there is nothing to turn: show the side image alone.
  if (count < 2 || !active) {
    const still = angles[rest] ?? angles[0] ?? ownerAngles.find(angle => angle.angle === 'side');
    return (
      <div className="r-orbit r-orbit-turn" data-mode="still">
        <div className="r-orbit-stage">
          <AuthenticatedVehicleArtwork source={still?.url ?? null} fallbackSource={fallbackSource}
            className="r-orbit-angle" data-angle={still?.angle ?? 'side'} data-active
            alt={stillLabel(label)} draggable={false} />
        </div>
      </div>
    );
  }
  return (
    <div className="r-orbit r-orbit-turn">
      <div className="r-orbit-stage" role="slider" tabIndex={0} aria-label={label}
        aria-valuemin={0} aria-valuemax={count - 1} aria-valuenow={index} aria-valuetext={active.label}
        data-angle={active.angle}
        onKeyDown={event => {
          const next = angleForKey(event.key, current.current, count, rest);
          if (next === null) return;
          event.preventDefault();
          show(next);
        }}
        onPointerDown={event => {
          if (!event.isPrimary || event.button !== 0) return;
          drag.current = { id: event.pointerId, x: event.clientX, y: event.clientY, start: current.current, position: current.current, time: performance.now(), velocity: 0, moved: false };
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
          const position = previous.start + dx / angleStepWidth(event.currentTarget.clientWidth);
          const now = performance.now();
          previous.velocity = (position - previous.position) / Math.max(1, now - previous.time);
          previous.position = position;
          previous.time = now;
          if (wrapAngleIndex(position, count) !== current.current) show(position);
        }}
        onPointerUp={event => finish(event)} onPointerCancel={event => finish(event, true)}>
        {angles.map((angle, position) => (
          <TurnAngle key={angle.url} angle={angle} active={position === index}
            fallbackSource={angle.angle === 'side' ? fallbackSource ?? null : null} onFailed={markFailed} />
        ))}
      </div>
    </div>
  );
}


/** The first-view glide: the car starts this many frames (5° each) before its rest view and settles on it. */
export const ORBIT_GLIDE_FRAMES = 4;
const GLIDE_MS = 1500;
const RELEASE_MS = 450;
const KEY_STEP = 2;
const KEY_MS = 160;
const TOWARD_MS = 600;
/** Frames per drag width: one full drag across the stage turns the car 60 frames (300°). */
const FRAMES_PER_WIDTH = 60;
/** A release carries on for at most this many frames. */
const MAX_CARRY = 8;

const rememberedFrames = new Map<string, number>();
const glided = new Set<string>();

function prefersReducedMotion() {
  return typeof window !== 'undefined' && Boolean(window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);
}

export interface RVehicleOrbitProps {
  vehicleId: string;
  /** Accessible name of the turn control. */
  label?: string;
  /** The 20° settle glide on the first view of this vehicle in the page session. Off for reduced motion. */
  glide?: boolean;
  /** The owner starts to turn the car (drag, trackpad or key). */
  onTurnStart?: () => void;
  /** The car stops after a turn, at this frame. */
  onRest?: (frame: number) => void;
}

export const ORBIT_LABEL = 'Turn the car. Drag it, or use the left and right arrow keys.';

/**
 * The bundled 72-frame turn (5° per frame). Drag only: no buttons and no dial.
 * - First view: a 20° settle glide onto the rest view, once per vehicle and page session.
 * - Pointer drag (mouse, touch, pen) with a small inertia on release.
 * - Horizontal trackpad scroll turns the car; vertical scroll moves the page.
 * - Left and Right arrow keys step 10°; Home shows the front, End the rest view.
 * - Reduced motion: no glide and no inertia; keys jump.
 */
export function RVehicleOrbit({ vehicleId, label = ORBIT_LABEL, glide = true, onTurnStart, onRest }: RVehicleOrbitProps) {
  const callbacks = React.useRef({ onTurnStart, onRest });
  callbacks.current = { onTurnStart, onRest };
  const stage = React.useRef<HTMLDivElement>(null);
  const canvas = React.useRef<HTMLCanvasElement>(null);
  const startFrame = () => {
    const remembered = rememberedFrames.get(vehicleId);
    if (remembered != null) return remembered;
    return glide && !glided.has(vehicleId) && !prefersReducedMotion() ? DEFAULT_FRAME - ORBIT_GLIDE_FRAMES : DEFAULT_FRAME;
  };
  const frame = React.useRef(startFrame());
  const [angle, setAngle] = React.useState(() => wrapFrame(frame.current));
  const [ready, setReady] = React.useState(false);
  const [error, setError] = React.useState(false);
  const motion = React.useRef(0);
  const draw = React.useRef<(next: number) => void>(() => {});
  const drag = React.useRef<{ id: number; x: number; frame: number; time: number; velocity: number; moved: boolean } | null>(null);
  const [grabbing, setGrabbing] = React.useState(false);

  React.useEffect(() => {
    const frames = createOrbitFrames();
    let active = true;
    let rendering = false;
    let requested: number | null = null;
    draw.current = (next) => {
      frame.current = next;
      const index = wrapFrame(next);
      rememberedFrames.set(vehicleId, index);
      setAngle(index);
      requested = index;
      if (rendering) return;
      rendering = true;
      void (async () => {
        try {
          while (requested !== null && active) {
            const target = requested;
            requested = null;
            const image = await frames.get(target);
            if (!active) return;
            const context = canvas.current?.getContext('2d');
            if (context) {
              context.clearRect(0, 0, 840, 430);
              context.drawImage(image, 0, 0, 840, 430);
              setReady(true);
              setError(false);
            }
          }
        } catch {
          if (active) { requested = null; setError(true); }
        } finally {
          rendering = false;
        }
      })();
    };
    draw.current(frame.current);

    let observer: IntersectionObserver | null = null;
    if (glide && !glided.has(vehicleId) && !prefersReducedMotion() && typeof IntersectionObserver !== 'undefined' && stage.current) {
      observer = new IntersectionObserver(entries => {
        if (!entries.some(entry => entry.isIntersecting) || glided.has(vehicleId)) return;
        glided.add(vehicleId);
        observer?.disconnect();
        void frames.get(DEFAULT_FRAME).catch(() => null).then(() => {
          if (active && !drag.current) animate(frame.current + towardOffset(DEFAULT_FRAME), GLIDE_MS, false);
        });
      });
      observer.observe(stage.current);
    } else {
      glided.add(vehicleId);
    }
    return () => {
      active = false;
      cancelAnimationFrame(motion.current);
      observer?.disconnect();
      frames.close();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- one frame source per vehicle
  }, [vehicleId]);

  // Horizontal trackpad scroll. A native listener, because React wheel listeners are passive.
  React.useEffect(() => {
    const element = stage.current;
    if (!element) return;
    const wheel = (event: WheelEvent) => {
      if (Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return;
      event.preventDefault();
      cancelAnimationFrame(motion.current);
      glided.add(vehicleId);
      draw.current(frame.current + event.deltaX / 14);
    };
    element.addEventListener('wheel', wheel, { passive: false });
    return () => element.removeEventListener('wheel', wheel);
  }, [vehicleId]);

  function towardOffset(target: number) {
    return ((target - wrapFrame(frame.current) + ORBIT_FRAMES * 1.5) % ORBIT_FRAMES) - ORBIT_FRAMES / 2;
  }

  function animate(target: number, duration: number, notify = true) {
    cancelAnimationFrame(motion.current);
    const rest = () => { if (notify) callbacks.current.onRest?.(wrapFrame(frame.current)); };
    if (prefersReducedMotion()) { draw.current(target); rest(); return; }
    const from = frame.current;
    const began = performance.now();
    const tick = (time: number) => {
      const progress = Math.min(1, (time - began) / duration);
      draw.current(from + (target - from) * (1 - Math.pow(1 - progress, 3)));
      if (progress < 1) motion.current = requestAnimationFrame(tick);
      else rest();
    };
    motion.current = requestAnimationFrame(tick);
  }

  function finish(event: React.PointerEvent<HTMLDivElement>) {
    const previous = drag.current;
    if (!previous || previous.id !== event.pointerId) return;
    drag.current = null;
    setGrabbing(false);
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
    if (!previous.moved) return;
    const carry = prefersReducedMotion() ? 0 : Math.max(-MAX_CARRY, Math.min(MAX_CARRY, previous.velocity * 120));
    animate(frame.current + carry, RELEASE_MS);
  }

  return (
    <div className="r-orbit r-orbit-pack">
      <div ref={stage} className="r-orbit-stage" role="slider" tabIndex={0} aria-label={label}
        aria-valuemin={0} aria-valuemax={355} aria-valuenow={angle * 5} aria-valuetext={`${angle * 5} degrees`}
        data-grabbing={grabbing || undefined} data-ready={ready || undefined}
        onKeyDown={event => {
          const step = event.key === 'ArrowRight' ? KEY_STEP : event.key === 'ArrowLeft' ? -KEY_STEP : 0;
          const target = event.key === 'Home' ? FRONT_FRAME : event.key === 'End' ? DEFAULT_FRAME : null;
          if (!step && target === null) return;
          event.preventDefault();
          glided.add(vehicleId);
          callbacks.current.onTurnStart?.();
          if (step) animate(frame.current + step, KEY_MS);
          else animate(frame.current + towardOffset(target!), TOWARD_MS);
        }}
        onPointerDown={event => {
          if (!event.isPrimary || event.button !== 0) return;
          cancelAnimationFrame(motion.current);
          glided.add(vehicleId);
          drag.current = { id: event.pointerId, x: event.clientX, frame: frame.current, time: performance.now(), velocity: 0, moved: false };
          setGrabbing(true);
          event.currentTarget.setPointerCapture?.(event.pointerId);
        }}
        onPointerMove={event => {
          const previous = drag.current;
          if (!previous || previous.id !== event.pointerId) return;
          const dx = event.clientX - previous.x;
          if (!previous.moved && Math.abs(dx) < 4) return;
          if (!previous.moved) callbacks.current.onTurnStart?.();
          previous.moved = true;
          const next = previous.frame - dx / Math.max(1, event.currentTarget.clientWidth / FRAMES_PER_WIDTH);
          const now = performance.now();
          previous.velocity = (next - frame.current) / Math.max(1, now - previous.time);
          previous.time = now;
          draw.current(next);
        }}
        onPointerUp={finish} onPointerCancel={finish}>
        <span className="r-orbit-floor" aria-hidden="true" />
        <img src={heroUrl} alt="" draggable={false} className={ready ? 'r-orbit-poster r-orbit-hidden' : 'r-orbit-poster'} />
        <canvas ref={canvas} width={840} height={430} aria-hidden="true" className={ready ? '' : 'r-orbit-hidden'} />
      </div>
      <span className="sr-only" role="status">{error ? 'The car turn could not load. Drag or press an arrow key to try again.' : ''}</span>
    </div>
  );
}

/** The owner's own angle images for the theme, in turning order. */
export function useOwnerAngles(vehicle: Pick<Vehicle, 'id' | 'model' | 'images'>) {
  const authReady = useAuthReady();
  const dark = useDocumentTheme();
  // The image list kept from an earlier visit shows the owner's car at once; the server list replaces it.
  const stored = React.useMemo(() => readStoredVehicleImages<VehicleImages>(vehicle.id), [vehicle.id]);
  const images = useQuery({
    queryKey: queryKeys.vehicle.images(vehicle.id),
    queryFn: () => api.vehicleImages(vehicle.id),
    enabled: authReady,
    initialData: stored ?? undefined,
    initialDataUpdatedAt: 0,
    staleTime: 10 * 60 * 1000,
    retry: 1,
    refetchOnWindowFocus: false,
    // A mirror that is still filling (the server answers 202) is asked again until it is ready.
    refetchInterval: query => {
      const status = query.state.data?.cache?.status;
      return status && status !== 'ready' ? 5_000 : false;
    },
  });
  const cache = images.data?.cache?.status;
  React.useEffect(() => {
    if (images.data && images.dataUpdatedAt > 0 && (!cache || cache === 'ready')) storeVehicleImages(vehicle.id, images.data);
  }, [cache, images.data, images.dataUpdatedAt, vehicle.id]);
  // The mirrored images from the server first; the images on the vehicle record when the server has fewer than two angles.
  const pick = (source: Vehicle['images'] | undefined) => {
    const artwork = resolveVehicleArtwork(source, vehicle.model, 'vehicle-card');
    return { angles: (dark ? artwork.angles?.dark : artwork.angles?.light) ?? [], fallback: artwork.fallback };
  };
  // No image list yet: the first request of the first visit is still out (or waits for sign-in).
  const pending = images.isPending;
  if (images.data && cache && cache !== 'ready') return { ...pick(null), pending: true };
  const served = pick(images.data);
  return { ...(served.angles.length >= 2 ? served : pick(vehicle.images)), pending };
}

/**
 * A neutral stand-in of the turn's size, used while the owner's images load or when they
 * are not available. It never shows another model.
 */
export function RVehiclePlaceholder({ unavailable = false }: { unavailable?: boolean }) {
  return (
    <div className="r-orbit r-orbit-turn" data-mode={unavailable ? 'unavailable' : 'loading'}>
      <div className="r-orbit-stage r-orbit-placeholder" role="img"
        aria-label={unavailable ? 'Your car image is not available yet' : 'Loading your car'}>
        <span className="r-orbit-floor" aria-hidden="true" />
        <span className="r-orbit-shape" aria-hidden="true" />
      </div>
    </div>
  );
}

/**
 * The car turn for one vehicle. With at least two of the owner's own Rivian angle images
 * (served from the server's mirror, or kept in this browser from an earlier visit), the turn
 * cross-fades between them.
 * - An R2 never uses the bundled 72-frame turn, which shows an R1S body: while its images
 *   load it shows a neutral placeholder, and with one image it shows that image alone.
 * - Other models keep the bundled turn when they have no owner images.
 */
export function RVehicleTurn({ vehicle, label = ORBIT_LABEL, glide = true }: {
  vehicle: Pick<Vehicle, 'id' | 'model' | 'images'>; label?: string; glide?: boolean;
}) {
  const { angles, fallback, pending } = useOwnerAngles(vehicle);
  if (angles.length >= 2) return <RVehicleAngles key={vehicle.id} label={label} angles={angles} fallbackSource={fallback} />;
  if (normalizeVehicleArtworkModel(vehicle.model) !== 'r2') {
    return <RVehicleOrbit key={vehicle.id} vehicleId={vehicle.id} label={label} glide={glide} />;
  }
  if (pending) return <RVehiclePlaceholder />;
  if (angles.length === 1) return <RVehicleAngles key={vehicle.id} label={label} angles={angles} fallbackSource={null} />;
  return <RVehiclePlaceholder unavailable />;
}
