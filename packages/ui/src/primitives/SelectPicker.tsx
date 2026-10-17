import { selectionAction } from '../lib/controlEvents';
import * as React from 'react';
import { OverlayPortal } from './ResponsiveDialog';
import { Check, ChevronDown } from 'lucide-react';
import { cn } from '../lib/utils';
import { useDisclosure, useDismissLayer, useViewportPlacement } from '../hooks/useOverlayInteraction';
import { nextEnabledOption, selectPosition } from '../lib/overlayLayout';

export interface SelectPickerOption<TValue extends string = string> {
  value: TValue;
  label: React.ReactNode;
  description?: React.ReactNode;
  leading?: React.ReactNode;
  disabled?: boolean;
}

export interface SelectPickerProps<TValue extends string = string> {
  value: TValue;
  options: Array<SelectPickerOption<TValue>>;
  onChange: (value: TValue) => void;
  id?: string;
  'aria-label'?: string;
  placeholder?: React.ReactNode;
  disabled?: boolean;
  className?: string;
  triggerClassName?: string;
  menuClassName?: string;
  size?: 'sm' | 'md';
  align?: 'left' | 'right';
}

export function SelectPicker<TValue extends string = string>({
  value,
  options,
  onChange,
  id,
  'aria-label': ariaLabel,
  placeholder = 'Select an option',
  disabled = false,
  className,
  triggerClassName,
  menuClassName,
  size = 'md',
  align = 'left',
}: SelectPickerProps<TValue>) {
  const { open, setOpen, hide } = useDisclosure();
  const [activeIndex, setActiveIndex] = React.useState(-1);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const triggerRef = React.useRef<HTMLButtonElement>(null);
  const menuRef = React.useRef<HTMLDivElement>(null);
  const [position, setPosition] = React.useState<React.CSSProperties | null>(null);
  const optionRefs = React.useRef<Array<HTMLButtonElement | null>>([]);
  const listboxId = React.useId();
  const selectedIndex = options.findIndex((option) => option.value === value);
  const selectedOption = selectedIndex >= 0 ? options[selectedIndex] : undefined;

  const enabled = options.flatMap((option, index) => option.disabled ? [] : [index]);
  const naturalWidth = React.useRef(0);
  React.useLayoutEffect(() => {
    naturalWidth.current = (menuRef.current?.scrollWidth ?? 0) + 2;
    if (!open) setPosition(null);
  }, [open, align, size, options.length]);
  const place = React.useCallback(() => {
    const trigger = triggerRef.current, menu = menuRef.current;
    if (trigger && menu) setPosition(selectPosition(trigger.getBoundingClientRect(), naturalWidth.current, menu.scrollHeight, align, size === 'sm'));
  }, [align, size, options.length]);
  useViewportPlacement(open, place, true);
  useDismissLayer({ enabled: open, roots: [rootRef, menuRef], captureEscape: true, preventEscape: true, dismiss: reason => {
    setOpen(false);
    if (reason === 'escape') triggerRef.current?.focus({ preventScroll: true });
  } });

  const positioned = position !== null;
  React.useEffect(() => {
    if (!open || !positioned || activeIndex < 0) return;
    const option = optionRefs.current[activeIndex];
    const menu = menuRef.current;
    option?.focus({ preventScroll: true });
    if (option && menu) {
      const before = option.offsetTop < menu.scrollTop;
      const bottom = option.offsetTop + option.offsetHeight - menu.clientHeight;
      menu.scrollTop = before ? option.offsetTop : Math.max(menu.scrollTop, bottom);
    }
  }, [activeIndex, open, positioned]);

  function openPicker() {
    if (!disabled) {
      setActiveIndex(nextEnabledOption(enabled, options.length, selectedIndex >= 0 ? selectedIndex : enabled[0] ?? -1, 1));
      setOpen(true);
    }
  }
  const selectOption = selectionAction(onChange, hide, triggerRef, true);
  function handleTriggerKeyDown(event: React.KeyboardEvent<HTMLButtonElement>) {
    if (disabled || !['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) return;
    event.preventDefault();
    openPicker();
  }
  function handleOptionKeyDown(event: React.KeyboardEvent<HTMLButtonElement>, index: number) {
    const navigation: Record<string, readonly [number, 1 | -1]> = {
      ArrowDown: [index + 1, 1], ArrowUp: [index - 1, -1], Home: [0, 1], End: [options.length - 1, -1],
    };
    const move = navigation[event.key];
    if (move) {
      event.preventDefault();
      setActiveIndex(nextEnabledOption(enabled, options.length, ...move));
      return;
    }
    if (event.key === 'Tab') { triggerRef.current?.focus({ preventScroll: true }); setOpen(false); return; }
    if (event.key !== 'Enter' && event.key !== ' ') return;
    event.preventDefault();
    const option = options[index];
    if (option && !option.disabled) selectOption(option.value);
  }

  const triggerSize = size === 'sm' ? 'h-8 px-2.5 text-xs' : 'h-9 px-3 text-sm';

  return (
    <div ref={rootRef} className={cn('relative inline-block min-w-0', className)}>
      <button
        ref={triggerRef}
        id={id}
        type="button"
        disabled={disabled}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listboxId}
        onClick={() => (open ? setOpen(false) : openPicker())}
        onKeyDown={handleTriggerKeyDown}
        className={cn(
          'flex w-full min-w-0 items-center justify-between gap-3 rounded-lg border border-border bg-bg-elevated text-left text-fg-secondary transition-colors',
          'hover:border-border-strong hover:text-fg focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent',
          'disabled:cursor-not-allowed disabled:opacity-60',
          triggerSize,
          triggerClassName
        )}
      >
        <span className="flex min-w-0 flex-1 items-center gap-2">
          {selectedOption?.leading ? <span className="shrink-0">{selectedOption.leading}</span> : null}
          <span className="min-w-0 truncate">{selectedOption?.label ?? placeholder}</span>
        </span>
        <ChevronDown
          className={cn(
            'h-3.5 w-3.5 shrink-0 text-fg-tertiary transition-transform',
            open && 'rotate-180'
          )}
        />
      </button>

      {open ? (
        <OverlayPortal container={rootRef.current?.closest('[role="dialog"]') ?? null}>
        <div
          ref={menuRef}
          id={listboxId}
          role="listbox"
          aria-label={ariaLabel}
          className={cn(
            'fixed z-[100] w-max overflow-y-auto overscroll-contain rounded-lg border border-border bg-bg-elevated p-1 shadow-lg',
            menuClassName
          )}
          style={position ?? { visibility: 'hidden', maxWidth: 'calc(100vw - 24px)' }}
        >
          {options.length > 0 ? (
            options.map((option, index) => {
              const isSelected = option.value === value;
              const text = [option.label, option.description].filter(item => typeof item === 'string');
              const optionAriaLabel = typeof option.label === 'string' ? text.join(' ') : undefined;
              return (
                <button
                  key={option.value}
                  ref={(element) => {
                    optionRefs.current[index] = element;
                  }}
                  type="button"
                  role="option"
                  aria-label={optionAriaLabel}
                  aria-selected={isSelected}
                  disabled={option.disabled}
                  onClick={() => selectOption(option.value)}
                  onKeyDown={(event) => handleOptionKeyDown(event, index)}
                  className={cn(
                    'flex w-full items-center justify-between gap-3 rounded-md px-3 py-2 text-left text-sm text-fg transition-colors',
                    'hover:bg-bg-surface focus:bg-bg-surface focus:outline-none focus-visible:ring-1 focus-visible:ring-accent',
                    'disabled:cursor-not-allowed disabled:opacity-45',
                    isSelected && 'bg-accent/10 text-accent'
                  )}
                >
                  <span className="flex min-w-0 items-center gap-2">
                    {option.leading ? <span className="shrink-0">{option.leading}</span> : null}
                    <span className="min-w-0 whitespace-normal break-words">
                      <span className="block">{option.label}</span>
                      {option.description ? (
                        <span className="mt-0.5 block text-xs text-fg-tertiary">
                          {option.description}
                        </span>
                      ) : null}
                    </span>
                  </span>
                  {isSelected ? <Check className="h-4 w-4 shrink-0" /> : null}
                </button>
              );
            })
          ) : (
            <div className="px-3 py-2 text-sm text-fg-tertiary">No options available</div>
          )}
        </div>
        </OverlayPortal>
      ) : null}
    </div>
  );
}
