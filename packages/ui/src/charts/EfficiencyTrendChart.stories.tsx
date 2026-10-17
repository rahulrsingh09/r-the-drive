import { storyTimeline } from './chartStoryTimeline';
import type { Meta, StoryObj } from '@storybook/react';
import { EfficiencyTrendChart } from './EfficiencyTrendChart';

const meta = {
  title: 'Charts/EfficiencyTrendChart',
  component: EfficiencyTrendChart,
  parameters: { layout: 'padded' },
  args: { height: 240 },
} satisfies Meta<typeof EfficiencyTrendChart>;

export default meta;
type Story = StoryObj<typeof meta>;

function makeData(trips = 60) {
  const timeline = storyTimeline('2024-01-01', trips, 'hour', 5);
  const observations = timeline.map((_, index) => ({ trip: Math.random(), rolling: index < 4 ? null : Math.random() }));
  return timeline.map((ts, index) => {
    const base = 320 + Math.sin(index / 10) * 40;
    const reading = observations[index]!;
    return { ts, trip_efficiency_wh_mi: Number((base + (reading.trip - 0.5) * 60).toFixed(1)),
      rolling_24h_wh_mi: reading.rolling == null ? null : Number((base + (reading.rolling - 0.5) * 20).toFixed(1)) };
  });
}


export const Default: Story = {
  args: { data: makeData(60) },
};

export const WithBrush: Story = {
  args: { data: makeData(90), showBrush: true },
};

export const Short: Story = {
  args: { data: makeData(7) },
};

export const Loading: Story = {
  args: { data: [], loading: true },
};
