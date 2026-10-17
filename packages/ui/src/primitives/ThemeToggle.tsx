import { selectionAction } from '../lib/controlEvents';
import * as React from 'react';
import { createPortal } from 'react-dom';
import { Check, Laptop, Moon, Sun, type LucideIcon } from 'lucide-react';
import { cn } from '../lib/utils';
import { useDisclosure, useDismissLayer, useMediaQuery, useViewportPlacement } from '../hooks/useOverlayInteraction';
import { floatingPosition, rovingIndex } from '../lib/overlayLayout';
import {
  applyThemeMode,
  resolveThemeMode,
  type ThemeMode,
} from '../lib/theme';

export interface ThemeToggleProps {
  className?: string;
  label?: string;
  showLabel?: boolean;
  ariaLabel?: string;
  align?: 'start' | 'end';
  variant?: 'solid' | 'ghost';
  disabled?: boolean;
  size?: 'default' | 'menu';
  /** Overrides the trigger icon size, e.g. to optically match neighbouring icons. */
  iconClassName?: string;
  mode?: ThemeMode;
  onModeChange?: (mode: ThemeMode) => void;
}

type Position = {
  top: number;
  left: number;
  visibility: 'hidden' | 'visible';
};

const THEME_OPTIONS: Array<{
  mode: ThemeMode;
  label: string;
  description: string;
  icon: LucideIcon;
}> = [
  {
    mode: 'light',
    label: 'Light',
    description: 'Always use the light palette',
    icon: Sun,
  },
  {
    mode: 'dark',
    label: 'Dark',
    description: 'Always use the dark palette',
    icon: Moon,
  },
  {
    mode: 'system',
    label: 'System',
    description: 'Follow your device appearance',
    icon: Laptop,
  },
];

export function ThemeToggle({
  className,
  label = 'Theme',
  showLabel = false,
  ariaLabel,
  align = 'end',
  variant = 'solid',
  disabled = false,
  size = 'default',
  iconClassName,
  mode: controlledMode,
  onModeChange,
}: ThemeToggleProps) {
  const [uncontrolledMode, setUncontrolledMode] = React.useState<ThemeMode>('dark');
  const mode = controlledMode ?? uncontrolledMode;
  const { open, setOpen, hide, toggle } = useDisclosure();
  const isMobile = useMediaQuery('(max-width: 639px)');
  const [position, setPosition] = React.useState<Position>({
    top: 0,
    left: 0,
    visibility: 'hidden',
  });
  const triggerRef = React.useRef<HTMLButtonElement | null>(null);
  const menuRef = React.useRef<HTMLDivElement | null>(null);
  const ModeIcon = mode === 'system' ? Laptop : resolveThemeMode(mode) === 'dark' ? Moon : Sun;
  const modeLabel = THEME_OPTIONS.find(option => option.mode === mode)!.label;
  const triggerAriaLabel = ariaLabel ?? (showLabel ? undefined : 'Theme options');

  useDismissLayer({ enabled: open, roots: [triggerRef, menuRef], captureEscape: true, preventEscape: true, dismiss: reason => {
    setOpen(false);
    if (reason === 'escape') triggerRef.current?.focus();
  } });
  React.useEffect(() => {
    if (!open || (!isMobile && position.visibility !== 'visible')) return;
    menuRef.current?.querySelector<HTMLButtonElement>('[aria-checked="true"]')?.focus();
  }, [open, isMobile, position.visibility]);
  const measure = React.useCallback(() => {
    const trigger = triggerRef.current, menu = menuRef.current;
    if (trigger && menu) setPosition(floatingPosition(trigger.getBoundingClientRect(), menu.getBoundingClientRect(), 'bottom', align, false));
  }, [align]);
  useViewportPlacement(open && !isMobile, measure);
  function navigateMenu(event: React.KeyboardEvent) {
    const buttons = [...menuRef.current!.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (!['Home', 'End', 'ArrowDown', 'ArrowUp'].includes(event.key)) return;
    const next = rovingIndex(event.key, index, buttons.length);
    if (next !== null && next >= 0) { event.preventDefault(); buttons[next]?.focus(); }
  }

  const changeMode = onModeChange ?? ((next: ThemeMode) => {
    applyThemeMode(next);
    setUncontrolledMode(next);
  });
  const setThemeMode = selectionAction(changeMode, hide, triggerRef);

  const trigger = (
    <button
      ref={triggerRef}
      type="button"
      aria-disabled={disabled || undefined}
      onClick={disabled ? undefined : toggle}
      aria-label={triggerAriaLabel}
      aria-haspopup="menu"
      aria-expanded={open}
      className={cn(
        variant === 'ghost'
          ? 'group inline-flex items-center gap-2 rounded-lg text-fg-tertiary transition-colors duration-150 hover:text-fg hover:bg-bg-elevated'
          : 'group inline-flex items-center gap-2 rounded-lg border border-border bg-bg-elevated text-fg-secondary transition-colors duration-150 hover:border-border-strong hover:text-fg hover:bg-bg-elevated/80',
        showLabel ? 'h-9 px-3 text-sm font-medium' : 'h-8 w-8 justify-center',
        disabled && 'cursor-wait opacity-60',
        size === 'menu' && 'gap-3',
        className
      )}
    >
      <span className={cn('inline-flex shrink-0 items-center justify-center', size === 'menu' ? 'h-5 w-5' : 'h-4 w-4', iconClassName)}>
        <ModeIcon className={cn(size === 'menu' ? 'h-5 w-5' : 'h-4 w-4', iconClassName)} />
      </span>
      {showLabel && (
        <span className="min-w-0 flex-1 truncate text-left text-sm font-medium text-fg">
          {label}
        </span>
      )}
      {showLabel && (
        <span className={cn('shrink-0 font-medium text-fg-tertiary', size === 'menu' ? 'text-sm' : 'text-xs')}>
          {modeLabel}
        </span>
      )}
    </button>
  );

  const panel = open ? (
    <div
      ref={menuRef}
      role="menu"
      aria-label="Theme options"
      onKeyDown={navigateMenu}
      className={cn(
        'fixed z-[70] overflow-hidden rounded-2xl border border-border bg-bg-surface shadow-xl',
        isMobile
          ? 'inset-x-2 bottom-2 max-h-[calc(100vh-1rem)] w-auto'
          : 'w-72 max-w-[calc(100vw-1rem)]'
      )}
      style={isMobile ? { paddingBottom: 'calc(env(safe-area-inset-bottom, 0px) + 0.5rem)' } : { top: position.top, left: position.left, visibility: position.visibility }}
    >
      <div className="border-b border-border px-4 py-3">
        <p className="text-sm font-medium text-fg">Theme</p>
        <p className="mt-0.5 text-xs text-fg-tertiary">Choose light, dark, or follow your device</p>
      </div>

      <div className="max-h-[min(22rem,calc(100vh-6rem))] overflow-y-auto p-2">
        {THEME_OPTIONS.map((option) => {
          const appearance = option.mode === mode
            ? { checked: true, item: 'bg-accent-muted text-accent', icon: 'border-accent/30 bg-accent/10', mark: <Check className="h-4 w-4 shrink-0 text-accent" /> }
            : { checked: false, item: 'text-fg-secondary hover:bg-bg-elevated hover:text-fg', icon: 'border-border bg-bg-elevated', mark: null };
          const Icon = option.icon;

          return (
            <button
              key={option.mode}
              type="button"
              role="menuitemradio"
              aria-checked={appearance.checked}
              onClick={() => setThemeMode(option.mode)}
              className={cn(
                'flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left transition-colors',
                appearance.item
              )}
            >
              <span
                className={cn(
                  'inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-full border',
                  appearance.icon
                )}
              >
                <Icon className="h-4 w-4" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-medium">{option.label}</span>
                <span className="block text-xs text-fg-tertiary">{option.description}</span>
              </span>
              {appearance.mark}
            </button>
          );
        })}
      </div>
    </div>
  ) : null;

  return (
    <>
      {trigger}
      {typeof document !== 'undefined' ? createPortal(panel, document.body) : panel}
    </>
  );
}
