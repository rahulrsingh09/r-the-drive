import * as React from 'react';
import { Calendar, ChevronDown, ChevronLeft, ChevronRight, Clock } from 'lucide-react';
import type { DashboardTimeframe, PresetKey } from '@panda/types';
import { addMonths, format, getYear, isSameMonth, startOfMonth } from 'date-fns';
import { cn } from '../lib/utils';
import { formatAppDate, formatAppDateTime, getAppDateParts } from '../lib/dateTime';
import { calendarCells } from '../lib/dateRange';
import { useDateRangeEditor } from '../hooks/useDateRangeEditor';
import { useDisclosure, useDismissLayer } from '../hooks/useOverlayInteraction';
export { presetToRange } from '../lib/dateRange';
import { SelectPicker } from './SelectPicker';

export type { BoundedPresetKey, DashboardTimeframe, DateRange, PresetKey } from '@panda/types';

const PRESETS: Array<{ key: PresetKey; label: string }> = [
  { key: '1h', label: 'Last 1h' },
  { key: '6h', label: 'Last 6h' },
  { key: '12h', label: 'Last 12h' },
  { key: '24h', label: 'Last 24h' },
  { key: '7d', label: 'Last 7 days' },
  { key: '30d', label: 'Last 30 days' },
  { key: '90d', label: 'Last 90 days' },
  { key: '1y', label: 'Last year' },
  { key: 'lifetime', label: 'Lifetime' },
];

const MONTH_OPTIONS = Array.from({ length: 12 }, (_, index) => ({
  value: index,
  label: format(new Date(2026, index, 1), 'MMMM'),
}));

const YEAR_OPTIONS = Array.from({ length: 31 }, (_, offset) => {
  const year = getYear(new Date()) - 15 + offset;
  return { value: year, label: String(year) };
});

export interface DateRangePickerProps {
  timeframe: DashboardTimeframe;
  onChange: (timeframe: DashboardTimeframe) => void;
  className?: string;
  triggerClassName?: string;
}

export function DateRangePicker({
  timeframe,
  onChange,
  className,
  triggerClassName,
}: DateRangePickerProps) {
  const { open, setOpen } = useDisclosure();
  const editor = useDateRangeEditor(timeframe);
  const { state, send } = editor;
  const { expanded: customExpanded, month: monthCursor, target, error: inputError } = state;
  const { date: customFrom, input: customFromInput } = state.from;
  const { date: customTo, input: customToInput } = state.to;
  const ref = React.useRef<HTMLDivElement>(null);
  useDismissLayer({ roots: [ref], pointer: 'mousedown', requireRoot: true, dismiss: () => setOpen(false) });
  const displayLabel = timeframe.kind === 'custom' ? `${formatAppDateTime(timeframe.from)} - ${formatAppDateTime(timeframe.to)}`
    : PRESETS.find(preset => preset.key === (timeframe.kind === 'lifetime' ? 'lifetime' : timeframe.preset))?.label ?? 'Custom range';
  function applyCustomRange() {
    const range = editor.range();
    if (range) { onChange({ kind: 'custom', ...range }); setOpen(false); }
  }
  function choosePreset(key: PresetKey) {
    send({ type: 'preset' });
    onChange(key === 'lifetime' ? { kind: 'lifetime' } : { kind: 'preset', preset: key });
    setOpen(false);
  }

  return (
    <div ref={ref} className={cn('relative', className)}>
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-haspopup="dialog"
        aria-expanded={open}
        className={cn(
          'flex h-9 items-center gap-2 rounded-lg border border-border bg-bg-elevated px-3 text-sm',
          'text-fg-secondary transition-colors duration-150 hover:border-border-strong hover:text-fg',
          triggerClassName
        )}
      >
        <Calendar className="h-3.5 w-3.5 text-fg-tertiary" />
        {displayLabel}
        <ChevronDown
          className={cn('h-3.5 w-3.5 text-fg-tertiary transition-transform', open && 'rotate-180')}
        />
      </button>

      {open ? (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-bg-page/70 p-4 backdrop-blur-sm sm:absolute sm:inset-auto sm:right-0 sm:top-full sm:mt-1 sm:block sm:w-max sm:bg-transparent sm:p-0 sm:backdrop-blur-none"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget) setOpen(false);
          }}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-label="Select timeframe"
            className="max-h-[calc(100dvh-2rem)] w-full max-w-md space-y-3 overflow-y-auto overscroll-contain rounded-xl border border-border bg-bg-elevated p-3 shadow-lg sm:max-h-[calc(100vh-1rem)] sm:min-w-[380px] sm:max-w-[calc(100vw-1rem)]"
          >
            <div className="grid grid-cols-2 gap-1">
              {PRESETS.map((preset) => (
                <button
                  key={preset.key}
                  type="button"
                  onClick={() => choosePreset(preset.key)}
                  className={cn(
                    'rounded-md px-3 py-1.5 text-left text-sm transition-colors',
                    (timeframe.kind === 'lifetime' ? preset.key === 'lifetime' : timeframe.kind === 'preset' && timeframe.preset === preset.key)
                      ? 'bg-accent-muted text-accent'
                      : 'text-fg-secondary hover:bg-bg-surface hover:text-fg'
                  )}
                >
                  {preset.label}
                </button>
              ))}
              <button
                type="button"
                onClick={() => send({ type: 'expand' })}
                className={cn(
                  'flex items-center justify-between rounded-md px-3 py-1.5 text-left text-sm transition-colors',
                  customExpanded || timeframe.kind === 'custom'
                    ? 'bg-accent-muted text-accent'
                    : 'text-fg-secondary hover:bg-bg-surface hover:text-fg'
                )}
              >
                <span>Custom Range</span>
                <ChevronDown
                  className={cn('h-3.5 w-3.5 transition-transform', customExpanded && 'rotate-180')}
                />
              </button>
            </div>

            {customExpanded ? (
              <div className="space-y-3 border-t border-border pt-3">
                <div className="flex items-center gap-2 text-xs font-medium uppercase tracking-wider text-fg-tertiary">
                  <Calendar className="h-3.5 w-3.5" />
                  Custom range
                </div>

                <div className="grid gap-2 sm:grid-cols-2">
                  <DateTimeRow
                    label="From"
                    active={target === 'from'}
                    value={customFrom}
                    inputValue={customFromInput}
                    onInputChange={value => send({ type: 'input', target: 'from', value })}
                    onTarget={() => send({ type: 'target', target: 'from' })}
                    onHour={hour => send({ type: 'time', target: 'from', patch: { hour } })}
                    onMinute={minute => send({ type: 'time', target: 'from', patch: { minute } })}
                  />
                  <DateTimeRow
                    label="To"
                    active={target === 'to'}
                    value={customTo}
                    inputValue={customToInput}
                    onInputChange={value => send({ type: 'input', target: 'to', value })}
                    onTarget={() => send({ type: 'target', target: 'to' })}
                    onHour={hour => send({ type: 'time', target: 'to', patch: { hour } })}
                    onMinute={minute => send({ type: 'time', target: 'to', patch: { minute } })}
                  />
                </div>

                <ThemedCalendar
                  month={monthCursor}
                  selected={target === 'from' ? customFrom : customTo}
                  from={customFrom}
                  to={customTo}
                  onMonth={date => send({ type: 'month', date })}
                  onPick={date => send({ type: 'day', date })}
                />

                {inputError ? (
                  <p className="text-xs text-status-negative">{inputError}</p>
                ) : (
                  <p className="text-xs text-fg-tertiary">
                    Applying: {formatAppDateTime(customFrom <= customTo ? customFrom : customTo)} -{' '}
                    {formatAppDateTime(customFrom <= customTo ? customTo : customFrom)}
                  </p>
                )}

                <button
                  type="button"
                  onClick={applyCustomRange}
                  className="h-8 w-full rounded-lg bg-accent px-3 text-sm font-medium text-fg-on-accent hover:bg-accent/90"
                >
                  Apply custom range
                </button>
              </div>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function DateTimeRow({
  label,
  value,
  inputValue,
  active,
  onInputChange,
  onTarget,
  onHour,
  onMinute,
}: {
  label: string;
  value: Date;
  inputValue: string;
  active: boolean;
  onInputChange: (value: string) => void;
  onTarget: () => void;
  onHour: (hours: number) => void;
  onMinute: (minutes: number) => void;
}) {
  const appParts = getAppDateParts(value);
  const hours = appParts?.hour ?? value.getHours();
  const minutes = snapMinuteToOption(appParts?.minute ?? value.getMinutes());

  return (
    <div
      className={cn(
        'w-full min-w-0 rounded-lg border px-2 py-2 text-left',
        active
          ? 'border-accent bg-accent-muted/40'
          : 'border-border bg-bg-surface hover:border-border-strong'
      )}
    >
      <button type="button" onClick={onTarget} className="w-full text-left">
        <div className="flex items-baseline justify-between gap-2">
          <div className="text-[11px] uppercase tracking-wide text-fg-tertiary">{label}</div>
          <div className="text-[10px] font-medium uppercase tracking-[0.16em] text-fg-tertiary">
            mm/dd/yyyy
          </div>
        </div>
        <div className="mt-1 text-xs text-fg">{formatAppDate(value, { weekday: 'short' })}</div>
      </button>
      <div className="mt-2 grid gap-2">
        <input
          value={inputValue}
          onChange={(event) => onInputChange(event.target.value)}
          onFocus={onTarget}
          placeholder="6/3/26"
          className="h-8 w-full min-w-0 rounded-md border border-border bg-bg-elevated px-2 text-xs text-fg outline-none placeholder:text-fg-tertiary focus:border-accent"
        />
        <div className="flex items-center gap-2">
          <Clock className="h-3.5 w-3.5 text-fg-tertiary" />
          <SelectPicker
            className="min-w-0 flex-1"
            triggerClassName="h-7 rounded-md px-1 text-xs"
            value={String(hours)}
            onChange={(value) => onHour(Number(value))}
            aria-label={`${label} hour`}
            size="sm"
            options={Array.from({ length: 24 }, (_, index) => ({
              value: String(index),
              label: index.toString().padStart(2, '0'),
            }))}
          />
          <span className="text-fg-tertiary">:</span>
          <SelectPicker
            className="min-w-0 flex-1"
            triggerClassName="h-7 rounded-md px-1 text-xs"
            value={String(minutes)}
            onChange={(value) => onMinute(Number(value))}
            aria-label={`${label} minute`}
            size="sm"
            options={[0, 15, 30, 45].map((minute) => ({
              value: String(minute),
              label: minute.toString().padStart(2, '0'),
            }))}
          />
        </div>
      </div>
    </div>
  );
}

function ThemedCalendar({
  month,
  selected,
  from,
  to,
  onMonth,
  onPick,
}: {
  month: Date;
  selected: Date;
  from: Date;
  to: Date;
  onMonth: (next: Date) => void;
  onPick: (date: Date) => void;
}) {
  const cells = calendarCells(month, selected, from, to);

  return (
    <div className="rounded-lg border border-border bg-bg-surface p-2">
      <div className="mb-2 flex items-center justify-between gap-2">
        <button
          type="button"
          onClick={() => onMonth(startOfMonth(addMonths(month, -1)))}
          className="rounded-md p-1 hover:bg-bg-elevated"
        >
          <ChevronLeft className="h-4 w-4 text-fg-secondary" />
        </button>
        <div className="flex items-center gap-2">
          <SelectPicker
            aria-label="Month"
            value={String(month.getMonth())}
            onChange={(value) => onMonth(new Date(month.getFullYear(), Number(value), 1))}
            size="sm"
            options={MONTH_OPTIONS.map((option) => ({
              value: String(option.value),
              label: option.label,
            }))}
          />
          <SelectPicker
            aria-label="Year"
            value={String(month.getFullYear())}
            onChange={(value) => onMonth(new Date(Number(value), month.getMonth(), 1))}
            size="sm"
            options={YEAR_OPTIONS.map((option) => ({
              value: String(option.value),
              label: option.label,
            }))}
          />
        </div>
        <button
          type="button"
          onClick={() => onMonth(startOfMonth(addMonths(month, 1)))}
          className="rounded-md p-1 hover:bg-bg-elevated"
        >
          <ChevronRight className="h-4 w-4 text-fg-secondary" />
        </button>
      </div>
      <div className="grid grid-cols-7 gap-1 text-center text-[11px] text-fg-tertiary">
        {['S', 'M', 'T', 'W', 'T', 'F', 'S'].map((day, index) => (
          <div key={`${day}-${index}`}>{day}</div>
        ))}
      </div>
      <div className="mt-1 grid grid-cols-7 gap-1">
        {cells.map((cell, index) => {
          if (!cell) return <div key={`blank-${index}`} className="h-8" />;
          const { date, inRange, selectedDay } = cell;
          return (
            <button
              key={date.toISOString()}
              type="button"
              onClick={() => onPick(date)}
              className={cn(
                'h-8 rounded-md text-xs',
                !isSameMonth(date, month) && 'text-fg-tertiary/40',
                inRange
                  ? 'bg-accent-muted text-fg'
                  : 'text-fg-secondary hover:bg-bg-elevated hover:text-fg',
                selectedDay && 'ring-1 ring-accent text-accent'
              )}
            >
              {date.getDate()}
            </button>
          );
        })}
      </div>
    </div>
  );
}

function snapMinuteToOption(value: number) {
  return [0, 15, 30, 45].sort((first, second) => Math.abs(first - value) - Math.abs(second - value))[0]!;
}
