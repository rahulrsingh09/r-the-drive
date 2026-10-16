import React from 'react';
import { getWidgetForInstance } from './registry';
import type { WidgetInstance, WidgetCtx } from './registry';

export type WidgetPresentationProps = {
  instance: WidgetInstance;
  ctx: WidgetCtx;
  children: React.ReactNode;
};

const WidgetPresentationContext = React.createContext<
  Readonly<Record<string, React.ComponentType<WidgetPresentationProps>>>
>({});

export const WidgetPresentationProvider = WidgetPresentationContext.Provider;

interface WidgetHostProps {
  instance: WidgetInstance;
  ctx: WidgetCtx;
}

export function WidgetHost({ instance, ctx }: WidgetHostProps) {
  const presentations = React.useContext(WidgetPresentationContext);
  const def = getWidgetForInstance(instance);

  if (!def) {
    return (
      <div className="flex h-full items-center justify-center rounded-lg border border-dashed border-border text-xs text-fg-tertiary">
        Unknown widget: {instance.componentType}/{instance.definitionId}
      </div>
    );
  }

  const Component = def.component;
  const Presentation = presentations[`${instance.componentType}:${instance.definitionId}`];
  const content = <Component instance={instance} ctx={ctx} />;
  return (
    <div className="flex h-full min-h-0 flex-col" data-widget-type={instance.componentType} data-widget-definition={instance.definitionId}
      data-chart-id={instance.componentType === 'chart' && typeof instance.options?.chartId === 'string' ? instance.options.chartId : undefined}>
      {instance.title && shouldShowWidgetTitle(instance) ? (
        <p className="mb-2 shrink-0 text-xs font-medium uppercase tracking-wider text-fg-tertiary">
          {instance.title}
        </p>
      ) : null}
      <div className="min-h-0 flex-1 [&>*]:h-full">
        {Presentation ? <Presentation instance={instance} ctx={ctx}>{content}</Presentation> : content}
      </div>
    </div>
  );
}

function shouldShowWidgetTitle(instance: WidgetInstance) {
  if (instance.componentType === 'chart' && instance.options?.showPicker === false) return false;
  if (instance.componentType === 'sensor' || instance.componentType === 'battery' || instance.componentType === 'charging') return false;
  if (instance.componentType === 'custom' && instance.definitionId === 'charging.connection') return false;
  if (instance.componentType === 'custom' && instance.definitionId === 'trips.stat') return false;
  return true;
}
