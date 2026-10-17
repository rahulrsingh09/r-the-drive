import { usePhantomDrainTable } from './usePhantomDrainTable';
import React from 'react';
import { DashboardChartWidget, SensorChipSummary, type WidgetInstance } from '@panda/dashboards';
import { usePhantomDrainPeriods, useVehicleHealth } from '@panda/hooks';
import type { PhantomDrainPeriod } from '@panda/types';
import { DataTable, TableControls, PhantomDrainPeriodCard, phantomDrainColumns } from '@panda/ui/tables';
import { formatPercent } from '@panda/ui/lib/utils';
import { DashboardPageShell, type DashboardPageShellRenderState } from './DashboardPageShell';
import type { DashboardPageProps } from './DashboardPage';
import { buildPhantomDrainSummaryCards, summarizePhantomDrainPeriods } from './phantomDrainSummary';
import { ParkedEnergyPanel } from './ParkedEnergyPanel';

const PHANTOM_DRAIN_CHART_INSTANCE: WidgetInstance = {
  id: 'd2000002-0000-0000-0000-000000000010',
  componentType: 'chart',
  definitionId: 'catalog',
  layout: { x: 0, y: 0, w: 12, h: 10 },
  options: {
    page: 'battery',
    chartId: 'phantom-drain',
    chartIds: ['soc-history', 'range-history', 'phantom-drain', 'battery-capacity-mileage', 'projected-range-mileage'],
    showPicker: true,
  },
};

const PERIOD_SEARCH_FIELDS = [
  ['duration_hours', 'decimal', 1, '', ' h'],
  ['sleep_share_pct', 'ratio', 0, '', ''],
  ['state_coverage_pct', 'ratio', 0, '', ' coverage'],
  ['soc_start', 'percent', 0, '', ''],
  ['soc_end', 'percent', 0, '', ''],
  ['soc_lost_pct', 'percent', 2, '-', ''],
  ['drain_pct_per_hour', 'percent', 2, 'drain ', ' / h'],
  ['range_lost_mi', 'decimal', 1, '', ' mi'],
  ['energy_drained_kwh', 'decimal', 2, '', ' kWh'],
  ['avg_power_w', 'decimal', 0, '', ' W'],
] as const;

function formatPeriodSearchText(period: PhantomDrainPeriod) {
  const parts: string[] = [];
  const append = (text: string | null | undefined) => { if (text) parts.push(text); };
  append(period.period_start);
  append(period.period_end);
  for (const [field, format, digits, prefix, suffix] of PERIOD_SEARCH_FIELDS) {
    const value = period[field];
    if (value == null) continue;
    let text: string | null;
    switch (format) {
      case 'decimal': text = value.toFixed(digits); break;
      case 'percent': text = formatPercent(value, digits); break;
      case 'ratio': text = Number.isNaN(value) ? null : formatPercent(value * 100, digits); break;
    }
    append(prefix || suffix ? `${prefix}${text}${suffix}` : text);
  }
  if (period.has_reduced_range) append('reduced range');
  append(period.validation_reason);
  return parts.join(' ').toLowerCase();
}

function PhantomDrainContent({ state }: { state: DashboardPageShellRenderState }) {
  const { vehicleId, ctx } = state;
  const { data: health } = useVehicleHealth(vehicleId);
  const { data, isLoading } = usePhantomDrainPeriods(vehicleId, ctx.from, ctx.to, 500, 6);
  const parallaxCollectorRunning =
    health?.vehicle_id === vehicleId && health.extended_telemetry.collector?.running === true;
  const periods = data?.periods ?? [];
  const { search, setSearch, page, pageSize, setPageSize, filteredPeriods, pagedPeriods, totalPages, previousPage, nextPage } = usePhantomDrainTable(
    periods, JSON.stringify([ctx.from, ctx.to, ctx.vehicleId]), formatPeriodSearchText,
  );

  const summary = React.useMemo(() => summarizePhantomDrainPeriods(periods), [periods]);
  const summaryCards = React.useMemo(() => buildPhantomDrainSummaryCards(summary), [summary]);
  return (
    <div className="grid gap-4 min-w-0">
      {parallaxCollectorRunning ? (
        <ParkedEnergyPanel vehicleId={vehicleId} from={ctx.from} to={ctx.to} />
      ) : null}

      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="text-base font-semibold text-fg">Panda battery-change estimate</h2>
          <p className="text-xs text-fg-tertiary">
            Derived from validated parked sessions and retained when Rivian&apos;s breakdown is unavailable.
          </p>
        </div>
      </div>

      <div className="grid min-w-0 grid-cols-2 gap-3 xl:grid-cols-4">
        {summaryCards.map(({ key, dataAccent, title, value, icon, ...optional }) => (
          <SensorChipSummary
            key={key}
            title={title}
            value={value}
            icon={icon}
            dataAccent={dataAccent}
            valueColor="data"
            valueSize="sm"
            {...optional}
          />
        ))}
      </div>

      <section className="min-w-0" aria-label="Battery history">
        <p className="mb-2 text-xs text-fg-secondary md:hidden">Tap or drag across the chart for details</p>
        <div className="h-[340px] min-w-0 sm:h-[380px]" data-testid="phantom-drain-chart-section">
        <DashboardChartWidget instance={PHANTOM_DRAIN_CHART_INSTANCE} ctx={ctx} />
        </div>
      </section>

      <div data-testid="phantom-drain-periods-table" className="min-w-0 rounded-xl border border-border p-3">
        <h2 className="mb-3 text-base font-semibold text-fg">Parked periods</h2>
        <TableControls
          search={search}
          onSearchChange={setSearch}
          searchPlaceholder="Search periods"
          rowsPerPage={pageSize}
          rowsPerPageOptions={[15, 25, 50, 100]}
          onRowsPerPageChange={setPageSize}
          page={page}
          totalPages={totalPages}
          totalItems={filteredPeriods.length}
          itemLabel="period"
        />

        <DataTable
          data={pagedPeriods}
          columns={phantomDrainColumns}
          loading={isLoading}
          loadingRows={pageSize}
          emptyTitle={search ? 'No matching phantom drain periods' : 'No phantom drain periods'}
          emptyDescription={search ? 'No parked periods match that search.' : 'No qualifying parked periods were found in this date range.'}
          columnVisibilityMenu
          fixedLayout
          defaultHiddenColumns={['range_lost_per_hour_mi', 'state_coverage_pct', 'validation_reason']}
          renderMobileRow={(row) => <PhantomDrainPeriodCard period={row.original} />}
        />

        <div className="mt-3 flex flex-wrap items-center justify-between gap-3 border-t border-border pt-3">
          <p className="text-xs text-fg-tertiary">
            Showing {pagedPeriods.length} of {filteredPeriods.length} period{filteredPeriods.length === 1 ? '' : 's'}
          </p>
          <div className="flex gap-2">
            <button
              type="button"
              disabled={page <= 1}
              onClick={previousPage}
              className="min-h-11 rounded-lg border border-border px-4 py-2 text-xs transition-colors hover:bg-bg-elevated disabled:opacity-40"
            >
              Prev
            </button>
            <button
              type="button"
              disabled={page >= totalPages || totalPages <= 1}
              onClick={nextPage}
              className="min-h-11 rounded-lg border border-border px-4 py-2 text-xs transition-colors hover:bg-bg-elevated disabled:opacity-40"
            >
              Next
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

export function BatteryPhantomDrainPage({ navKey, slug, title }: DashboardPageProps) {
  return (
    <DashboardPageShell
      navKey={navKey}
      slug={slug}
      title={title}
      renderBeforeDashboard={(state) => <PhantomDrainContent state={state} />}
      renderDashboard={() => false}
    />
  );
}
