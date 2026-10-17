import { pageNavigation, TABLE_PAGE_OPTIONS, TABLE_PAGE_SIZES, tripBatchActions } from './tableControls';
import { useTripTagBatch } from './useTripTagBatch';
import { projectMapRoutes, tripTagBatch, tripPresentation } from './tableProjection';
import { useWidgetMediaQuery } from '../widgetLifecycle';
import React from 'react';
import { useNavigate } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { Eraser, Moon, Plus, Save, Sun } from 'lucide-react';
import { LuBadgeInfo } from 'react-icons/lu';
import { PiArrowFatLinesRight } from 'react-icons/pi';
import { api, useAuth, useBasemapConfig, useTrips, useTripMapRoutes, useUserPreferences } from '@panda/hooks';
import { useDocumentTheme } from '@panda/ui/hooks';
import { DataTable, createTripColumns, type TripRow } from '@panda/ui/tables';
import { Badge, Button, SelectPicker, Tooltip } from '@panda/ui/primitives';
import { TripMapChart, type MapStyleMode } from '@panda/ui/charts';
import { formatMiles, formatDuration } from '@panda/ui/lib/utils';
import { registerWidget } from '../../registry';
import type { WidgetInstance, WidgetCtx } from '../../registry';
import { useMeasuredWidgetHeight } from '../useMeasuredWidgetHeight';
import { TripTagBadges, TripTagPicker } from './TripTagPicker';
import { TripRouteKey } from './TripRouteKey';

function useIsMobile() {
  return useWidgetMediaQuery('(max-width: 639px)', 640);
}

function TripCard({
  trip,
  isSelected,
  onClick,
  onInfoClick,
}: {
  trip: TripRow;
  isSelected: boolean;
  onClick: () => void;
  onInfoClick: () => void;
}) {
  const model = tripPresentation(trip);
  const { start: startLabel, end: endLabel } = model;

  return (
    <div
      className={`relative w-full rounded-xl border px-3 py-2.5 transition-colors ${
        isSelected
          ? 'border-accent/50 bg-accent/10 ring-1 ring-inset ring-accent/30'
          : 'border-border bg-bg-elevated/60 hover:bg-bg-elevated'
      }`}
    >
      <button
        type="button"
        onClick={onInfoClick}
        className="absolute right-2 top-2 inline-flex h-7 w-7 items-center justify-center rounded-md border border-border bg-bg-surface text-fg-tertiary transition-colors hover:border-border-strong hover:text-fg"
        aria-label="Open trip details"
      >
        <LuBadgeInfo className="h-4 w-4" />
      </button>
      <button type="button" onClick={onClick} className="w-full text-left pr-9">
      <div className="flex items-center justify-between gap-2 mb-1.5">
        <span className="text-xs text-fg-tertiary">
          {model.date}
        </span>
        {trip.drive_mode && model.mode && (
          <Badge size="sm" className={model.mode.className}>
            {model.mode.label}
          </Badge>
        )}
      </div>
      {(startLabel || endLabel) && (
        <div className="flex min-w-0 items-center gap-1 mb-2 text-sm font-medium text-fg">
          {startLabel && <span className="truncate">{startLabel}</span>}
          {startLabel && endLabel && <PiArrowFatLinesRight className="h-3.5 w-3.5 shrink-0 text-fg-tertiary" />}
          {endLabel && <span className="truncate">{endLabel}</span>}
        </div>
      )}
      <div className="flex items-center gap-3 text-xs text-fg-secondary flex-wrap">
        <span className="font-mono font-medium text-fg">{formatMiles(trip.distance_mi)}</span>
        <span className="text-fg-tertiary">·</span>
        <span>{formatDuration(trip.duration_min)}</span>
        {model.efficiency !== undefined && (
          <>
            <span className="text-fg-tertiary">·</span>
            <span className="font-mono">{model.efficiency}</span>
          </>
        )}
        {model.soc && (
          <span className="ml-auto font-mono text-fg-tertiary">
            {model.soc[0]} <PiArrowFatLinesRight className="inline h-3 w-3" /> {model.soc[1]}
          </span>
        )}
      </div>
      <TripTagBadges tags={trip.tags} />
      </button>
    </div>
  );
}
import {
  useTripSelection,
  toggleTripSelection,
  clearTripSelection,
  resetTripSelection,
  registerTripsInStore,
} from './tripSelectionStore';
import {
  resetTripTableState,
  setTripTablePage,
  setTripTablePageSize,
  setTripTableSearch,
  useTripTableState,
} from './tripTableStateStore';

const BATCH_ACTION_ICONS = { clear: Eraser, add: Plus, save: Save };

export function TripsMapWidget({ ctx }: { instance: WidgetInstance; ctx: WidgetCtx }) {
  const navigate = useNavigate();
  const { selectedIds, tripRegistry } = useTripSelection();
  const [highlightedRouteId, setHighlightedRouteId] = React.useState<string | null>(null);
  const { search } = useTripTableState();
  const isDark = useDocumentTheme();
  const accessToken = useAuth((state) => state.accessToken);
  const mapStyle: MapStyleMode = isDark ? 'dark' : 'light';
  const [mapStyleOverride, setMapStyleOverride] = React.useState<MapStyleMode | null>(null);
  const effectiveMapStyle: MapStyleMode = mapStyleOverride ?? mapStyle;
  const deferredSearch = React.useDeferredValue(search);
  const mapQuery = useTripMapRoutes(ctx.vehicleId, ctx.from, ctx.to, deferredSearch, ctx.tripTagFilter);
  const { routes, selectedRouteIds, key: routeKey } = React.useMemo(() => projectMapRoutes(mapQuery.data?.routes ?? [], selectedIds), [mapQuery.data, selectedIds]);
  const { ref, height } = useMeasuredWidgetHeight(360, 1);
  const basemap = useBasemapConfig();
  const userPreferences = useUserPreferences();

  React.useEffect(() => {
    resetTripSelection(`${ctx.vehicleId}::${ctx.from}::${ctx.to}`, { force: true });
    resetTripTableState(`${ctx.vehicleId}::${ctx.from}::${ctx.to}`, { force: true });
    setHighlightedRouteId(null);
  }, [ctx.vehicleId, ctx.from, ctx.to]);

  const selectedTripId = selectedRouteIds[0];

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <div ref={ref} className="relative min-h-0 flex-1">
        <div className="relative overflow-hidden rounded-lg border border-border bg-bg-surface" style={{ height }}>
          {!mapQuery.isLoading && !mapQuery.isError ? (
            <TripMapChart
              track={[]}
              routes={routes}
              selectedRouteIds={selectedRouteIds}
              highlightedRouteId={selectedRouteIds.includes(highlightedRouteId ?? '') ? highlightedRouteId : null}
              onRouteClick={toggleTripSelection}
              height={height}
              mapStyle={effectiveMapStyle}
              mapStylePreference={userPreferences.data?.map_style ?? 'follow-theme'}
              className="h-full w-full"
              accessToken={accessToken}
              basemapConfig={basemap.data}
              basemapError={basemap.isError ? 'Map configuration unavailable' : null}
              onBasemapRetry={() => void basemap.refetch()}
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center bg-bg-elevated text-sm text-fg-tertiary">
              Loading route map...
            </div>
          )}
          <button
            onClick={() => setMapStyleOverride((s) => (s ?? mapStyle) === 'dark' ? 'light' : 'dark')}
            aria-label={effectiveMapStyle === 'dark' ? 'Switch to light map' : 'Switch to dark map'}
            className="absolute bottom-2 right-2 z-10 flex h-8 w-8 items-center justify-center rounded-lg border border-accent bg-bg-surface text-accent shadow-lg transition-colors hover:bg-bg-elevated"
          >
            {effectiveMapStyle === 'dark' ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
          </button>
        </div>
      </div>

      <TripRouteKey
        routes={routeKey}
        trips={tripRegistry}
        highlightedId={highlightedRouteId}
        onHighlight={setHighlightedRouteId}
      />
      {selectedRouteIds.length > 0 ? (
        <div className="flex shrink-0 items-center justify-between gap-3 rounded-lg border border-accent/30 bg-accent/10 px-3 py-2 text-xs text-fg-secondary">
          {selectedRouteIds.length === 1 ? (
            <button
              type="button"
              className="group flex w-full items-center justify-center gap-2 rounded-md px-3 py-2 text-sm font-medium text-accent transition-colors hover:bg-accent/15"
              onClick={() => navigate({ to: '/trips/$tripId', params: { tripId: selectedTripId! } })}
            >
              Open trip
              <PiArrowFatLinesRight className="h-4 w-4 transition-transform group-hover:translate-x-0.5" />
            </button>
          ) : (
            <>
              <span>{selectedRouteIds.length} routes selected.</span>
              <button className="font-medium text-accent hover:underline" onClick={clearTripSelection}>Show all</button>
            </>
          )}
        </div>
      ) : mapQuery.isError ? (
        <div className="shrink-0 rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-fg-secondary">
          Unable to load route map data.
        </div>
      ) : routes.length === 0 ? (
        <div className="shrink-0 rounded-lg border border-border bg-bg-elevated px-3 py-2 text-xs text-fg-tertiary">
          {mapQuery.data?.total_trips
            ? 'No GPS route points in this timeframe.'
            : 'No trips in this timeframe.'}
        </div>
      ) : mapQuery.data?.missing_route_count ? (
        <div className="shrink-0 rounded-lg border border-border bg-bg-elevated px-3 py-2 text-xs text-fg-tertiary">
          {mapQuery.data.missing_route_count} trip{mapQuery.data.missing_route_count === 1 ? '' : 's'} have no GPS route.
        </div>
      ) : null}
    </div>
  );
}

export function TripsTableWidget({ ctx }: { instance: WidgetInstance; ctx: WidgetCtx }) {
  const navigate = useNavigate();
  const { page, pageSize, search } = useTripTableState();
  const deferredSearch = React.useDeferredValue(search);
  const { selectedIds, tripRegistry } = useTripSelection();
  const { tags: batchTagIds, pending: pendingBatchAction, stage: setBatchTagIds, run: runBatchTagAction, updateAssignments } = useTripTagBatch(ctx.vehicleId, selectedIds);
  const isMobile = useIsMobile();
  const { data, isLoading } = useTrips(ctx.vehicleId, ctx.from, ctx.to, page, pageSize, deferredSearch.trim(), ctx.tripTagFilter);
  const placesQuery = useQuery({
    queryKey: ['places'],
    queryFn: () => api.listPlaces(),
    staleTime: 5 * 60 * 1000,
  });
  const paginationState = pageNavigation(page, data);
  const selectedSet = new Set(selectedIds);
  const trips = (data?.items ?? []) as TripRow[];
  const hasVisibleTags = trips.some((trip) => (trip.tags?.length ?? 0) > 0);
  const columns = React.useMemo(
    () => createTripColumns(placesQuery.data ?? [], {
      includeTags: hasVisibleTags,
      onInfoClick: (tripId) => {
        navigate({ to: '/trips/$tripId', params: { tripId } });
      },
    }),
    [hasVisibleTags, navigate, placesQuery.data],
  );

  React.useEffect(() => {
    resetTripTableState(`${ctx.vehicleId}::${ctx.from}::${ctx.to}`, { force: true });
  }, [ctx.from, ctx.to, ctx.vehicleId]);

  const { commonTagIds, hasMixedTags } = React.useMemo(() => tripTagBatch(selectedIds, tripRegistry, trips), [selectedIds, tripRegistry, trips]);
  const batchActions = tripBatchActions(selectedIds.length, batchTagIds, commonTagIds, pendingBatchAction, updateAssignments.isPending);

  React.useEffect(() => {
    setBatchTagIds(commonTagIds);
  }, [commonTagIds, ctx.vehicleId, selectedIds.join('|')]);

  React.useEffect(() => {
    if (trips.length > 0) registerTripsInStore(trips);
  }, [trips]);

  const pagination = data ? (
    <div className="flex shrink-0 items-center justify-between border-t border-border pt-3">
      <p className="text-xs text-fg-tertiary">
        Page {page} of {paginationState.pages} · {data.total} trip{data.total === 1 ? '' : 's'}
      </p>
      <div className="flex gap-2">
        <button
          disabled={paginationState.previous.disabled}
          onClick={() => setTripTablePage(paginationState.previous.page)}
          className="rounded-lg border border-border px-3 py-1.5 text-xs transition-colors hover:bg-bg-elevated disabled:opacity-40"
        >
          Prev
        </button>
        <button
          disabled={paginationState.next.disabled}
          onClick={() => setTripTablePage(paginationState.next.page)}
          className="rounded-lg border border-border px-3 py-1.5 text-xs transition-colors hover:bg-bg-elevated disabled:opacity-40"
        >
          Next
        </button>
      </div>
    </div>
  ) : null;

  return (
    <div className="flex !h-auto min-h-full flex-col gap-3">
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-3">
        <label className="relative min-w-0 flex-1 sm:min-w-[14rem]">
          <span className="sr-only">Search trips</span>
          <input
            type="search"
            value={search}
            onChange={(event) => setTripTableSearch(event.target.value)}
            placeholder="Search start, destination, or tag"
            className="h-9 w-full rounded-lg border border-border bg-bg-surface px-3 text-sm text-fg outline-none placeholder:text-fg-tertiary focus:border-accent focus:ring-0"
          />
        </label>
        <div className="flex h-9 items-center gap-3">
          {selectedIds.length > 0 ? (
            <div className="inline-flex h-9 items-center gap-2 rounded-lg border border-accent/30 bg-accent/10 px-3 text-xs text-fg-secondary">
              <span>{selectedIds.length} selected</span>
              <button className="font-medium text-accent hover:underline" onClick={clearTripSelection}>
                Clear
              </button>
            </div>
          ) : null}
          {!isMobile && (
            <label className="flex h-9 items-center gap-2 text-xs text-fg-tertiary">
              Rows
              <SelectPicker
                className="min-w-[4.5rem]"
                value={String(pageSize)}
                onChange={(value) => {
                  const next = Number(value);
                  if (TABLE_PAGE_SIZES.includes(next as (typeof TABLE_PAGE_SIZES)[number])) {
                    setTripTablePageSize(next);
                  }
                }}
                aria-label="Trips per page"
                size="md"
                options={TABLE_PAGE_OPTIONS}
              />
            </label>
          )}
        </div>
      </div>

      {selectedIds.length > 0 && ctx.canManageTripTags ? (
        <div data-trip-selection-actions className="rounded-xl border border-border bg-bg-surface p-3">
          <div className="flex flex-wrap items-center gap-2">
            <div className="min-w-0 basis-full sm:basis-auto sm:flex-1">
              <TripTagPicker
                vehicleId={ctx.vehicleId}
                canManage
                selectedIds={batchTagIds}
                onChange={setBatchTagIds}
                label="Add tags to selected trips"
                mode="inline"
                mixed={hasMixedTags}
                disabled={updateAssignments.isPending}
              />
            </div>
            {batchActions.map((action) => {
              const Icon = BATCH_ACTION_ICONS[action.key];
              return (
              <Tooltip key={action.key} content={action.tooltip}>
                <Button
                  type="button"
                  variant={action.variant}
                  size="md"
                  className="h-11 flex-1 gap-2 px-3 sm:w-11 sm:flex-none sm:px-0"
                  aria-label={action.label}
                  disabled={action.disabled}
                  loading={action.loading}
                  onClick={() => void runBatchTagAction(action.key, action.mode, action.tags)}
                >
                  <Icon className="h-4 w-4" />
                  <span className="sm:hidden">{action.mobile}</span>
                </Button>
              </Tooltip>
              );
            })}
          </div>
          {updateAssignments.isError ? <p role="alert" className="mt-2 text-sm text-status-danger">Couldn’t update trip tags. Try again.</p> : null}
        </div>
      ) : null}

      {isMobile ? (
        <div className="flex flex-col gap-2">
          {isLoading
            ? Array.from({ length: 5 }).map((_, i) => (
                <div key={i} className="h-20 animate-pulse rounded-xl border border-border bg-bg-elevated/60" />
              ))
            : trips.length === 0
            ? <p className="py-8 text-center text-sm text-fg-tertiary">No trips found</p>
            : trips.map((trip) => (
                <TripCard
                  key={trip.id}
                  trip={trip}
                  isSelected={selectedSet.has(trip.id)}
                  onClick={() => toggleTripSelection(trip.id)}
                  onInfoClick={() => navigate({ to: '/trips/$tripId', params: { tripId: trip.id } })}
                />
              ))
          }
        </div>
      ) : (
        <DataTable
          data={trips}
          columns={columns}
          loading={isLoading}
          loadingRows={pageSize}
          onRowClick={(row) => toggleTripSelection(row.original.id)}
          getRowIsSelected={(row) => selectedSet.has(row.original.id)}
          emptyTitle="No trips found"
          emptyDescription={deferredSearch.trim() ? 'No trips match that start, destination, or tag.' : 'Trips will appear here once your vehicle has been driven.'}
          fixedLayout
          columnVisibilityMenu
          className="overflow-x-hidden"
        />
      )}

      {pagination}
    </div>
  );
}

registerWidget({
  componentType: 'custom',
  definitionId: 'trips.map',
  title: 'Trip Map',
  defaultSize: { w: 12, h: 10 },
  minSize: { w: 6, h: 6 },
  defaultOptions: {},
  component: TripsMapWidget,
});

registerWidget({
  componentType: 'custom',
  definitionId: 'trips.table',
  title: 'Trip History',
  defaultSize: { w: 12, h: 12 },
  minSize: { w: 6, h: 6 },
  defaultOptions: {},
  component: TripsTableWidget,
});
