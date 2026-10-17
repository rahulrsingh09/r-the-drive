import { storyTimeline } from './chartStoryTimeline';
import type { Meta, StoryObj } from '@storybook/react';
import { ElevationProfileChart } from './ElevationProfileChart';

const meta = {
  title: 'Charts/ElevationProfileChart',
  component: ElevationProfileChart,
  parameters: { layout: 'padded' },
  args: { height: 180 },
} satisfies Meta<typeof ElevationProfileChart>;

export default meta;
type Story = StoryObj<typeof meta>;

function makeTrack(points = 120) {
  const timeline = storyTimeline('2024-03-10T09:00:00Z', points, 'second', 30);
  const increments = timeline.map((_, index) => [Math.sin(index / 10) * 8, (Math.random() - 0.5) * 5]);
  let altitude = 300;
  return timeline.map((ts, index) => {
    for (const increment of increments[index]!) altitude += increment;
    return { ts, value: Number(altitude.toFixed(1)) };
  });
}


export const Default: Story = {
  args: { data: makeTrack() },
};

export const Metric: Story = {
  args: { data: makeTrack(), unit: 'm' },
};

export const Hilly: Story = {
  args: {
    data: storyTimeline('2024-03-10T09:00:00Z', 200, 'second', 20).map((ts, index) => ({
      ts, value: Number((250 + Math.sin(index / 8) * 120 + index * 0.5).toFixed(1)),
    })),
  },
};

export const Loading: Story = {
  args: { data: [], loading: true },
};
