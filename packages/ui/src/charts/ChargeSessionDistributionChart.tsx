import * as React from 'react';
import { CHART_BAR_STYLE, CHART_COLORS, CHART_FONT } from './ChartProvider';
import { ChartSkeleton } from '../primitives/Skeleton';
import { formatSmartNumber } from '../lib/utils';
import { measureChartText, selectAdaptiveAxisLabelIndices, selectValueLabelIndices } from './chartLabelLayout';

export interface ChargeSessionDistributionBand {
  label: string;
  count: number;
  averageRateKw: number | null;
}

export interface ChargeSessionDistributionChartProps {
  bands: ChargeSessionDistributionBand[];
  height?: number;
  loading?: boolean;
  emptyTitle?: string | undefined;
}

function formatCount(value: number) {
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

function formatRate(value: number | null) {
  return value == null || !Number.isFinite(value) ? '-' : `${formatSmartNumber(value, Math.abs(value) >= 100 ? 0 : 1)} kW`;
}

export function ChargeSessionDistributionChart({
  bands,
  height = 280,
  loading = false,
  emptyTitle = 'No charging sessions for this period',
}: ChargeSessionDistributionChartProps) {
  if (loading) {
    return <ChartSkeleton height={height} />;
  }

  if (!bands.some(band => band.count > 0 || band.averageRateKw != null)) {
    return (
      <div
        className="flex items-center justify-center rounded-lg border border-dashed border-border text-xs text-fg-tertiary"
        style={{ height }}
      >
        {emptyTitle}
      </div>
    );
  }

  const width = 960;
  const chartHeight = Math.max(220, height);
  const margin = { top: 18, right: 96, bottom: 64, left: 96 };
  const innerWidth = width - margin.left - margin.right;
  const innerHeight = chartHeight - margin.top - margin.bottom;
  const domain = bands.reduce((range, band) => ({ count: Math.max(range.count, band.count), rate: Math.max(range.rate, band.averageRateKw ?? 0) }), { count: 1, rate: 1 });
  const maxCount = domain.count;
  const maxRate = domain.rate;
  const bandWidth = innerWidth / Math.max(1, bands.length);
  const barWidth = Math.min(72, bandWidth * 0.56);
  const geometry = bands.map((band, index) => {
    const centerX = margin.left + bandWidth * index + bandWidth / 2;
    const barTop = margin.top + innerHeight - (band.count / maxCount) * innerHeight;
    return { band, centerX, barTop, barHeight: Math.max(0, margin.top + innerHeight - barTop),
      ratePointY: band.averageRateKw == null ? null : margin.top + innerHeight - (band.averageRateKw / maxRate) * innerHeight };
  });
  const linePoints = geometry.flatMap(row => row.ratePointY == null ? [] : [`${row.centerX},${row.ratePointY}`]).join(' ');
  const tickFractions = [0, 0.25, 0.5, 0.75, 1];
  const axisLabelIndices = new Set(selectAdaptiveAxisLabelIndices(bands.map(band => band.label), geometry.map(row => row.centerX)));
  const valueLabelIndices = new Set(selectValueLabelIndices(geometry.map((row, index) => ({ index, value: row.band.count, x: row.centerX, y: row.barTop - 8, width: measureChartText(formatCount(row.band.count)) }))));

  return (
    <div className="rounded-lg border border-border bg-surface-1 p-3">
      <svg
        aria-label="Charging session distribution"
        role="img"
        viewBox={`0 0 ${width} ${chartHeight}`}
        width="100%"
        height={chartHeight}
      >
        {tickFractions.map((fraction) => {
          const y = margin.top + innerHeight - fraction * innerHeight;
          return (
            <g key={`grid-${fraction}`}>
              <line x1={margin.left} y1={y} x2={width - margin.right} y2={y} stroke={CHART_COLORS.grid} strokeWidth={1} />
              <text x={margin.left - 14} y={y + 4} textAnchor="end" fill={CHART_COLORS.muted} fontFamily={CHART_FONT.fontFamily} fontSize={CHART_FONT.fontSize} fontWeight={CHART_FONT.fontWeight}>
                {formatCount(maxCount * fraction)}
              </text>
              <text x={width - margin.right + 14} y={y + 4} textAnchor="start" fill={CHART_COLORS.muted} fontFamily={CHART_FONT.fontFamily} fontSize={CHART_FONT.fontSize} fontWeight={CHART_FONT.fontWeight}>
                {formatRate(maxRate * fraction)}
              </text>
            </g>
          );
        })}

        {geometry.map(({ band, centerX, barTop, barHeight, ratePointY }, index) => {

          return (
            <g key={band.label}>
              <rect
                x={centerX - barWidth / 2}
                y={barTop}
                width={barWidth}
                height={barHeight}
                rx={CHART_BAR_STYLE.radius}
                fill={CHART_COLORS.accent}
                opacity={CHART_BAR_STYLE.fillOpacity}
              />
              {band.count > 0 && valueLabelIndices.has(index) ? (
                <text data-testid="charge-distribution-value-label" x={centerX} y={barTop - 8} textAnchor="middle" fill={CHART_COLORS.accent} fontFamily={CHART_FONT.fontFamily} fontSize={CHART_FONT.fontSize} fontWeight={CHART_FONT.fontWeight}>
                  {band.count}
                </text>
              ) : null}
              {ratePointY != null && (
                <circle cx={centerX} cy={ratePointY} r={5} fill={CHART_COLORS.sky} stroke={CHART_COLORS.sky} />
              )}
              {axisLabelIndices.has(index) ? (
                <text data-testid="charge-distribution-axis-label" x={centerX} y={chartHeight - 24} textAnchor="middle" fill={CHART_COLORS.muted} fontFamily={CHART_FONT.fontFamily} fontSize={CHART_FONT.fontSize} fontWeight={CHART_FONT.fontWeight}>
                  {band.label}
                </text>
              ) : null}
            </g>
          );
        })}

        {linePoints && (
          <polyline
            points={linePoints}
            fill="none"
            stroke={CHART_COLORS.sky}
            strokeWidth={3}
            strokeLinejoin="round"
            strokeLinecap="round"
          />
        )}

        <text
          x={24}
          y={margin.top + innerHeight / 2}
          fill={CHART_COLORS.accent}
          fontFamily={CHART_FONT.fontFamily}
          fontSize={CHART_FONT.fontSize + 1}
          fontWeight={700}
          textAnchor="middle"
          transform={`rotate(-90 24 ${margin.top + innerHeight / 2})`}
        >
          Sessions
        </text>
        <text
          x={width - 24}
          y={margin.top + innerHeight / 2}
          fill={CHART_COLORS.sky}
          fontFamily={CHART_FONT.fontFamily}
          fontSize={CHART_FONT.fontSize + 1}
          fontWeight={700}
          textAnchor="middle"
          transform={`rotate(-90 ${width - 24} ${margin.top + innerHeight / 2})`}
        >
          Avg charge rate
        </text>
      </svg>
    </div>
  );
}
