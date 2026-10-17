import { useMobileChartController } from './mobileChartController';
import React from 'react';
import { createPortal } from 'react-dom';
import { Maximize2, RotateCw, X } from 'lucide-react';
import { ChartPicker, type ChartPickerOption } from '@panda/ui/primitives';
import { cn } from '@panda/ui/lib/utils';

interface MobileChartViewerProps {
  chartId: string;
  chartTitle: string;
  chartOptions: ChartPickerOption[];
  onChartChange: (chartId: string) => void;
  defaultChartId: string;
  onSetDefault: (chartId: string) => void;
  onClose: () => void;
  children: (height: number) => React.ReactNode;
}

export function MobileChartViewer({
  chartId,
  chartTitle,
  chartOptions,
  onChartChange,
  defaultChartId,
  onSetDefault,
  onClose,
  children,
}: MobileChartViewerProps) {
  const { rootRef, closeRef, close, isPortrait, chartHeight, showControls, onViewerPointerDownCapture, onViewerPointerMoveCapture, onViewerPointerUpCapture, onViewerKeyDown, cancelPointer } = useMobileChartController(onClose);
  if (typeof document === 'undefined') return null;

  return createPortal(
    <div
      ref={rootRef}
      role="dialog"
      aria-modal="true"
      aria-label={`${chartTitle} expanded chart`}
      aria-describedby="mobile-chart-viewer-instructions"
      tabIndex={-1}
      data-mobile-chart-viewer="true"
      className="fixed inset-0 z-[100] isolate h-[100dvh] w-[100dvw] overflow-hidden overscroll-none bg-bg-page touch-none"
      onPointerDownCapture={onViewerPointerDownCapture}
      onPointerMoveCapture={onViewerPointerMoveCapture}
      onPointerUpCapture={onViewerPointerUpCapture}
      onPointerCancelCapture={cancelPointer}
      onKeyDown={onViewerKeyDown}
    >
      <p id="mobile-chart-viewer-instructions" className="sr-only">Tap the chart, or press Enter or Space, to show or hide chart controls. Press Escape to close the expanded chart.</p>
      <div
        ref={(node) => {
          if (node) node.inert = !showControls;
        }}
        data-mobile-chart-controls="true"
        aria-hidden={!showControls}
        className={cn(
          'relative z-30 transition-opacity duration-200 motion-reduce:transition-none',
          showControls ? 'opacity-100' : 'pointer-events-none opacity-0',
        )}
      >
        <div className="absolute left-[max(0.75rem,env(safe-area-inset-left))] top-[max(0.75rem,env(safe-area-inset-top))]">
          {chartOptions.length > 1 ? (
            <ChartPicker
              variant="compact"
              value={chartId}
              options={chartOptions}
              onChange={onChartChange}
              searchValue=""
              onSearchChange={() => undefined}
              selectLabel="Choose chart"
              defaultValue={defaultChartId}
              onSetDefault={onSetDefault}
            />
          ) : (
            <p className="max-w-[min(16rem,calc(100vw-8rem))] truncate rounded-lg border border-border bg-bg-surface px-3 py-2 text-sm font-semibold text-fg shadow-sm">{chartTitle}</p>
          )}
        </div>
        <button
          ref={closeRef}
          type="button"
          onClick={close}
          className="absolute right-[max(0.75rem,env(safe-area-inset-right))] top-[max(0.75rem,env(safe-area-inset-top))] flex h-10 w-10 items-center justify-center rounded-lg border border-border bg-bg-surface text-fg-tertiary shadow-sm transition-colors hover:border-border-strong hover:text-fg focus:outline-none focus:ring-1 focus:ring-accent"
          aria-label="Close expanded chart"
        >
          <X className="h-5 w-5" />
        </button>
      </div>

      {isPortrait ? (
        <div className="m-[max(0.75rem,env(safe-area-inset-top))] flex h-[calc(100%-1.5rem)] flex-col items-center justify-center gap-6 rounded-2xl border border-accent bg-accent p-8 text-center text-fg-on-accent shadow-glow-button">
          <div className="relative flex h-20 w-20 items-center justify-center rounded-full border border-fg-on-accent/35 bg-fg-on-accent/10 shadow-sm">
            <RotateCw className="h-12 w-12" strokeWidth={2.5} aria-hidden="true" />
            <Maximize2 className="absolute -bottom-1 -right-1 h-7 w-7 rounded-full border border-accent bg-bg-surface p-1 text-accent shadow-sm" aria-hidden="true" />
          </div>
          <div>
            <p className="text-2xl font-bold tracking-tight">Rotate for a wider chart</p>
            <p className="mx-auto mt-3 max-w-sm text-base leading-6 text-fg-on-accent/90">Turn your phone sideways to explore {chartTitle} with the full interactive controls.</p>
          </div>
        </div>
      ) : (
        <div className={cn('h-full w-full', chartOptions.length > 1 && 'pt-1')} data-chart-presentation="mobile-viewer">
          {children(chartHeight)}
        </div>
      )}
    </div>,
    document.body,
  );
}
