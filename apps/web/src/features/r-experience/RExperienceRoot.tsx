import React from 'react';
import { DataPaletteProvider, type DataPaletteOverride } from '@panda/ui/hooks';
import { WidgetPresentationProvider } from '@panda/dashboards';
import { RDashboardVehicle } from './RDashboardVehicle';
import { attachRDocument } from './browserBindings';
import './r-experience.css';

const widgetPresentations = { 'custom:overview.vehicle': RDashboardVehicle };

export function RExperienceRoot({ children }: { children: React.ReactNode }) {
  const [dataPalette, setDataPalette] = React.useState<DataPaletteOverride>({ palette: 'rad' });
  React.useLayoutEffect(() => {
    const documentBinding = attachRDocument();
    setDataPalette(documentBinding.palette);
    return documentBinding.dispose;
  }, []);
  return <DataPaletteProvider value={dataPalette}>
    <WidgetPresentationProvider value={widgetPresentations}>{children}</WidgetPresentationProvider>
  </DataPaletteProvider>;
}
