import React, { useEffect, useState } from 'react';
import { formatAppDateTime } from '@panda/ui/lib/dateTime';
import { useQuery } from '@tanstack/react-query';
import { GiWeight } from 'react-icons/gi';
import {
  MdSignalWifiStatusbar1Bar,
  MdSignalWifiStatusbar2Bar,
  MdSignalWifiStatusbar3Bar,
  MdSignalWifiStatusbar4Bar,
  MdSignalWifiStatusbarNull,
} from 'react-icons/md';
import { RiTaxiWifiLine } from 'react-icons/ri';
import { TbCarDoor } from 'react-icons/tb';
import {
  Activity,
  AlertTriangle,
  BatteryWarning,
  Bell,
  Cable,
  CheckCircle2,
  CircleAlert,
  Cpu,
  Droplets,
  Gauge,
  Info,
  Link2Off,
  LockKeyhole,
  Plug,
  Radio,
  Shield,
  Snowflake,
  TriangleAlert,
  Wrench,
} from 'lucide-react';
import {
  api,
  queryKeys,
  resolveVehicleArtwork,
  useAuth,
  useCurrentVehicleStatus,
  useResolvedVehicleSelection,
  useVehicleHealth,
} from '@panda/hooks';
import { resolveVehicleGateCapability, type ClosureMotion, type VehicleHealth } from '@panda/types';
import { SensorChipSummary } from '@panda/dashboards';
import {
  Badge,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  EfficiencyDisplayToggle,
  PageLayout,
  SelectPicker,
  Skeleton,
  Tooltip,
  type BadgeProps,
} from '@panda/ui/primitives';
import {
  DEFAULT_TARGET_TIRE_PRESSURE_PSI,
  formatTireLabel,
  getTireHealthTone,
  normalizeTireWheels,
  summarizeTireHealth,
} from '@panda/ui/lib/vehicleTires';
import {
  formatDistanceKm,
  formatEfficiencyFromWhPerKm,
  formatMassKg,
  getUnitPreferences,
} from '@panda/ui/lib/utils';
import {
  buildAvailabilityTooltip,
  formatAvailabilityLastUpdated,
  presentVehicleStatusDefinition,
  summarizeStatusAvailability,
  type StatusAvailabilitySummary,
  type StatusTone,
} from '@panda/ui/lib/vehicleStatus';
import { AppLayout } from '../../components/layout/AppLayout';
import { NoVehicleState } from '../../components/layout/NoVehicleState';
import { RHealthHero } from '../r-experience/RHealthHero';

type BadgeVariant = NonNullable<BadgeProps['variant']>;
type HealthState = { label: string; variant: BadgeVariant };
type DiagnosticState = {
  label: string;
  variant: BadgeVariant;
  isMissing?: boolean;
  tooltip?: string | null;
  lastUpdatedLabel?: string | null;
};

export function VehicleHealthContent() {
  const { accessToken, setActiveVehicleId } = useAuth();
  const setSessionVehicleId = setActiveVehicleId ?? (() => {});
  const {
    authReady,
    effectiveVehicleId,
    vehicleSelectionReady,
    vehicles: availableVehicles,
  } = useResolvedVehicleSelection();
  const hasVehicleChoices = availableVehicles.length > 1;
  const activeVehicle = availableVehicles.find((vehicle) => vehicle.id === effectiveVehicleId);
  const { data, isLoading } = useVehicleHealth(effectiveVehicleId);
  const { data: status } = useCurrentVehicleStatus(effectiveVehicleId);
  const { data: images } = useQuery({
    queryKey: queryKeys.vehicle.images(effectiveVehicleId),
    queryFn: () => api.vehicleImages(effectiveVehicleId!),
    enabled: authReady && Boolean(effectiveVehicleId) && !!accessToken,
  });

  const diagnostics = summarizeDiagnostics(status);
  const extended = data?.extended_telemetry;
  const [unitPreferences, setUnitPreferences] = useState(getUnitPreferences);
  const unitMode = unitPreferences.mode;
  useEffect(() => {
    const handleUnits = () => setUnitPreferences(getUnitPreferences());
    window.addEventListener('rm-units-change', handleUnits as EventListener);
    window.addEventListener('storage', handleUnits);
    return () => {
      window.removeEventListener('rm-units-change', handleUnits as EventListener);
      window.removeEventListener('storage', handleUnits);
    };
  }, []);
  const vehicleName = data?.vehicle?.name || data?.vehicle?.model || 'Rivian';
  const displayModel = [data?.vehicle?.model, data?.vehicle?.trim].filter(Boolean).join(' ');
  const freshness = getFreshness(data?.runtime?.last_event_at ?? data?.latest?.ts ?? null);
  const collector = getCollectorState(data?.runtime?.worker_health ?? null);
  const twelveVolt = getHealthState(data?.latest?.twelve_volt_health ?? null);
  const thermal = getThermalState(
    data?.latest?.hv_thermal_event ?? null,
    data?.thermal_events_30d ?? 0
  );
  const closureModel = data?.vehicle?.model ?? activeVehicle?.model ?? null;
  const targetTirePressurePsi = Math.round(
    activeVehicle?.target_tire_pressure_psi ?? DEFAULT_TARGET_TIRE_PRESSURE_PSI
  );
  const tireWheels = normalizeTireWheels(status, data?.tires ?? null).map((wheel) => ({
    ...wheel,
    availability: status
      ? summarizeStatusAvailability(status, [
          `tire_${wheel.position}_psi`,
          `tire_${wheel.position}_status`,
          `tire_${wheel.position}_valid`,
        ])
      : null,
  }));
  const tireSummary = summarizeTires(tireWheels, targetTirePressurePsi);
  const softwareHistory = dedupeSoftwareHistory(data?.software_history ?? []);
  const currentSoftwareEntry =
    softwareHistory.find((entry) => !entry.observed_until) ?? softwareHistory[0];
  const currentSoftwareVersion =
    data?.current_software_version ?? currentSoftwareEntry?.version ?? 'Unknown';
  const updateVersion = sanitizeUpdateVersion(
    data?.latest?.ota_available_version ?? null,
    currentSoftwareVersion
  );
  const resolvedHealthArtwork = resolveVehicleArtwork(
    images ?? activeVehicle?.images,
    data?.vehicle?.model ?? activeVehicle?.model,
    'health'
  );
  const heroImageUrl = resolvedHealthArtwork.light;
  const fallbackHeroImageUrl = resolvedHealthArtwork.fallback;
  const closureStatusFallback = Object.fromEntries(HEALTH_CLOSURE_DEFINITIONS.map(({ field }) => [field, status?.[field] ?? data?.closures?.[field] ?? null])) as Record<HealthClosureField, boolean | null>;
  const closureRows = getHealthClosureRows(closureModel, closureStatusFallback, status);
  const closures = summarizeClosures(closureRows);

  return (
    <AppLayout activeKey="health">
      <PageLayout
        title="Vehicle Health"
        subtitle="Mechanical signals, software state, and telemetry freshness for your Rivian."
        className="pt-10 lg:pt-0"
        actions={
          hasVehicleChoices || unitMode !== 'custom' ? (
            <div className="flex flex-wrap items-center justify-end gap-2">
              {unitMode !== 'custom' ? <EfficiencyDisplayToggle /> : null}
              {hasVehicleChoices ? (
                <SelectPicker
                  className="min-w-[11rem]"
                  value={effectiveVehicleId ?? ''}
                  onChange={(vehicleId) => setSessionVehicleId(vehicleId || null)}
                  aria-label="Select vehicle"
                  options={availableVehicles.map((vehicle) => ({
                    value: vehicle.id,
                    label: vehicle.display_name || vehicle.model,
                    description:
                      vehicle.display_name && vehicle.model !== vehicle.display_name
                        ? vehicle.model
                        : undefined,
                  }))}
                />
              ) : null}
            </div>
          ) : null
        }
      >
        {!authReady || !vehicleSelectionReady ? (
          <div className="text-xs text-fg-tertiary p-4">Loading...</div>
        ) : !effectiveVehicleId ? (
          <NoVehicleState
            title="No vehicle selected"
            description="Connect your Rivian account to view vehicle health."
          />
        ) : (
          <>
            <section className="grid gap-4 xl:items-stretch xl:grid-cols-[minmax(0,1.35fr)_minmax(22rem,0.65fr)]">
              <RHealthHero vehicle={activeVehicle} name={vehicleName} model={displayModel} vin={data?.vehicle?.vin} source={heroImageUrl} fallback={fallbackHeroImageUrl}>
                    <HeroMetric label="Collector" state={collector} kind="collector" />
                    <HeroMetric label="12V" state={twelveVolt} kind="battery" />
                    <HeroMetric label="Thermal" state={thermal} kind="thermal" />
                    <HeroMetric label="Tires" state={tireSummary} kind="tires" />
              </RHealthHero>

              <div className="grid min-w-0 gap-2 xl:h-full xl:grid-rows-[auto_minmax(0,1fr)]">
                <Card data-testid="vehicle-telemetry-summary" padding="none" className="px-3 py-2">
                  <CardHeader className="mb-0.5 flex-wrap items-start gap-1">
                    <div className="min-w-0">
                      <CardTitle>Connectivity</CardTitle>
                    </div>
                  </CardHeader>
                  <CardContent>
                    {isLoading ? (
                      <CompactTelemetrySkeleton />
                    ) : !extended?.network && !extended?.efficiency && !extended?.mass && !extended?.cold_weather ? (
                      <EmptyPanel text="Extended telemetry has not produced a meaningful frame yet. Canonical vehicle telemetry continues independently." />
                    ) : (
                      <div className="grid gap-2">
                        <ConnectivitySection network={extended?.network} />
                        <div className="grid gap-1.5 sm:grid-cols-2">
                          <Tooltip
                            content={formatEfficiencyProvenance(extended.efficiency?.reference_wh_per_km)}
                            className="w-full min-w-0"
                          >
                            <HealthLine
                              icon={<Gauge className="h-4 w-4" />}
                              label="Est. Efficiency"
                              value={formatEfficiencyFromWhPerKm(extended.efficiency?.learned_wh_per_km)}
                            />
                          </Tooltip>
                          <Tooltip content="Rivian estimate" className="w-full min-w-0">
                            <HealthLine
                              icon={<GiWeight className="h-4 w-4" />}
                              label="Vehicle mass"
                              value={formatMassKg(extended.mass?.estimated_mass_kg)}
                            />
                          </Tooltip>
                        </div>
                        {extended?.cold_weather ? (
                          <HealthLine
                            icon={<Snowflake className="h-4 w-4" />}
                            label="Cold-weather impact"
                            value={
                              extended.cold_weather.cold_range_impact_km == null
                                ? 'Observed'
                                : formatDistanceKm(extended.cold_weather.cold_range_impact_km)
                            }
                            detail="Rivian-reported cold-limited range impact."
                          />
                        ) : null}
                      </div>
                    )}
                  </CardContent>
                </Card>

                <Card padding="none" className="flex flex-col px-3 py-2 xl:h-full">
                  <CardHeader className="mb-0.5">
                    <CardTitle>Signal Freshness</CardTitle>
                    <Badge variant={freshness.variant} dot>
                      {freshness.label}
                    </Badge>
                  </CardHeader>
                  <CardContent className="flex flex-col gap-1.5 xl:flex-1 xl:justify-between xl:gap-0">
                    <HealthLine
                      icon={<Radio className="h-4 w-4" />}
                      label="Last vehicle event"
                      value={formatDateTime(data?.runtime?.last_event_at ?? data?.latest?.ts)}
                    />
                    <Tooltip
                      content="Generated from the latest stored telemetry and software periods."
                      className="w-full"
                    >
                      <HealthLine
                        icon={<Activity className="h-4 w-4" />}
                        label="API snapshot"
                        value={formatDateTime(data?.generated_at)}
                      />
                    </Tooltip>
                    <HealthLine
                      icon={<Cable className="h-4 w-4" />}
                      label="Acquisition"
                      labelAccessory={
                        <Tooltip
                          content={formatAcquisitionStatusTooltip(extended)}
                          contentClassName="w-72"
                        >
                          <button
                            type="button"
                            className="inline-flex h-11 w-11 shrink-0 sm:h-5 sm:w-5 items-center justify-center rounded-md text-fg-tertiary transition-colors hover:bg-bg-elevated hover:text-fg focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent"
                            aria-label="About acquisition status"
                          >
                            <Info className="h-2.5 w-2.5" />
                          </button>
                        </Tooltip>
                      }
                      value={formatAcquisitionState(extended)}
                    />
                  </CardContent>
                </Card>
              </div>
            </section>

            <section className="grid gap-4 lg:grid-cols-3">
              <StatusPanel
                icon={<BatteryWarning className="h-4 w-4" />}
                title="12V Battery"
                value={twelveVolt.label}
                detail="Reported by Rivian telemetry when the vehicle publishes low-voltage battery health."
                variant={twelveVolt.variant}
                isLoading={isLoading}
              />
              <StatusPanel
                icon={<Gauge className="h-4 w-4" />}
                title="HV Thermal Activity"
                titleAccessory={
                  <Tooltip content="HV thermal events are usually normal battery temperature regulation. High counts alone do not indicate a fault.">
                    <span className="text-fg-tertiary">
                      <Info className="h-3.5 w-3.5" />
                    </span>
                  </Tooltip>
                }
                value={thermal.label}
                detail={`${data?.thermal_events_30d ?? 0} thermal regulation events observed in the last 30 days.`}
                variant={thermal.variant}
                isLoading={isLoading}
              />
              <StatusPanel
                icon={<Cpu className="h-4 w-4" />}
                title="Software"
                value={currentSoftwareVersion}
                detailNode={
                  updateVersion ? (
                    <span>{`Update ${updateVersion} available`}</span>
                  ) : data?.ota_release_notes_url ? (
                    <a
                      className="text-accent underline-offset-2 hover:underline"
                      href={data.ota_release_notes_url}
                      target="_blank"
                      rel="noreferrer"
                    >
                      View release notes
                    </a>
                  ) : (
                    <span>Current version is up to date.</span>
                  )
                }
                variant={updateVersion ? 'info' : 'success'}
                isLoading={isLoading}
              />
            </section>

            <Card>
              <CardHeader>
                <CardTitle>Diagnostics</CardTitle>
                <Badge variant={diagnostics.overall.variant} dot>
                  {diagnostics.overall.label}
                </Badge>
              </CardHeader>
              <CardContent>
                <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                  {diagnostics.rows.map((row) => (
                    <DiagnosticRow
                      key={row.label}
                      icon={row.icon}
                      label={row.label}
                      state={row.state}
                    />
                  ))}
                </div>
              </CardContent>
            </Card>

            <section className="grid gap-4 xl:grid-cols-[minmax(0,0.95fr)_minmax(0,1.05fr)]">
              <Card>
                <CardHeader className="flex-wrap items-start gap-x-3 gap-y-2">
                  <div className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-2 gap-y-1">
                    <CardTitle>Tire Pressure</CardTitle>
                    {data?.tires?.ts ? (
                      <p
                        data-testid="tire-reading-updated"
                        className="min-w-0 break-words text-xs leading-5 text-fg-tertiary"
                      >
                        Updated: {formatAppDateTime(data.tires.ts)}
                      </p>
                    ) : null}
                  </div>
                  <Badge variant={tireSummary.variant}>{tireSummary.label}</Badge>
                </CardHeader>
                <CardContent>
                  {isLoading ? (
                    <HealthGridSkeleton />
                  ) : !data?.tires ? (
                    <EmptyPanel text="No tire telemetry found yet." />
                  ) : (
                    <div className="grid grid-cols-2 gap-3">
                      {tireWheels.map((wheel) => (
                        <TireGauge
                          key={wheel.position}
                          label={wheel.label}
                          targetPressurePsi={targetTirePressurePsi}
                          value={wheel.psi}
                          status={wheel.status}
                          valid={wheel.valid}
                          availability={wheel.availability}
                        />
                      ))}
                    </div>
                  )}
                </CardContent>
              </Card>

              <Card>
                <CardHeader>
                  <CardTitle>Doors &amp; Gates</CardTitle>
                  <Badge variant={closures.variant} dot>
                    {closures.label}
                  </Badge>
                </CardHeader>
                <CardContent>
                  {isLoading ? (
                    <HealthGridSkeleton />
                  ) : !data?.closures ? (
                    <EmptyPanel text="No door and gate telemetry found yet." />
                  ) : (
                    <div className="grid gap-2 sm:grid-cols-2">
                      {closureRows.map((row) => (
                        <ClosureRow
                          key={row.field}
                          field={row.field}
                          label={row.label}
                          value={row.value}
                          motion={status?.closure_motion?.[row.field] ?? null}
                          availability={row.availability}
                        />
                      ))}
                    </div>
                  )}
                </CardContent>
              </Card>
            </section>

            <Card>
              <CardHeader>
                <CardTitle>Software History</CardTitle>
                <Badge variant="info" className="max-w-full truncate font-mono">
                  {currentSoftwareVersion}
                </Badge>
              </CardHeader>
              <CardContent className="space-y-3">
                {softwareHistory.length === 0 ? (
                  <EmptyPanel text="No software version history yet." />
                ) : (
                  <>
                    {currentSoftwareEntry ? (
                      <div className="rounded-xl border border-accent/30 bg-accent-muted/40 px-3 py-2">
                        <p className="text-xs font-semibold uppercase tracking-wider text-fg-tertiary">
                          Current version
                        </p>
                        <p className="mt-1 font-mono text-sm text-fg">
                          {currentSoftwareEntry.version}
                        </p>
                        <p className="mt-1 text-xs text-fg-secondary">
                          Observed since {formatDateTime(currentSoftwareEntry.installed_at)}
                        </p>
                        {data?.ota_release_notes_url ? (
                          <a
                            className="mt-2 inline-flex text-xs text-accent underline-offset-2 hover:underline"
                            href={data.ota_release_notes_url}
                            target="_blank"
                            rel="noreferrer"
                          >
                            Changelog
                          </a>
                        ) : null}
                      </div>
                    ) : null}

                    <details className="group rounded-xl border border-border bg-bg-elevated/45 p-3">
                      <summary className="cursor-pointer list-none text-xs font-semibold uppercase tracking-wider text-fg-tertiary">
                        <span className="inline-flex items-center gap-2">
                          Full history ({softwareHistory.length} entries)
                          <span className="transition-transform group-open:rotate-180">▾</span>
                        </span>
                      </summary>
                      <div className="relative mt-3 space-y-3 before:absolute before:left-[0.42rem] before:top-2 before:h-[calc(100%-1rem)] before:w-px before:bg-border">
                        {softwareHistory.map((entry, index) => (
                          <div
                            key={`${entry.version}-${entry.installed_at}-${entry.observed_until ?? 'open'}`}
                            className="relative grid gap-1 pl-6 sm:grid-cols-[minmax(10rem,0.7fr)_minmax(0,1fr)]"
                          >
                            <span
                              className={`absolute left-0 top-1.5 h-3 w-3 rounded-full border ${index === 0 ? 'border-accent bg-accent' : 'border-border-strong bg-bg-elevated'}`}
                            />
                            <div>
                              <p className="font-mono text-sm text-fg">{entry.version}</p>
                              <p className="mt-0.5 text-xs text-fg-tertiary">
                                {entry.observed_until ? 'Previous software' : 'Current software'}
                              </p>
                            </div>
                            <p className="text-sm text-fg-secondary">
                              Observed {formatDateTime(entry.installed_at)}
                              <span className="text-fg-tertiary"> to </span>
                              {entry.observed_until
                                ? formatDateTime(entry.observed_until)
                                : 'Current'}
                            </p>
                          </div>
                        ))}
                      </div>
                    </details>
                  </>
                )}
              </CardContent>
            </Card>

          </>
        )}
      </PageLayout>
    </AppLayout>
  );
}

function formatParallaxState(status: string | undefined) {
  const labels = new Map([
    ['connected', 'Connected'], ['reconnecting', 'Reconnecting'], ['stale', 'Stale'],
    ['disabled', 'Disabled'], ['duplicate_owner', 'Duplicate owner'], ['error', 'Error'],
    ['disconnected', 'Disconnected'], ['starting', 'Starting'],
  ]);
  return labels.get(status ?? '') ?? 'Never observed';
}

function formatAcquisitionState(extended: VehicleHealth['extended_telemetry'] | undefined) {
  // The collector state is the authoritative connection state. Parallax is a
  // compatibility fallback for older API responses, not a second status.
  return formatParallaxState(extended?.collector?.status ?? extended?.parallax?.status);
}

function formatWifiRadioDetail(network: VehicleHealth['extended_telemetry']['network'] | undefined) {
  const details: string[] = [];
  if (network?.wifi_frequency_mhz != null) details.push(`${(network.wifi_frequency_mhz / 1000).toFixed(1)} GHz`);
  if (network?.wifi_channel_width_mhz != null) details.push(`${network.wifi_channel_width_mhz} MHz`);
  return details.join(' · ') || 'No radio reading';
}

function getWifiStrengthLabel(rssi: number) {
  return wifiStrength(rssi).label;
}

function getWifiStrengthLevel(connected: boolean | null | undefined, rssi: number | null | undefined) {
  return connected === true && rssi != null ? wifiStrength(rssi).level : 0;
}

function WifiStrengthGlyph({
  connected,
  rssi,
}: {
  connected: boolean | null | undefined;
  rssi: number | null | undefined;
}) {
  const icons = [MdSignalWifiStatusbarNull, MdSignalWifiStatusbar1Bar, MdSignalWifiStatusbar2Bar, MdSignalWifiStatusbar3Bar, MdSignalWifiStatusbar4Bar];
  const Icon = icons[getWifiStrengthLevel(connected, rssi)]!;
  return <Icon className="h-7 w-7" aria-hidden="true" />;
}

function ConnectivitySection({
  network,
}: {
  network: VehicleHealth['extended_telemetry']['network'] | undefined;
}) {
  const connected = network?.wifi_connected;
  const rssi = network?.wifi_rssi_dbm;
  const connectivity = new Map<boolean | null | undefined, { status: string; variant: BadgeVariant }>([
    [true, { status: 'Connected', variant: 'success' }], [false, { status: 'Disconnected', variant: 'warning' }],
  ]).get(connected) ?? { status: 'Unavailable', variant: 'default' };
  const strength = getWifiStrengthLevel(connected, rssi) ? getWifiStrengthLabel(rssi!) : 'Unavailable';
  const metrics: Array<React.ComponentProps<typeof ConnectivityMetric>> = [
    { icon: <WifiStrengthGlyph connected={connected} rssi={rssi} />, iconLabel: `Wi-Fi strength: ${strength}`, label: 'Wi-Fi signal', value: strength, detail: rssi == null ? 'No dBm reading' : `${rssi} dBm` },
    { icon: <Gauge className="h-4 w-4" />, label: 'Throughput', value: network?.wifi_link_speed_mbps == null ? 'Unavailable' : `${network.wifi_link_speed_mbps} Mbps`, detail: formatWifiRadioDetail(network) },
    { icon: <RiTaxiWifiLine className="h-4 w-4" />, label: 'Wi-Fi status', value: connectivity.status, valueVariant: connectivity.variant,
      ...(connected === false && network?.cellular_access_technology ? { detail: `${network.cellular_access_technology} cellular` } : {}) },
  ];
  return (
    <section aria-label="Connectivity">
      <div className="grid grid-cols-3 gap-1.5">
        {metrics.map((metric) => <ConnectivityMetric key={metric.label} {...metric} />)}
      </div>
    </section>
  );
}

function ConnectivityMetric({
  icon,
  iconLabel,
  label,
  value,
  valueVariant,
  detail,
}: {
  icon: React.ReactNode;
  iconLabel?: string;
  label: string;
  value: string;
  valueVariant?: BadgeProps['variant'];
  detail?: string | undefined;
}) {
  return (
    <div className="min-w-0 rounded-lg border border-border bg-bg-elevated/45 p-1.5">
      <div className="flex h-8 w-8 items-center justify-center rounded-md border border-border bg-bg-elevated text-accent">
        {iconLabel ? <span role="img" aria-label={iconLabel}>{icon}</span> : icon}
      </div>
      <p className="mt-1.5 truncate text-[10px] font-semibold uppercase tracking-wider text-fg-tertiary">{label}</p>
      {valueVariant ? (
        <Badge variant={valueVariant} size="sm" className="mt-0.5 max-w-full truncate">
          {value}
        </Badge>
      ) : (
        <p className="mt-0.5 truncate font-mono text-[12px] text-fg">{value}</p>
      )}
      {detail ? <p className="mt-0.5 truncate text-[10px] text-fg-secondary">{detail}</p> : null}
    </div>
  );
}

function formatEfficiencyProvenance(referenceWhPerKm: number | null | undefined) {
  return referenceWhPerKm == null
    ? 'Rivian estimate. No reference efficiency is available.'
    : `Rivian estimate · ${formatEfficiencyFromWhPerKm(referenceWhPerKm)} reference`;
}

function formatAcquisitionTooltip(extended: VehicleHealth['extended_telemetry'] | undefined) {
  const parallax = extended?.parallax;
  const frame = parallax?.last_meaningful_frame_at
    ? formatAppDateTime(parallax.last_meaningful_frame_at)
    : parallax?.last_error ?? 'No meaningful frame observed';
  const diagnostics = `${parallax?.reconnect_count ?? 0} reconnects · ${parallax?.decode_error_count ?? 0} decode · ${parallax?.empty_frame_count ?? 0} empty · ${parallax?.ambiguity_count ?? 0} ambiguous`;
  return (
    <div className="grid gap-1">
      <span className="font-medium text-fg">Meaningful frame</span>
      <span>{frame}</span>
      <span>{diagnostics}</span>
    </div>
  );
}

function formatAcquisitionStatusTooltip(extended: VehicleHealth['extended_telemetry'] | undefined) {
  return (
    <div className="grid gap-2 leading-4">
      <div className="grid gap-0.5">
        <span className="font-medium text-fg">Parallax acquisition</span>
        <span>This is Panda&apos;s integrated Parallax acquisition WebSocket, separate from the canonical vehicle telemetry connection.</span>
      </div>
      <div className="grid gap-0.5 border-t border-border pt-2">
        <span className="font-medium text-fg">What the status means</span>
        <span>Connected means the socket handshake and subscription are active. Error means the latest token, socket, subscription, or heartbeat attempt failed and will be retried; it does not by itself indicate a vehicle fault.</span>
      </div>
      <div className="grid gap-0.5 border-t border-border pt-2">
        <span className="font-medium text-fg">Latest evidence</span>
        {formatAcquisitionTooltip(extended)}
      </div>
    </div>
  );
}

function HeroMetric({
  label,
  state,
  kind,
}: {
  label: string;
  state: HealthState;
  kind: 'collector' | 'battery' | 'thermal' | 'tires';
}) {
  const indicator = getHeroStateIcon(label, state);
  const leading = getHeroLeadingIcon(kind);

  return (
    <div
      className="flex w-full min-w-0 items-center justify-between gap-2 rounded-xl border border-border bg-bg-glass px-3 py-2.5"
      title={`${label}: ${state.label}`}
      aria-label={`${label}: ${state.label}`}
    >
      <span className="inline-flex min-w-0 items-center gap-1.5">
        <span className="text-fg-tertiary">{leading}</span>
        <span className="truncate text-[13px] font-semibold uppercase tracking-wider text-fg-tertiary">
          {label}
        </span>
      </span>
      <span className="shrink-0 text-fg-tertiary">{indicator}</span>
    </div>
  );
}

function StatusPanel({
  icon,
  title,
  titleAccessory,
  value,
  detail,
  detailNode,
  variant,
  isLoading,
}: {
  icon: React.ReactNode;
  title: string;
  titleAccessory?: React.ReactNode;
  value: string;
  detail?: string;
  detailNode?: React.ReactNode;
  variant: BadgeVariant;
  isLoading: boolean;
}) {
  return (
    <Card>
      <div className="flex items-start gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-border bg-bg-elevated text-accent">
          {icon}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-2">
            <p className="inline-flex items-center gap-1 text-xs font-semibold uppercase tracking-wider text-fg-tertiary">
              {title}
              {titleAccessory}
            </p>
            {isLoading ? (
              <Skeleton className="h-7 w-28" />
            ) : (
              <Badge variant={variant} className="max-w-full truncate">
                {value}
              </Badge>
            )}
          </div>
          <p className="mt-3 text-sm leading-5 text-fg-secondary">{detailNode ?? detail}</p>
        </div>
      </div>
    </Card>
  );
}

function HealthLine({
  icon,
  label,
  value,
  detail,
  labelAccessory,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  detail?: React.ReactNode;
  labelAccessory?: React.ReactNode;
}) {
  return (
    <div className="grid grid-cols-[2.125rem_minmax(0,1fr)] items-start gap-x-2">
      <div className="flex h-[34px] w-[34px] shrink-0 items-center justify-center rounded-lg border border-border bg-bg-elevated text-accent">
        {icon}
      </div>
      <div className="grid h-[34px] min-w-0 grid-rows-[14px_20px]">
        <p className="relative -top-px inline-flex items-start gap-1 text-[13px] font-semibold uppercase leading-[14px] tracking-wider text-fg-tertiary">
          {label}
          {labelAccessory}
        </p>
        <p className="self-end truncate font-mono text-[15px] leading-5 text-fg">{value}</p>
      </div>
      {detail ? <p className="col-start-2 mt-1 text-[11px] leading-3 text-fg-tertiary">{detail}</p> : null}
    </div>
  );
}

function TireGauge({
  label,
  value,
  targetPressurePsi,
  status,
  valid,
  availability,
}: {
  label: string;
  value: number | null;
  targetPressurePsi: number;
  status: string | null;
  valid: boolean | null;
  availability: StatusAvailabilitySummary | null;
}) {
  const state = getTireState(status, valid, availability);
  const tone = getTireHealthTone({ psi: value, status, targetPsi: targetPressurePsi });
  const presentation: { value: string; tone: 'info' | 'danger' | 'warning' | 'success' | 'neutral' } = {
    value: formatTireLabel(value, status),
    tone: tone === 'danger' || tone === 'warning' || tone === 'success' ? tone : 'neutral',
  };
  if (availability?.availability === 'never_seen') {
    presentation.tone = 'info';
    if (value === null) presentation.value = 'Unavailable';
  }
  if (valid === false) Object.assign(presentation, { value: 'Invalid Sensor', tone: 'info' });
  const sensor = (
    <SensorChipSummary
      title={label}
      value={presentation.value}
      icon="lucide:gauge"
      valueTone={presentation.tone}
      valueSize="lg"
    />
  );
  return state.tooltip ? <Tooltip content={state.tooltip}>{sensor}</Tooltip> : sensor;
}

function DiagnosticRow({
  icon,
  label,
  state,
}: {
  icon: React.ReactNode;
  label: string;
  state: DiagnosticState;
}) {
  const badge = (
    <Badge variant={state.variant} size="sm">
      {state.label}
    </Badge>
  );
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border bg-bg-elevated/55 px-3 py-2">
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <span className="text-fg-tertiary">{icon}</span>
          <span className="truncate text-sm text-fg-secondary">{label}</span>
        </div>
        {state.lastUpdatedLabel ? (
          <p className="mt-1 truncate text-[11px] text-fg-tertiary">{state.lastUpdatedLabel}</p>
        ) : null}
      </div>
      {state.tooltip ? <Tooltip content={state.tooltip}>{badge}</Tooltip> : badge}
    </div>
  );
}

function ClosureRow({
  field,
  label,
  value,
  motion,
  availability,
}: {
  field: HealthClosureField;
  label: string;
  value: boolean | null;
  motion: ClosureMotion | null;
  availability: StatusAvailabilitySummary | null;
}) {
  const isUnavailable = availability?.availability === 'never_seen' && value === null;
  const isGate = field.startsWith('closure_');
  const states = new Map<boolean | null, { variant: BadgeVariant; label: string }>([
    [true, { variant: 'success', label: 'Closed' }],
    [false, { variant: 'warning', label: 'Open' }],
    [null, { variant: 'default', label: 'Unknown' }],
  ]);
  const presentation = { ...(states.get(value) ?? states.get(null)!) };
  if (motion) presentation.variant = 'info';
  if (motion === 'opening') presentation.label = 'Opening…';
  if (motion === 'closing') presentation.label = 'Closing…';
  if (isUnavailable) Object.assign(presentation, { variant: 'info', label: 'Unavailable' });
  const badge = <Badge variant={presentation.variant}>{presentation.label}</Badge>;
  const tooltip = buildAvailabilityTooltip(
    label,
    availability ?? {
      availability: 'never_seen',
      reasonCode: 'never_seen',
      lastSeenAt: null,
      latestEventAt: null,
      everSeen: false,
    }
  );
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border bg-bg-elevated/55 px-3 py-2">
      <div className="min-w-0">
        <div className="flex min-w-0 items-center gap-2">
          <ClosureIcon field={field} isGate={isGate} isUnavailable={isUnavailable} />
          <span className="truncate text-sm text-fg-secondary">{label}</span>
        </div>
        {availability && formatAvailabilityLastUpdated(availability) ? (
          <p className="mt-1 truncate text-[11px] text-fg-tertiary">
            {formatAvailabilityLastUpdated(availability)}
          </p>
        ) : null}
      </div>
      {tooltip ? <Tooltip content={tooltip}>{badge}</Tooltip> : badge}
    </div>
  );
}

function HealthGridSkeleton() {
  return (
    <div className="grid grid-cols-2 gap-3">
      {Array.from({ length: 4 }).map((_, index) => (
        <Skeleton key={index} className="h-24" />
      ))}
    </div>
  );
}

function CompactTelemetrySkeleton() {
  return (
    <div className="space-y-3">
      {Array.from({ length: 3 }).map((_, index) => (
        <Skeleton key={index} className="h-12" />
      ))}
    </div>
  );
}

function EmptyPanel({ text }: { text: string }) {
  return (
    <div className="rounded-xl border border-dashed border-border bg-bg-elevated/40 px-4 py-8 text-center text-sm text-fg-tertiary">
      {text}
    </div>
  );
}

function ClosureIcon({
  field,
  isGate,
  isUnavailable,
}: {
  field: HealthClosureField;
  isGate: boolean;
  isUnavailable: boolean;
}) {
  const className = 'h-4 w-4 shrink-0 text-fg-tertiary';
  const isRightDoor = !isGate && field.endsWith('_right_closed');
  const testId = `closure-icon-${field}`;

  return (
    <span
      className="inline-flex shrink-0"
      data-testid={testId}
      data-closure-kind={isGate ? 'gate' : 'door'}
      aria-hidden="true"
    >
      {isUnavailable ? (
        <CircleAlert className={className} />
      ) : !isGate ? (
        <TbCarDoor
          className={className}
          style={isRightDoor ? { transform: 'scaleX(-1)' } : undefined}
        />
      ) : field === 'closure_frunk_closed' ? (
        <FrunkIcon className={className} />
      ) : (
        <LiftgateIcon className={className} />
      )}
    </span>
  );
}

function FrunkIcon({ className }: { className: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M5 20v-1.6c0-2.1 1.4-3.8 3.5-4.2l9.5-1.9" />
      <path d="m8.5 5.5 2.7-1.4 8.9 8.8-3.2.8-8.4-8.2Z" />
      <path d="M5 14v-4m0 0h4m-4 0 6 6" />
    </svg>
  );
}

function LiftgateIcon({ className }: { className: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M5 4h7.2c1.5 0 2.8.6 3.8 1.7l2.4 2.8H12" />
      <path d="M5 20c2.1-.2 3.5-1.6 3.5-4V8.5C8.5 6 7.2 4.5 5 4" />
      <path d="M12 15h7" />
      <path d="m16 12 3 3-3 3" />
    </svg>
  );
}

function summarizeTires(
  wheels: Array<ReturnType<typeof normalizeTireWheels>[number] & { availability: StatusAvailabilitySummary | null }>,
  targetPsi: number
): HealthState & { detail: string } {
  const health = summarizeTireHealth(wheels, targetPsi);
  const missing = { label: 'Unavailable', detail: health.hasInvalidSensor ? 'Invalid sensor' : 'No readings yet', variant: 'info' as const };
  if (health.hasInvalidSensor || (!health.readings.length && wheels.some((wheel) => wheel.availability?.availability === 'never_seen'))) return missing;
  if (health.tone === 'danger' || health.tone === 'warning') return { label: 'Check', detail: 'Attention needed', variant: health.tone };
  switch (health.readings.length) {
    case 0: return missing;
    case 4: return { label: 'Normal', detail: `${Math.round(Math.min(...health.readings))}-${Math.round(Math.max(...health.readings))} psi`, variant: 'success' };
    default: return { label: 'Partial', detail: `${health.readings.length}/4 wheels`, variant: 'info' };
  }
}

type HealthClosureField =
  | 'closure_frunk_closed'
  | 'closure_liftgate_closed'
  | 'closure_tailgate_closed'
  | 'door_front_left_closed'
  | 'door_front_right_closed'
  | 'door_rear_left_closed'
  | 'door_rear_right_closed';

type HealthClosureRow = {
  field: HealthClosureField;
  label: string;
  value: boolean | null;
  availability: StatusAvailabilitySummary | null;
};

const HEALTH_CLOSURE_DEFINITIONS: Array<Pick<HealthClosureRow, 'field' | 'label'>> = [
  { field: 'closure_frunk_closed', label: 'Frunk' },
  { field: 'closure_liftgate_closed', label: 'Liftgate' },
  { field: 'closure_tailgate_closed', label: 'Tailgate' },
  { field: 'door_front_left_closed', label: 'Front left door' },
  { field: 'door_front_right_closed', label: 'Front right door' },
  { field: 'door_rear_left_closed', label: 'Rear left door' },
  { field: 'door_rear_right_closed', label: 'Rear right door' },
];

function getHealthClosureRows(
  model: string | null | undefined,
  values: Record<HealthClosureField, boolean | null>,
  status: import('@panda/types').VehicleStatus | null | undefined
): HealthClosureRow[] {
  const gate = resolveVehicleGateCapability(model);
  const supported = gate === 'tailgate' ? 'closure_tailgate_closed' : gate === 'liftgate' ? 'closure_liftgate_closed' : null;
  const observed = new Set(HEALTH_CLOSURE_DEFINITIONS.filter(({ field }) => values[field] !== null || status?.field_availability?.[field]?.ever_seen === true).map(({ field }) => field));
  return HEALTH_CLOSURE_DEFINITIONS.flatMap(({ field, label }) => {
    const isGate = field === 'closure_liftgate_closed' || field === 'closure_tailgate_closed';
    const include = gate === null ? observed.has(field) : !isGate || field === supported;
    return include ? [{ field, label, value: values[field], availability: status ? summarizeStatusAvailability(status, [field]) : null }] : [];
  });
}

function summarizeClosures(rows: HealthClosureRow[]) {
  const counts = rows.reduce((result, row) => ({ open: result.open + Number(row.value === false), known: result.known + Number(row.value !== null) }), { open: 0, known: 0 });
  const state = counts.open ? 'open' : counts.known ? 'secured' : 'missing';
  return {
    open: { label: `${counts.open} open`, variant: 'warning' as const },
    secured: { label: 'Secured', variant: 'success' as const },
    missing: { label: 'Unavailable', variant: 'info' as const },
  }[state];
}

function classifyHealth(value: string | null, healthy: RegExp, failed: RegExp): HealthState {
  if (!value) return { label: 'Unknown', variant: 'default' };
  const evidence = [healthy.test(value), failed.test(value)];
  return { label: titleCase(value), variant: evidence[0] ? 'success' : evidence[1] ? 'danger' : 'warning' };
}

function getCollectorState(value: string | null): HealthState {
  return classifyHealth(value, /connected|healthy|ok/i, /auth|error|failed/i);
}

function getHealthState(value: string | null): HealthState {
  return classifyHealth(value, /normal|good|ok/i, /critical|fault|fail/i);
}

function getThermalState(value: string | null, count: number): HealthState {
  void count;
  const nominal = !value || /^(off|none|inactive|normal|ok|good)$/i.test(value);
  return nominal ? { label: 'Nominal', variant: 'success' } : { label: titleCase(value), variant: 'warning' };
}

function getTireState(
  status: string | null,
  valid: boolean | null,
  availability: StatusAvailabilitySummary | null
): DiagnosticState {
  const summary = availability ?? nullSummary();
  const lastUpdatedLabel = formatAvailabilityLastUpdated(summary);
  if (valid === false) {
    const invalid: StatusAvailabilitySummary = { availability: availability?.availability ?? 'current' as const, reasonCode: 'invalid_sensor', lastSeenAt: availability?.lastSeenAt ?? null, latestEventAt: availability?.latestEventAt ?? null, everSeen: availability?.everSeen ?? true };
    return { label: 'Invalid Sensor', variant: 'info', tooltip: buildAvailabilityTooltip('Tire pressure', invalid), lastUpdatedLabel: formatAvailabilityLastUpdated(availability ?? invalid) };
  }
  if (!status) return availability?.availability === 'never_seen'
    ? { label: 'Unavailable', variant: 'info', tooltip: buildAvailabilityTooltip('Tire pressure', availability) }
    : { label: 'No status', variant: 'default', lastUpdatedLabel };
  const state = classifyHealth(status, /normal|ok/i, /critical|fault/i);
  return state.variant === 'danger' ? state : { ...state, lastUpdatedLabel };
}

function summarizeDiagnostics(status: import('@panda/types').VehicleStatus | null | undefined) {
  const rows = [
    {
      label: 'Brake Fluid',
      icon: <Droplets className="h-4 w-4" />,
      state: asDiagnosticState(presentVehicleStatusDefinition('brake_fluid_warning', status)),
    },
    {
      label: 'Wiper Fluid',
      icon: <Droplets className="h-4 w-4" />,
      state: asDiagnosticState(presentVehicleStatusDefinition('wiper_fluid_warning', status)),
    },
    {
      label: 'Service Mode',
      icon: <Wrench className="h-4 w-4" />,
      state: asDiagnosticState(presentVehicleStatusDefinition('service_mode', status)),
    },
    {
      label: 'Alarm',
      icon: <Bell className="h-4 w-4" />,
      state: asDiagnosticState(presentVehicleStatusDefinition('alarm_status', status)),
    },
    {
      label: 'Gear Guard',
      icon: <Shield className="h-4 w-4" />,
      state: asDiagnosticState(presentVehicleStatusDefinition('gear_guard_locked', status)),
    },
    {
      label: 'Charge Port',
      icon: <Plug className="h-4 w-4" />,
      state: asDiagnosticState(presentVehicleStatusDefinition('charge_port_open', status)),
    },
    {
      label: 'Charger Derate',
      icon: <AlertTriangle className="h-4 w-4" />,
      state: asDiagnosticState(presentVehicleStatusDefinition('charger_derate_active', status)),
    },
    {
      label: 'Defrost',
      icon: <Snowflake className="h-4 w-4" />,
      state: asDiagnosticState(presentVehicleStatusDefinition('defrost_active', status)),
    },
    {
      label: 'Cabin Precondition',
      icon: <Activity className="h-4 w-4" />,
      state: asDiagnosticState(presentVehicleStatusDefinition('cabin_precon', status)),
    },
  ];

  const priorities = ['danger', 'warning', 'info', 'success'] as const;
  const observed = new Set(rows.filter((row) => !row.state.isMissing).map((row) => row.state.variant));
  const severity = priorities.find((tone) => observed.has(tone));
  const labels = { danger: 'Attention', warning: 'Check', info: 'Active', success: 'All clear' };
  const overall: DiagnosticState = severity ? { label: labels[severity], variant: severity } : { label: 'Unavailable', variant: 'info' };

  return { rows, overall };
}

function asDiagnosticState(
  presented: ReturnType<typeof presentVehicleStatusDefinition>
): DiagnosticState {
  return {
    label: presented.renderUnavailableChip ? 'Unavailable' : presented.label,
    variant: asBadgeVariant(presented.renderUnavailableChip ? 'info' : presented.variant),
    isMissing: presented.renderUnavailableChip,
    tooltip: presented.tooltip,
    lastUpdatedLabel: presented.lastUpdatedLabel,
  };
}

function asBadgeVariant(tone: StatusTone): BadgeVariant {
  if (tone === 'danger') return 'danger';
  if (tone === 'warning') return 'warning';
  if (tone === 'success') return 'success';
  if (tone === 'info') return 'info';
  return 'default';
}

function nullSummary(): StatusAvailabilitySummary {
  return {
    availability: 'never_seen',
    reasonCode: 'never_seen',
    lastSeenAt: null,
    latestEventAt: null,
    everSeen: false,
  };
}

function getFreshness(ts: string | null) {
  if (!ts) return { label: 'No events', variant: 'default' as const };
  const age = Date.now() - Date.parse(ts);
  if (!Number.isFinite(age)) return { label: 'Unknown', variant: 'default' as const };
  const intervals = [
    { until: 900_000, label: 'Live', variant: 'success' as const },
    { until: 7_200_000, label: 'Recent', variant: 'info' as const },
    { until: 86_400_000, label: 'Stale', variant: 'warning' as const },
  ];
  const interval = intervals.find((candidate) => age < candidate.until);
  return interval ? { label: interval.label, variant: interval.variant } : { label: 'Old', variant: 'danger' as const };
}

function formatDateTime(value?: string | null) {
  if (!value) return 'Unknown';
  return formatAppDateTime(value, { second: '2-digit', timeZoneName: 'short' });
}



function titleCase(value: string) {
  return value.replace(/[_-]+/g, ' ').replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function sanitizeUpdateVersion(version: string | null, currentVersion: string) {
  if (!version) return null;
  const normalized = version.trim();
  if (!normalized) return null;
  if (/^0+(\.0+)*$/.test(normalized)) return null;
  if (normalized === currentVersion) return null;
  return normalized;
}

function dedupeSoftwareHistory(entries: import('@panda/types').VehicleHealthSoftwareEntry[]) {
  const sorted = entries.slice().sort((a, b) => Date.parse(b.installed_at) - Date.parse(a.installed_at));
  const runs: typeof entries[] = [];
  for (const entry of sorted) {
    const run = runs[runs.length - 1];
    if (run?.[0]?.version === entry.version) run.push(entry);
    else runs.push([entry]);
  }
  return runs.map(([first, ...rest]) => rest.reduce((aggregate, entry) => ({
    ...aggregate,
    installed_at: Date.parse(entry.installed_at) < Date.parse(aggregate.installed_at) ? entry.installed_at : aggregate.installed_at,
    observed_until: aggregate.observed_until === null || entry.observed_until === null ? null : Date.parse(aggregate.observed_until) > Date.parse(entry.observed_until) ? aggregate.observed_until : entry.observed_until,
  }), first!));
}

function getHeroStateIcon(label: string, state: HealthState) {
  type IconChoice = { Icon: React.ElementType; className: string };
  const normal = 'h-5 w-5', positive = `${normal} text-status-positive`, warning = `${normal} text-status-warning`;
  const choices: Array<{ keyword: string; fallback: IconChoice; states: Partial<Record<BadgeVariant, IconChoice>> }> = [
    { keyword: 'collector', fallback: { Icon: Radio, className: normal }, states: { success: { Icon: Cable, className: positive }, danger: { Icon: Link2Off, className: `${normal} text-status-critical` } } },
    { keyword: '12v', fallback: { Icon: BatteryWarning, className: normal }, states: { success: { Icon: CheckCircle2, className: positive }, danger: { Icon: BatteryWarning, className: warning }, warning: { Icon: BatteryWarning, className: warning } } },
    { keyword: 'thermal', fallback: { Icon: Gauge, className: normal }, states: { success: { Icon: CheckCircle2, className: positive }, danger: { Icon: TriangleAlert, className: warning }, warning: { Icon: TriangleAlert, className: warning } } },
    { keyword: 'tires', fallback: { Icon: CheckCircle2, className: normal }, states: { success: { Icon: CheckCircle2, className: positive }, danger: { Icon: TriangleAlert, className: warning }, warning: { Icon: TriangleAlert, className: warning } } },
  ];
  const match = choices.find(({ keyword }) => label.toLowerCase().includes(keyword));
  if (!match) return iconFallback(state);
  const { Icon, className } = match.states[state.variant] ?? match.fallback;
  return <Icon className={className} />;
}

function iconFallback(state: HealthState) {
  const icons: Partial<Record<BadgeVariant, React.ElementType>> = { success: CheckCircle2, danger: TriangleAlert, warning: TriangleAlert };
  const Icon = icons[state.variant] ?? CircleAlert;
  return <Icon className="h-5 w-5" />;
}

function getHeroLeadingIcon(kind: 'collector' | 'battery' | 'thermal' | 'tires') {
  const Icon = { collector: Radio, battery: BatteryWarning, thermal: Gauge, tires: LockKeyhole }[kind];
  return <Icon className="h-5 w-5" />;
}

function wifiStrength(rssi: number) {
  const bands = [
    { minimum: -55, label: 'Strong', level: 4 },
    { minimum: -67, label: 'Good', level: 3 },
    { minimum: -75, label: 'Fair', level: 2 },
  ];
  return bands.find(({ minimum }) => rssi >= minimum) ?? { label: 'Weak', level: 1 };
}
