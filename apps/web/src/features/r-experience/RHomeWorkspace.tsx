import React from 'react';
import { DashboardRenderer, type DashboardConfig } from '@panda/dashboards';
import type { DashboardPageShellRenderState } from '../../components/dashboard/dashboardShellTypes';
import { getTimeframeLabel } from '../../lib/dates';
import { usesBundledLayout } from './defaultLayout';
import { RDashboardTools } from './RDashboardTools';
import { HomeVehicleHeadingContext } from './RDashboardVehicle';
import './r-full-dashboard.css';

export function RHomeWorkspace({ state, actions, activity }: {
  state: DashboardPageShellRenderState; actions: React.ReactNode; activity: React.ReactNode;
}) {
  const config = state.activeConfig;
  if (!config) return null;
  const vehicleWidget = config.widgets.find(widget => widget.componentType === 'custom' && widget.definitionId === 'overview.vehicle');
  const remaining = config.widgets.filter(widget => widget.id !== vehicleWidget?.id);
  const totals = remaining.filter(widget => widget.componentType === 'sensor');
  const charts = remaining.filter(widget => widget.componentType !== 'sensor');
  const sectionConfig = (widgets: DashboardConfig['widgets']): DashboardConfig => {
    const firstRow = widgets.length ? Math.min(...widgets.map(widget => widget.layout.y)) : 0;
    return { ...config, widgets: widgets.map(widget => ({
      ...widget, layout: { ...widget.layout, y: widget.layout.y - firstRow },
    })) };
  };
  return <div className="r-home-workspace r-dashboard-surface"
    data-r-default-layout={usesBundledLayout(state.activeConfig) || undefined}>
    <header className="r-workspace-heading">
      <div className="r-workspace-period">{!vehicleWidget && <h1>Home</h1>}<span>Driving summary · {getTimeframeLabel(state.timeframe)}</span></div>
      <div className="r-workspace-actions">{actions}<RDashboardTools state={state} /></div>
    </header>
    <section className="r-home-content" aria-label="Dashboard analysis" data-has-vehicle={Boolean(vehicleWidget)}>
      {vehicleWidget && <div className="r-home-vehicle">
        <HomeVehicleHeadingContext.Provider value={vehicleWidget.id}>
          <DashboardRenderer config={{ ...config, widgets: [vehicleWidget] }} ctx={state.ctx} />
        </HomeVehicleHeadingContext.Provider>
      </div>}
      {totals.length > 0 && <section className="r-home-totals" aria-label="Period totals">
        <DashboardRenderer config={sectionConfig(totals)} ctx={state.ctx} />
      </section>}
      {activity}
      {charts.length > 0 && <section className="r-home-charts" aria-label="Overview charts">
        <DashboardRenderer config={sectionConfig(charts)} ctx={state.ctx} />
      </section>}
    </section>
  </div>;
}
