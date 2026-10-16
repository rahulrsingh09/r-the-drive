import * as React from 'react';
import { OverlayPortal } from './ResponsiveDialog';
import { Check, ChevronDown } from 'lucide-react';
import { cn } from '../lib/utils';

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
  const [open, setOpen] = React.useState(false);
  const [activeIndex, setActiveIndex] = React.useState(-1);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const triggerRef = React.useRef<HTMLButtonElement>(null);
  const menuRef = React.useRef<HTMLDivElement>(null);
  const [position, setPosition] = React.useState<React.CSSProperties | null>(null);
  const optionRefs = React.useRef<Array<HTMLButtonElement | null>>([]);
  const listboxId = React.useId();
  const selectedIndex = options.findIndex((option) => option.value === value);
  const selectedOption = selectedIndex >= 0 ? options[selectedIndex] : undefined;

  const firstEnabledIndex = React.useMemo(
    () => options.findIndex((option) => !option.disabled),
    [options]
  );

  React.useLayoutEffect(() => {
    if (!open) { setPosition(null); return; }
    const naturalWidth = (menuRef.current?.scrollWidth ?? 0) + 2;
    const place = () => {
      const trigger = triggerRef.current;
      const menu = menuRef.current;
      if (!trigger || !menu) return;
      const rect = trigger.getBoundingClientRect();
      const viewport = window.visualViewport;
      const leftEdge = (viewport?.offsetLeft ?? 0) + 12;
      const topEdge = (viewport?.offsetTop ?? 0) + 12;
      const rightEdge = leftEdge + (viewport?.width ?? window.innerWidth) - 24;
      const bottomEdge = topEdge + (viewport?.height ?? window.innerHeight) - 24;
      const width = Math.min(Math.max(rect.width, naturalWidth), rightEdge - leftEdge);
      const gap = size === 'sm' ? 4 : 6;
      const below = bottomEdge - rect.bottom - gap;
      const above = rect.top - topEdge - gap;
      const down = below >= Math.min(288, menu.scrollHeight) || below >= above;
      const maxHeight = Math.max(32, Math.min(288, down ? below : above));
      const height = Math.min(menu.scrollHeight, maxHeight);
      setPosition({
        left: Math.max(leftEdge, Math.min(align === 'right' ? rect.right - width : rect.left, rightEdge - width)),
        top: Math.max(topEdge, down ? rect.bottom + gap : rect.top - gap - height),
        width, maxHeight,
      });
    };
    place();
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    window.visualViewport?.addEventListener('resize', place);
    window.visualViewport?.addEventListener('scroll', place);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
      window.visualViewport?.removeEventListener('resize', place);
      window.visualViewport?.removeEventListener('scroll', place);
    };
  }, [open, align, size, options.length]);

  React.useEffect(() => {
    if (!open) return;

    function handlePointerDown(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node) && !menuRef.current?.contains(event.target as Node)) setOpen(false);
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
        triggerRef.current?.focus({ preventScroll: true });
      }
    }

    document.addEventListener('pointerdown', handlePointerDown);
    document.addEventListener('keydown', handleEscape, true);
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown);
      document.removeEventListener('keydown', handleEscape, true);
    };
  }, [open]);

  const positioned = position !== null;
  React.useEffect(() => {
    if (!open || !positioned || activeIndex < 0) return;
    const option = optionRefs.current[activeIndex];
    const menu = menuRef.current;
    option?.focus({ preventScroll: true });
    if (option && menu) {
      if (option.offsetTop < menu.scrollTop) menu.scrollTop = option.offsetTop;
      else if (option.offsetTop + option.offsetHeight > menu.scrollTop + menu.clientHeight) {
        menu.scrollTop = option.offsetTop + option.offsetHeight - menu.clientHeight;
      }
    }
  }, [activeIndex, open, positioned]);

  function getEnabledIndex(start: number, direction: 1 | -1) {
    if (options.length === 0) return -1;
    let index = start;
    for (let count = 0; count < options.length; count += 1) {
      if (index < 0) index = options.length - 1;
      if (index >= options.length) index = 0;
      if (!options[index]?.disabled) return index;
      index += direction;
    }
    return -1;
  }

  function openPicker() {
    if (disabled) return;
    setActiveIndex(getEnabledIndex(selectedIndex >= 0 ? selectedIndex : firstEnabledIndex, 1));
    setOpen(true);
  }

  function selectOption(nextValue: TValue) {
    onChange(nextValue);
    setOpen(false);
    triggerRef.current?.focus({ preventScroll: true });
  }

  function handleTriggerKeyDown(event: React.KeyboardEvent<HTMLButtonElement>) {
    if (disabled) return;
    if (
      event.key === 'ArrowDown' ||
      event.key === 'ArrowUp' ||
      event.key === 'Enter' ||
      event.key === ' '
    ) {
      event.preventDefault();
      openPicker();
    }
  }

  function handleOptionKeyDown(event: React.KeyboardEvent<HTMLButtonElement>, index: number) {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const nextIndex = getEnabledIndex(
        index + (event.key === 'ArrowDown' ? 1 : -1),
        event.key === 'ArrowDown' ? 1 : -1
      );
      setActiveIndex(nextIndex);
    } else if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      const nextIndex =
        event.key === 'Home' ? getEnabledIndex(0, 1) : getEnabledIndex(options.length - 1, -1);
      setActiveIndex(nextIndex);
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      const option = options[index];
      if (option && !option.disabled) selectOption(option.value);
    } else if (event.key === 'Tab') {
      triggerRef.current?.focus({ preventScroll: true });
      setOpen(false);
    }
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
              const optionAriaLabel =
                typeof option.label === 'string'
                  ? typeof option.description === 'string'
                    ? `${option.label} ${option.description}`
                    : option.label
                  : undefined;
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
