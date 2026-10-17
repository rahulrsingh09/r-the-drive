import React, { useCallback, useEffect, useState } from 'react';
import GridLayout, { useContainerWidth } from 'react-grid-layout';
import type { LayoutItem } from 'react-grid-layout';
import 'react-grid-layout/css/styles.css';
import { getAllWidgets, getWidgetEditorMeta, getWidgetForInstance } from './registry';
import { sanitizeWidgetInstance } from './layout';
import {
  applyWidgetLayout,
  createWidgetInstance,
  DASHBOARD_GRID_COLUMNS,
  DASHBOARD_ROW_HEIGHT,
  dashboardKey,
} from './dashboardModel';
import { WidgetChrome } from './WidgetChrome';
import type { DashboardConfig, WidgetInstance } from './schema';
import type { WidgetCtx, WidgetDef } from './registry';
import type { DashboardVisibilityState } from './dashboardVisibility';
import {
  DEFAULT_DASHBOARD_VISIBILITY_STATE,
  getWidgetVisibilityRules,
} from './dashboardVisibility';
import { EditorDrawer } from './editor/EditorDrawer';
import { PaletteView } from './editor/PaletteView';
import { WidgetEditForm } from './editor/WidgetEditForm';

interface GridEditorProps {
  config: DashboardConfig;
  ctx: WidgetCtx;
  onConfigChange: ((next: DashboardConfig) => void) | undefined;
  editActions?: React.ReactNode;
  previewControls?: React.ReactNode;
  visibleWidgetIds?: string[];
  visibilityState?: DashboardVisibilityState;
  onVisibilityStateChange?: (
    type: 'vehicle-connection',
    value: DashboardVisibilityState['vehicle-connection'],
  ) => void;
}

function layoutFromConfig(widgets: WidgetInstance[]): LayoutItem[] {
  return widgets.map(widget => {
    const definition = getWidgetForInstance(widget);
    const editor = getWidgetEditorMeta(definition);
    const limits = editor.fixedSize
      ? { minimum: widget.layout, maximum: widget.layout }
      : { minimum: definition?.minSize ?? { w: 1, h: 1 }, maximum: editor.maxSize };
    const item: LayoutItem = {
      i: widget.id,
      ...widget.layout,
      minW: limits.minimum.w,
      minH: limits.minimum.h,
      isDraggable: editor.movable,
      isResizable: editor.resizable,
      resizeHandles: editor.resizable ? ['se'] : [],
    };
    if (limits.maximum) Object.assign(item, { maxW: limits.maximum.w, maxH: limits.maximum.h });
    return item;
  });
}

type EditorChange =
  | { kind: 'layout'; layout: readonly LayoutItem[] }
  | { kind: 'remove'; id: string }
  | { kind: 'add'; definition: WidgetDef }
  | { kind: 'replace'; widget: WidgetInstance };

function editWidgets(widgets: WidgetInstance[], change: EditorChange): WidgetInstance[] {
  switch (change.kind) {
    case 'layout': return applyWidgetLayout(widgets, change.layout);
    case 'remove': return widgets.filter(widget => widget.id !== change.id);
    case 'replace': return widgets.map(widget => widget.id === change.widget.id ? change.widget : widget);
    case 'add': {
      let bottom = 0;
      for (const { layout } of widgets) bottom = Math.max(bottom, layout.y + layout.h);
      return widgets.concat(createWidgetInstance(change.definition, { x: 0, y: bottom }));
    }
  }
}

export default function GridEditor({
  config,
  ctx,
  onConfigChange,
  editActions,
  previewControls,
  visibleWidgetIds: requestedVisibleWidgetIds,
  visibilityState = DEFAULT_DASHBOARD_VISIBILITY_STATE,
  onVisibilityStateChange = () => {},
}: GridEditorProps) {
  const widgets = Array.isArray(config.widgets) ? config.widgets.map(sanitizeWidgetInstance) : [];
  const visibleWidgetIds = requestedVisibleWidgetIds ?? widgets.map(widget => widget.id);
  const visibleWidgetIdSet = new Set(visibleWidgetIds);
  const visibleWidgets = widgets.filter(widget => visibleWidgetIdSet.has(widget.id));
  const [selection, setSelection] = useState<{ dashboard: string; id: string | null }>(() => ({
    dashboard: dashboardKey(config, config.slug), id: null,
  }));
  const editingId = selection.id;
  const setEditingId = (id: string | null) => setSelection({ dashboard: dashboardKey(config, config.slug), id });
  const { width, containerRef, mounted } = useContainerWidth();
  const currentDashboardKey = dashboardKey(config, config.slug);

  const commit = useCallback((change: EditorChange) => {
    const next = editWidgets(widgets, change);
    if (next !== widgets) onConfigChange?.({ ...config, widgets: next.map(sanitizeWidgetInstance) });
  }, [config, widgets, onConfigChange]);

  const handleLayoutChange = (layout: readonly LayoutItem[]) => commit({ kind: 'layout', layout });
  function removeWidget(id: string) {
    commit({ kind: 'remove', id });
    if (editingId === id) setEditingId(null);
  }
  const addWidget = (definition: WidgetDef) => commit({ kind: 'add', definition });
  function updateWidget(next: WidgetInstance) {
    const widget = sanitizeWidgetInstance(next);
    const mismatches = getWidgetVisibilityRules(widget).filter(rule => visibilityState[rule.type] !== rule.value);
    for (const { type, value } of mismatches) onVisibilityStateChange(type, value);
    commit({ kind: 'replace', widget });
  }
  function patchWidgetOptions(widgetId: string, patch: Record<string, unknown>) {
    const index = widgets.findIndex(widget => widget.id === widgetId);
    if (index < 0) return;
    const widget = widgets[index]!;
    updateWidget(Object.assign({}, widget, { options: Object.assign({}, widget.options, patch) }));
  }

  const editingWidget = editingId ? widgets.find(widget => widget.id === editingId) ?? null : null;
  const drawerMode = editingWidget ? 'edit' : 'palette';
  useEffect(() => {
    const changedDashboard = selection.dashboard !== currentDashboardKey;
    const missingSelection = editingId && (!editingWidget || !visibleWidgetIdSet.has(editingId));
    if (changedDashboard || missingSelection) setSelection({ dashboard: currentDashboardKey, id: null });
  }, [currentDashboardKey, selection.dashboard, editingId, editingWidget, visibleWidgetIds.join('|')]);

  return (
    <>
      <style>{`
        .rgl-editor .react-grid-placeholder {
          background: color-mix(in oklab, var(--rm-accent) 48%, transparent) !important;
          border: 1px solid color-mix(in oklab, var(--rm-accent) 75%, transparent);
          border-radius: 8px;
          opacity: 0.18;
        }
        .rgl-editor .react-resizable-handle {
          z-index: 60;
          opacity: 0.62;
          pointer-events: auto;
          background-image: none;
          transition: opacity 120ms ease, border-color 120ms ease;
        }
        .rgl-editor .react-grid-item:hover .react-resizable-handle,
        .rgl-editor .react-grid-item:focus-within .react-resizable-handle,
        .rgl-editor .react-grid-item.resizing .react-resizable-handle,
        .rgl-editor .react-grid-item.react-draggable-dragging .react-resizable-handle {
          opacity: 1;
        }
        .rgl-editor .rgl-widget-control {
          /* Keep editor controls functional even if a host app misses package Tailwind utilities. */
          position: absolute;
          top: 0.5rem;
          z-index: 200;
          display: flex;
          align-items: center;
          gap: 0.25rem;
          opacity: 0.72;
          pointer-events: auto;
          transition: opacity 120ms ease;
        }
        .rgl-editor [data-widget-move-control="true"] {
          left: 0.5rem;
        }
        .rgl-editor [data-widget-edit-control="true"] {
          right: 0.5rem;
        }
        .rgl-editor .react-grid-item:hover .rgl-widget-control,
        .rgl-editor .react-grid-item:focus-within .rgl-widget-control,
        .rgl-editor .react-grid-item.resizing .rgl-widget-control,
        .rgl-editor .react-grid-item.react-draggable-dragging .rgl-widget-control,
        .rgl-editor .rgl-card[data-editing="true"] .rgl-widget-control {
          opacity: 1;
        }
        @media (hover: none), (pointer: coarse) {
          .rgl-editor .rgl-widget-control,
          .rgl-editor .react-resizable-handle {
            opacity: 1;
          }
          .rgl-editor .rgl-action {
            height: 2.25rem;
            width: 2.25rem;
          }
        }
        .rgl-editor .react-grid-item.react-resizable-hide .react-resizable-handle,
        .rgl-editor .rgl-card[data-widget-resizable="false"] .react-resizable-handle {
          display: none;
          pointer-events: none;
        }
        .rgl-editor .react-resizable-handle-se {
          right: 0;
          bottom: 0;
          width: 28px;
          height: 28px;
          border: 1px solid color-mix(in oklab, var(--rm-accent) 70%, transparent);
          border-radius: 10px 0 8px 0;
          background: var(--rm-bg-elevated);
          cursor: se-resize;
        }
        .rgl-editor .react-resizable-handle-se::after {
          content: '';
          position: absolute;
          right: 6px;
          bottom: 6px;
          width: 8px;
          height: 8px;
          border-right: 2px solid var(--rm-accent);
          border-bottom: 2px solid var(--rm-accent);
        }
        .rgl-editor .rgl-action {
          height: 2rem;
          width: 2rem;
          display: inline-flex;
          align-items: center;
          justify-content: center;
          border: 1px solid var(--rm-border-default);
          background: color-mix(in oklab, var(--rm-bg-elevated) 92%, transparent);
          color: var(--rm-text-secondary);
          backdrop-filter: blur(10px);
        }
        .rgl-editor .rgl-action:hover {
          color: var(--rm-text-primary);
          border-color: var(--rm-border-strong);
        }
      `}</style>

      <div className="rgl-editor min-w-0">
        <div ref={containerRef as React.Ref<HTMLDivElement>} className="w-full min-w-0">
          {mounted ? (
            <GridLayout
              layout={layoutFromConfig(visibleWidgets)}
              width={width}
              gridConfig={{
                cols: DASHBOARD_GRID_COLUMNS,
                rowHeight: DASHBOARD_ROW_HEIGHT,
                margin: [16, 16] as readonly [number, number],
                containerPadding: [0, 0] as readonly [number, number],
                maxRows: Infinity,
              }}
              dragConfig={{ enabled: true, bounded: false, handle: '.drag-handle', threshold: 3 }}
              resizeConfig={{ enabled: true, handles: ['se'] }}
              onLayoutChange={handleLayoutChange}
              className="rounded-xl border border-dashed border-border bg-bg-elevated/30"
            >
              {visibleWidgets.map((widget) => {
                const isEditing = editingId === widget.id;
                const def = getWidgetForInstance(widget);
                const editor = getWidgetEditorMeta(def);
                return (
                  <WidgetChrome
                    key={widget.id}
                    instance={widget}
                    ctx={ctx}
                    mode="edit"
                    editor={editor}
                    isEditing={isEditing}
                    {...(widget.managed
                      ? {}
                      : { onToggleEdit: () => setEditingId(isEditing ? null : widget.id) })}
                    updateWidgetOptions={patchWidgetOptions}
                  />
                );
              })}
            </GridLayout>
          ) : null}
        </div>
      </div>

      <EditorDrawer
        mode={drawerMode}
        onBackToPalette={() => setEditingId(null)}
        editActions={editActions}
        previewControls={previewControls}
        paletteContent={<PaletteView widgets={getAllWidgets()} onAdd={addWidget} />}
        editContent={
          editingWidget ? (
            <WidgetEditForm
              widget={editingWidget}
              onChange={updateWidget}
              onClose={() => setEditingId(null)}
              onRemove={() => removeWidget(editingWidget.id)}
            />
          ) : null
        }
      />
    </>
  );
}
