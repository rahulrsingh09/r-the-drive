import { selectionAction } from '../lib/controlEvents';
import * as React from 'react';
import { Check, ChevronDown, Search, Star } from 'lucide-react';
import { cn } from '../lib/utils';
import { useDisclosure, useDismissLayer } from '../hooks/useOverlayInteraction';

export interface ChartPickerOption {
  value: string;
  label: string;
}

export interface ChartPickerProps<TValue extends string = string> {
  value: TValue;
  options: Array<ChartPickerOption & { value: TValue }>;
  onChange: (value: TValue) => void;
  searchValue: string;
  onSearchChange: (value: string) => void;
  searchPlaceholder?: string;
  selectLabel?: string;
  className?: string;
  trailing?: React.ReactNode;
  defaultValue?: TValue;
  onSetDefault?: (value: TValue) => void;
  variant?: 'default' | 'compact';
}

export function ChartPicker<TValue extends string = string>({
  value,
  options,
  onChange,
  searchValue,
  onSearchChange,
  searchPlaceholder = 'Search charts',
  selectLabel = 'Chart',
  className,
  trailing,
  defaultValue,
  onSetDefault,
  variant = 'default',
}: ChartPickerProps<TValue>) {
  const { open: isOpen, setOpen: setIsOpen, show, hide, toggle } = useDisclosure();
  const rootRef = React.useRef<HTMLDivElement>(null);
  const query = searchValue.trim().toLowerCase();
  const selection = options.reduce((result, option) => {
    if (option.value === value) result.selected.push(option);
    if (!query || option.label.toLowerCase().includes(query)) result.matches.push(option);
    return result;
  }, { selected: [] as typeof options, matches: [] as typeof options });
  const visibleOptions = selection.matches.some(option => option.value === value)
    ? selection.matches : [...selection.selected, ...selection.matches];
  const selectedOption = selection.selected[0] ?? options[0];
  useDismissLayer({ roots: [rootRef], pointer: 'mousedown', dismiss: () => setIsOpen(false) });

  const handleSelect = selectionAction(onChange, hide);
  const search = selectionAction(onSearchChange, show);

  const optionViews = new Map(options.map(option => [option, {
    selected: option.value === value,
    default: option.value === defaultValue,
    select: () => handleSelect(option.value),
  }]));
  function renderDefaultButton(option: ChartPickerOption & { value: TValue }) {
    const view = optionViews.get(option)!;
    const label = `Set ${option.label} as default`;
    const state = view.default
      ? { label: `${option.label} is the default chart`, title: 'Default chart', className: 'text-accent sm:opacity-100', fill: 'currentColor' }
      : { label, title: label, className: 'text-fg-tertiary hover:bg-bg-elevated hover:text-accent', fill: 'none' };
    return onSetDefault && <button type="button" aria-label={state.label} title={state.title} aria-pressed={view.default} disabled={view.default}
      onClick={event => { event.stopPropagation(); onSetDefault(option.value); }}
      className={cn('flex h-8 w-8 shrink-0 items-center justify-center rounded-md transition-colors focus:outline-none focus-visible:ring-1 focus-visible:ring-accent', 'sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100', state.className)}>
      <Star className="h-4 w-4" fill={state.fill} />
    </button>;
  }
  function renderOption(option: ChartPickerOption & { value: TValue }) {
    const view = optionViews.get(option)!;
    const compact = variant === 'compact';
    const duration = compact ? 'transition-colors' : 'transition-all';
    const padding = compact ? 'py-2.5' : 'py-2';
    return <div key={option.value} role="option" aria-selected={view.selected} onClick={view.select}
      className={cn(`group flex w-full items-center justify-between gap-1 rounded-md text-sm text-fg ${duration}`, 'hover:bg-bg-elevated', view.selected && 'bg-accent/10 text-accent')}>
      <button type="button" className={`flex min-w-0 flex-1 items-center justify-between gap-3 rounded-md px-3 ${padding} text-left focus:outline-none focus-visible:ring-1 focus-visible:ring-accent`}>
        <span className="truncate">{option.label}</span>
        {view.selected ? <Check className="h-4 w-4 shrink-0" /> : null}
      </button>
      {renderDefaultButton(option)}
    </div>;
  }

  if (variant === 'compact') {
    return (
      <div ref={rootRef} className={cn('relative', className)}>
        <button
          type="button"
          aria-label={selectLabel}
          aria-haspopup="listbox"
          aria-expanded={isOpen}
          onClick={toggle}
          className={cn(
            'flex h-10 max-w-[min(16rem,calc(100vw-8rem))] items-center justify-between gap-2 rounded-lg border border-border bg-bg-surface px-3 text-left text-sm font-medium text-fg',
            'transition-colors hover:border-border-strong focus:border-accent focus:outline-none focus:ring-0 focus-visible:!outline-none focus-visible:!outline-offset-0 focus-visible:ring-0',
          )}
        >
          <span className="truncate">{selectedOption?.label ?? 'Select chart'}</span>
          <ChevronDown className={cn('h-4 w-4 shrink-0 text-fg-tertiary transition-transform', isOpen && 'rotate-180')} />
        </button>
        {isOpen ? (
          <div className="absolute left-0 top-[calc(100%+0.375rem)] z-40 min-w-56 max-w-[calc(100vw-2rem)] overflow-hidden rounded-lg border border-border bg-bg-surface p-1 shadow-lg">
            <div role="listbox">
              {options.map(renderOption)}
            </div>
          </div>
        ) : null}
      </div>
    );
  }

  return (
    <div ref={rootRef} className={cn('relative mb-3 grid w-full gap-2', trailing ? 'grid-cols-[minmax(0,1fr)_auto] sm:grid-cols-[1fr_minmax(0,3fr)_auto]' : 'grid-cols-1 sm:grid-cols-4', className)}>
      <label className={cn('relative', trailing ? 'col-span-2 sm:col-span-1' : '')}>
        <span className="sr-only">Search charts</span>
        <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-fg-tertiary" />
        <input
          value={searchValue}
          onFocus={show}
          onChange={event => search(event.target.value)}
          placeholder={searchPlaceholder}
          className={cn(
            'h-11 sm:h-9 w-full rounded-lg border border-border bg-bg-surface pl-9 pr-3 text-sm text-fg',
            'placeholder:text-fg-tertiary transition-colors',
            'hover:border-border-strong focus:border-accent focus:outline-none focus:ring-0 focus-visible:!outline-none focus-visible:!outline-offset-0 focus-visible:ring-0',
          )}
        />
      </label>
      <div>
        <span className="sr-only">{selectLabel}</span>
        <button
          type="button"
          aria-label={selectLabel}
          aria-haspopup="listbox"
          aria-expanded={isOpen}
          onClick={toggle}
          className={cn(
            'flex min-h-11 sm:min-h-9 w-full items-center justify-between gap-2 rounded-lg border border-border bg-bg-surface px-3 py-2 sm:py-1 text-left text-sm text-fg',
            'transition-colors hover:border-border-strong focus:border-accent focus:outline-none focus:ring-0 focus-visible:!outline-none focus-visible:!outline-offset-0 focus-visible:ring-0',
          )}
        >
          <span className="min-w-0 break-words">{selectedOption?.label ?? 'Select chart'}</span>
          <ChevronDown className={cn('h-4 w-4 shrink-0 text-fg-tertiary transition-transform', isOpen && 'rotate-180')} />
        </button>
      </div>
      {trailing ? <div className="flex items-center">{trailing}</div> : null}

      {isOpen ? (
        <div
          className={cn(
            'absolute left-0 right-0 top-[calc(100%+0.375rem)] z-40 max-h-72 overflow-y-auto rounded-lg border border-border bg-bg-surface p-1 shadow-lg',
          )}
        >
          <div role="listbox">
            {visibleOptions.length > 0 ? (
              visibleOptions.map(renderOption)
            ) : (
              <div className="px-3 py-3 text-sm text-fg-tertiary">No charts found</div>
            )}
          </div>
        </div>
      ) : null}
    </div>
  );
}
