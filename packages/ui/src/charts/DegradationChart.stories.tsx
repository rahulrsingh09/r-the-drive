import { storyTimeline } from './chartStoryTimeline';
import type { Meta, StoryObj } from '@storybook/react';
import { DegradationChart } from './DegradationChart';

const meta = {
  title: 'Charts/DegradationChart',
  component: DegradationChart,
  parameters: { layout: 'padded' },
  args: { height: 240 },
} satisfies Meta<typeof DegradationChart>;

export default meta;
type Story = StoryObj<typeof meta>;

function makeData(points: number, startPct = 100, endPct = 93) {
  const timeline = storyTimeline('2022-01-01', points, 'month', 2);
  const percentages = timeline.map((_, index) => startPct - (startPct - endPct) * (index / (points - 1)) + (Math.random() - 0.5) * 0.4);
  return timeline.map((ts, index) => ({ ts, usable_kwh: Number(((percentages[index]! / 100) * 135).toFixed(2)), rated_kwh: 135, capacity_pct: Number(percentages[index]!.toFixed(2)) }));
}


export const Healthy: Story = {
  args: { data: makeData(18, 100, 96) },
};

export const MildDegradation: Story = {
  args: { data: makeData(18, 99.5, 92) },
};

export const HighDegradation: Story = {
  args: { data: makeData(18, 99, 86) },
};

export const Loading: Story = {
  args: { data: [], loading: true },
};

export const Empty: Story = {
  args: { data: [] },
};
