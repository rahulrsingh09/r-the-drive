export type BatteryGen = 'gen1' | 'gen2';

export const RIVIAN_BATTERY_PRESETS: Record<BatteryGen, Array<{ key: string; label: string; kwh: number | null }>> = {
  gen1: [
    { key: 'r1_standard_g1', label: 'R1T / R1S Standard (Gen 1)', kwh: 105 },
    { key: 'r1_large_g1',    label: 'R1T / R1S Large (Gen 1)',    kwh: 135 },
    { key: 'r1_max_g1',      label: 'R1T / R1S Max (Gen 1)',      kwh: 180 },
    { key: 'custom',         label: 'Custom',                     kwh: null },
  ],
  gen2: [
    { key: 'r1_standard_g2', label: 'R1T / R1S Standard (Gen 2)', kwh: 92.5 },
    { key: 'r1_large_g2',    label: 'R1T / R1S Large (Gen 2)',    kwh: 109 },
    { key: 'r1_max_g2',      label: 'R1T / R1S Max (Gen 2)',      kwh: 140 },
    { key: 'custom',         label: 'Custom',                     kwh: null },
  ],
};

export const ALL_PRESETS = [...RIVIAN_BATTERY_PRESETS.gen1, ...RIVIAN_BATTERY_PRESETS.gen2];
export const R2_PRESET = { key: 'r2', label: 'R2', kwh: 82 };
