import { useId } from 'react';
import { useCurrentVehicleStatus } from '@panda/hooks';
import { TbCarSuvFilled } from 'react-icons/tb';
import { Button } from '@panda/ui/primitives';
import { ArrowRight } from 'lucide-react';

interface ConnectedVehicleSuccessProps {
  vehicleId: string;
  vehicleName: string;
  initialWaitingMessage?: string | null;
  onOpenDashboard: () => void;
}

const HILLS_LINE =
  'M-10,65 C20,65 50,25 75,25 C100,25 125,65 148,65 C165,65 195,16 220,16 C245,16 260,64 280,64 C298,64 320,35 342,35 C360,35 380,56 410,52';
const HILLS_FILL = `${HILLS_LINE} L410,80 L-10,80 Z`;
const MOTION_DURATION = '2.2s';

export function ConnectedVehicleSuccess({
  vehicleId,
  vehicleName,
  initialWaitingMessage,
  onOpenDashboard,
}: ConnectedVehicleSuccessProps) {
  const hillPathId = `rm-success-hills-${useId().replace(/:/g, '')}`;
  const { data: status } = useCurrentVehicleStatus(vehicleId);
  const readiness = connectionReadiness(status?.worker_health, status?.last_event_at || status?.last_payload_at);
  const { title, message: readinessMessage } = {
    ready: { title: 'Vehicle connected', message: `${vehicleName} is connected and telemetry is available.` },
    attention: { title: 'Vehicle saved', message: `${vehicleName} was saved, but its telemetry collector needs attention. Open the dashboard to review vehicle health.` },
    waiting: { title: 'Vehicle saved', message: initialWaitingMessage ?? `${vehicleName} was saved. Panda is starting its telemetry collector; a sleeping vehicle may take a little while to report.` },
  }[readiness];
  const telemetryAvailable = readiness === 'ready';

  return (
    <div className="py-3 text-center">
      <div className="relative mx-auto h-24 max-w-sm overflow-hidden rounded-xl border border-border bg-bg-elevated/40">
        <svg
          viewBox="0 0 400 80"
          preserveAspectRatio="none"
          className="absolute inset-0 h-full w-full"
          aria-hidden="true"
        >
          <path d={HILLS_FILL} fill="var(--rm-accent)" fillOpacity="0.08" />
          <path
            id={hillPathId}
            d={HILLS_LINE}
            fill="none"
            stroke="var(--rm-accent)"
            strokeOpacity="0.25"
            strokeWidth="1.5"
          />

          <g
            className="rm-success-vehicle rm-success-vehicle-animated"
            transform="translate(-16 -29)"
            opacity="0"
          >
            <TbCarSuvFilled
              width="32"
              height="32"
              className="text-accent"
              aria-hidden="true"
              focusable="false"
              style={{
                filter:
                  'drop-shadow(0 1px 6px color-mix(in oklab, var(--rm-accent) 50%, transparent))',
              }}
            />

            <animateMotion
              dur={MOTION_DURATION}
              calcMode="paced"
              rotate="auto"
              repeatCount="indefinite"
            >
              <mpath href={`#${hillPathId}`} />
            </animateMotion>
            <animate
              attributeName="opacity"
              dur={MOTION_DURATION}
              keyTimes="0;0.12;0.82;1"
              repeatCount="indefinite"
              values="0;1;1;0"
            />
          </g>

          <g
            className="rm-success-vehicle rm-success-vehicle-static"
            transform="translate(200 25) translate(-16 -29)"
          >
            <TbCarSuvFilled
              width="32"
              height="32"
              className="text-accent"
              aria-hidden="true"
              focusable="false"
              style={{
                filter:
                  'drop-shadow(0 1px 6px color-mix(in oklab, var(--rm-accent) 50%, transparent))',
              }}
            />
          </g>
        </svg>
      </div>

      <div className="mx-auto mt-7 max-w-sm" aria-live="polite">
        <p className="font-display text-xl font-semibold text-fg">{title}</p>
        <p className={`mt-2 text-sm leading-6 ${initialWaitingMessage && !telemetryAvailable ? 'text-status-warning' : 'text-fg-secondary'}`}>
          {readinessMessage}
        </p>
      </div>

      <Button
        className="mt-7"
        iconRight={<ArrowRight className="h-4 w-4" />}
        onClick={onOpenDashboard}
      >
        Open Dashboard
      </Button>
    </div>
  );
}

function connectionReadiness(health: string | null | undefined, evidence: unknown): 'ready' | 'attention' | 'waiting' {
  const states = new Map<string, 'ready' | 'attention'>([
    ['connected', 'ready'], ['ok', 'ready'], ['error', 'attention'], ['degraded', 'attention'], ['stale', 'attention'],
  ]);
  const state = states.get(health?.toLowerCase() ?? '') ?? 'waiting';
  return state === 'ready' && !evidence ? 'waiting' : state;
}
