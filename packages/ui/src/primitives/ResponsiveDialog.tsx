import * as React from 'react';
import { createPortal } from 'react-dom';
import { cn } from '../lib/utils';
import { useDialogLifecycle } from '../hooks/useOverlayInteraction';

export function OverlayPortal({ children, container }: { children: React.ReactNode; container?: Element | null }) {
  if (typeof document === 'undefined') return null;
  return createPortal(children, container ?? document.body);
}

export interface ResponsiveDialogProps {
  titleId: string;
  onClose: () => void;
  children: React.ReactNode;
  className?: string;
}

/** Opaque, focus-managed dialog that becomes a safe-area fullscreen surface on mobile. */
export function ResponsiveDialog({ titleId, onClose, children, className }: ResponsiveDialogProps) {
  const dialogRef = React.useRef<HTMLDivElement>(null);
  useDialogLifecycle(dialogRef, onClose);

  return (
    <OverlayPortal>
    <div className="fixed inset-0 z-[70] flex items-stretch justify-center bg-bg-page/85 sm:items-center sm:p-4" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby={titleId} className={cn('flex min-h-[100dvh] w-full flex-col overflow-hidden bg-bg-surface shadow-lg sm:min-h-0 sm:max-h-[min(42rem,calc(100dvh-2rem))] sm:max-w-lg sm:rounded-xl sm:border sm:border-border', className)}>{children}</div>
    </div>
    </OverlayPortal>
  );
}
