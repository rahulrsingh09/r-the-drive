import React from 'react';
import { WidgetHost } from './WidgetHost';
import type { WidgetCtx, WidgetEditorMeta } from './registry';
import type { WidgetInstance } from './schema';

export interface WidgetChromeProps extends React.HTMLAttributes<HTMLDivElement> {
  instance: WidgetInstance;
  ctx: WidgetCtx;
  mode: 'view' | 'edit';
  editor?: WidgetEditorMeta;
  isEditing?: boolean;
  onToggleEdit?: () => void;
  updateWidgetOptions?: (widgetId: string, patch: Record<string, unknown>) => void;
}

export const WidgetChrome = React.forwardRef<HTMLDivElement, WidgetChromeProps>(function WidgetChrome({
  instance,
  ctx,
  mode: _mode,
  editor: _editor,
  isEditing: _isEditing,
  onToggleEdit: _onToggleEdit,
  updateWidgetOptions: _updateWidgetOptions,
  className,
  style,
  children,
  ...frameProps
}, ref) {
  return (
    <div {...frameProps} ref={ref} data-widget-frame="view"
      className={['dashboard-widget-frame h-full min-h-0', className].filter(Boolean).join(' ')} style={style}>
      <WidgetHost instance={instance} ctx={ctx} />
      {children}
    </div>
  );
});
