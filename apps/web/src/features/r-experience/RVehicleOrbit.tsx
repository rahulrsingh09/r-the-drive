import React from 'react';
import { RotateCcw, Play } from 'lucide-react';
import { initialOrbitMemory, OrbitController, type OrbitView } from './orbitController';
import heroUrl from './assets/r2-hero.webp';

export function RVehicleOrbit({ vehicleId }: { vehicleId: string }) {
  const canvas = React.useRef<HTMLCanvasElement>(null);
  const controller = React.useRef<OrbitController | null>(null);
  const [memory] = React.useState(() => initialOrbitMemory(vehicleId));
  const [{ angle, ready, loading, error }, updateView] = React.useState<OrbitView>({
    angle: memory.frame, ready: false, loading: false, error: false,
  });
  React.useEffect(() => {
    const session = new OrbitController(vehicleId, memory, canvas.current, patch => {
      updateView(current => ({ ...current, ...patch }));
    });
    controller.current = session;
    return () => session.close();
  }, [vehicleId, memory]);
  return (
    <div className="r-orbit">
      <div className="r-orbit-stage" role="slider" tabIndex={0} aria-label="Rotate vehicle"
        aria-valuemin={0} aria-valuemax={355} aria-valuenow={angle * 5}
        aria-valuetext={`${angle * 5} degrees`} aria-describedby="r-orbit-hint"
        onKeyDown={event => controller.current?.key(event)}
        onPointerDown={event => controller.current?.pointer('down', event)}
        onPointerMove={event => controller.current?.pointer('move', event)}
        onPointerUp={event => controller.current?.pointer('up', event)}
        onPointerCancel={event => controller.current?.pointer('cancel', event)}>
        <img src={heroUrl} alt="Rivian R2 Performance in Catalina Cove with 21-inch wheels" draggable={false} className={ready ? 'r-orbit-poster r-orbit-hidden' : 'r-orbit-poster'} />
        <canvas ref={canvas} width={840} height={430} aria-hidden="true" className={ready ? '' : 'r-orbit-hidden'} />
      </div>
      <div className="r-orbit-controls">
        <span id="r-orbit-hint">{error ? 'Rotation unavailable. Try again.' : loading && !ready ? 'Loading rotation…' : 'Drag to turn · 360°'}</span>
        <button type="button" onClick={() => controller.current?.control('front')}>Front</button>
        <button type="button" aria-label="Reset vehicle view" onClick={() => controller.current?.control('reset')}><RotateCcw /></button>
        <button type="button" aria-label="Replay vehicle rotation" onClick={() => controller.current?.control('replay')}><Play /></button>
      </div>
    </div>
  );
}
