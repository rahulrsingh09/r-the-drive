import * as React from 'react';
import { createPortal } from 'react-dom';
import { cn } from '../lib/utils';
import { useDisclosure, useDismissLayer, useViewportPlacement } from '../hooks/useOverlayInteraction';
import { floatingPosition } from '../lib/overlayLayout';

export interface TooltipProps {
  content: React.ReactNode;
  children: React.ReactNode;
  side?: 'top' | 'bottom';
  align?: 'start' | 'center' | 'end';
  className?: string;
  contentClassName?: string;
}

export function Tooltip({
  content,
  children,
  side = 'top',
  align = 'center',
  className,
  contentClassName,
}: TooltipProps) {
  const { open, setOpen } = useDisclosure();
  const [position, setPosition] = React.useState<{ top: number; left: number; visibility: 'hidden' | 'visible' }>({
    top: 0,
    left: 0,
    visibility: 'hidden',
  });
  const rootRef = React.useRef<HTMLSpanElement | null>(null);
  const triggerRef = React.useRef<HTMLSpanElement | null>(null);
  const contentRef = React.useRef<HTMLSpanElement | null>(null);
  const tooltipId = React.useId();

  const measure = React.useCallback(() => {
    const trigger = triggerRef.current, popup = contentRef.current;
    if (trigger && popup) setPosition(floatingPosition(trigger.getBoundingClientRect(), popup.getBoundingClientRect(), side, align));
  }, [side, align, content]);
  useViewportPlacement(open, measure);
  useDismissLayer({ enabled: open, roots: [rootRef], dismiss: () => setOpen(false) });

  const tooltip = open ? (
    <span
      id={tooltipId}
      ref={contentRef}
      role="tooltip"
      aria-hidden={!open}
      className={cn(
        'pointer-events-none fixed z-50 max-h-[calc(100vh-1rem)] w-56 max-w-[calc(100vw-1rem)] overflow-y-auto rounded-lg border border-border bg-bg-surface px-3 py-2 text-[11px] normal-case tracking-normal text-fg shadow-lg transition-opacity duration-150',
        open ? 'opacity-100' : 'opacity-0',
        contentClassName
      )}
      style={{ top: position.top, left: position.left, visibility: position.visibility }}
    >
      {content}
    </span>
  ) : null;

  let describedChild = children;
  if (React.isValidElement<{ 'aria-describedby'?: string }>(children)) {
    const current = children.props['aria-describedby'];
    const ids: string[] = [];
    if (current) ids.push(current);
    if (open) ids.push(tooltipId);
    describedChild = React.cloneElement(children, ids.length ? { 'aria-describedby': ids.join(' ') } : {});
  }

  return (
    <span
      ref={rootRef}
      className={cn('relative inline-flex', className)}
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
      onPointerDown={(event) => {
        if (event.pointerType === 'touch') setOpen(true);
      }}
      onFocus={() => setOpen(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          setOpen(false);
        }
      }}
    >
      <span ref={triggerRef} className="inline-flex">
        {describedChild}
      </span>
      {typeof document !== 'undefined' ? createPortal(tooltip, document.body) : tooltip}
    </span>
  );
}
