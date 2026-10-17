import { storyTimeline } from './chartStoryTimeline';
import type { Meta, StoryObj } from '@storybook/react';
import { DailyEnergyBarChart } from './DailyChargeSessionsChart';

const meta = {
  title: 'Charts/DailyEnergyBarChart',
  component: DailyEnergyBarChart,
  parameters: { layout: 'padded' },
  args: { height: 280 },
} satisfies Meta<typeof DailyEnergyBarChart>;

export default meta;
type Story = StoryObj<typeof meta>;

function makeDailyData(days: number) {
  const dates = storyTimeline('2024-01-01T00:00:00Z', days, 'utc-day', 1);
  return dates.map((day_start, index) => ({
    day_local: day_start.slice(0, 10), day_start: `${day_start.slice(0, 10)}T00:00:00Z`,
    total_energy_kwh: Number((8 + ((index * 17) % 61) + Math.sin(index / 4) * 5).toFixed(1)),
    session_count: 1 + (index % 3),
  }));
}


export const SevenDays: Story = {
  args: { daily: makeDailyData(7) },
};

export const OneHundredTwentyDays: Story = {
  args: { daily: makeDailyData(120) },
};
