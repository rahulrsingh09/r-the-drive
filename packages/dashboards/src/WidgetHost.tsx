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

const SELF_TITLED_TYPES = new Set<WidgetInstance['componentType']>(['sensor', 'battery', 'charging']);
const SELF_TITLED_DEFINITIONS = new Set(['custom:charging.connection', 'custom:trips.stat']);

function widgetTitle(instance: WidgetInstance, key: string) {
  const ownsTitle = SELF_TITLED_TYPES.has(instance.componentType) || SELF_TITLED_DEFINITIONS.has(key);
  const pickerHidden = instance.componentType === 'chart' && instance.options?.showPicker === false;
  return ownsTitle || pickerHidden ? undefined : instance.title;
}

export function WidgetHost({ instance, ctx }: WidgetHostProps) {
  const presentations = React.useContext(WidgetPresentationContext);
  const definition = getWidgetForInstance(instance);
  const key = `${instance.componentType}:${instance.definitionId}`;
  const layers: Array<(children: React.ReactNode) => React.ReactNode> = [];
  if (definition) {
    layers.push(() => React.createElement(definition.component, { instance, ctx }));
    const presentation = presentations[key];
    if (presentation) layers.push(children => React.createElement(presentation, { instance, ctx, children }));
  }
  const content = layers.reduce<React.ReactNode>(
    (children, render) => render(children),
    undefined,
  );
  const children: React.ReactNode[] = [];
  const attributes: Record<string, string | undefined> = {};
  let className = 'flex h-full items-center justify-center rounded-lg border border-dashed border-border text-xs text-fg-tertiary';
  if (layers.length) {
    className = 'flex h-full min-h-0 flex-col';
    attributes['data-widget-type'] = instance.componentType;
    attributes['data-widget-definition'] = instance.definitionId;
    const chartId = instance.options?.chartId;
    if (instance.componentType === 'chart' && typeof chartId === 'string') attributes['data-chart-id'] = chartId;
    const title = widgetTitle(instance, key);
    if (title) children.push(
      <p key="title" className="mb-2 shrink-0 text-xs font-medium uppercase tracking-wider text-fg-tertiary">
        {title}
      </p>,
    );
    children.push(<div key="content" className="min-h-0 flex-1 [&>*]:h-full">{content}</div>);
  } else {
    children.push('Unknown widget: ', instance.componentType, '/', instance.definitionId);
  }
  return <div className={className} {...attributes}>{children}</div>;
}
