import React from 'react';
import { DataPaletteProvider, type DataPaletteOverride } from '@panda/ui/hooks';
import { WidgetPresentationProvider } from '@panda/dashboards';
import { RDashboardVehicle } from './RDashboardVehicle';
import { readRDataPalette } from './dataPalette';
import './r-experience.css';

const widgetPresentations = { 'custom:overview.vehicle': RDashboardVehicle };

export function RExperienceRoot({ children }: { children: React.ReactNode }) {
  const [dataPalette, setDataPalette] = React.useState<DataPaletteOverride>({ palette: 'rad' });
  React.useLayoutEffect(() => {
    const body = document.body;
    body.classList.add('r-experience');
    document.documentElement.classList.add('r-interface');
    setDataPalette(readRDataPalette());
    const pointer = () => { body.dataset.inputMode = 'pointer'; };
    const keyboard = (event: KeyboardEvent) => {
      const target = event.target;
      if (!event.metaKey && !event.ctrlKey && !event.altKey
        && (event.key === 'Tab' || !(target instanceof Element && target.closest('input,textarea,[contenteditable]')))) {
        body.dataset.inputMode = 'keyboard';
      }
    };
    document.addEventListener('pointerdown', pointer, true);
    document.addEventListener('keydown', keyboard, true);
    return () => {
      body.classList.remove('r-experience');
      document.documentElement.classList.remove('r-interface');
      delete body.dataset.inputMode;
      document.removeEventListener('pointerdown', pointer, true);
      document.removeEventListener('keydown', keyboard, true);
    };
  }, []);
  return <DataPaletteProvider value={dataPalette}>
    <WidgetPresentationProvider value={widgetPresentations}>{children}</WidgetPresentationProvider>
  </DataPaletteProvider>;
}
