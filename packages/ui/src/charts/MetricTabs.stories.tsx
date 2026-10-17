import type { Meta, StoryObj } from '@storybook/react';
import React, { useReducer } from 'react';
import { MetricTabs, type MetricTab } from '../primitives/MetricTabs';
import { Battery, TrendingDown, Moon, Activity } from 'lucide-react';

const meta = {
  title: 'Primitives/MetricTabs',
  component: MetricTabs,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof MetricTabs>;

export default meta;
type Story = StoryObj<typeof meta>;

const TABS = [
  { key: 'soc',     label: 'State of Charge', icon: <Battery className="w-3.5 h-3.5" /> },
  { key: 'range',   label: 'Range',            icon: <Activity className="w-3.5 h-3.5" /> },
  { key: 'phantom', label: 'Phantom Drain',    icon: <Moon className="w-3.5 h-3.5" /> },
  { key: 'degrad',  label: 'Degradation',      icon: <TrendingDown className="w-3.5 h-3.5" /> },
];

function InteractiveWrapper({ tabs = TABS }: { tabs?: MetricTab[] }) {
  const [active, setActive] = useReducer((_current: string, selected: string) => selected, tabs[0]?.key ?? '');
  return (
    <MetricTabs tabs={tabs} active={active} onChange={setActive} title="Battery" subtitle="30-day history">
      <div className="h-48 flex items-center justify-center text-fg-tertiary text-sm">
        Content for: <strong className="ml-1 text-fg">{active}</strong>
      </div>
    </MetricTabs>
  );
}

export const Default: Story = {
  render: () => <InteractiveWrapper />,
  args: { tabs: TABS, active: 'soc', onChange: () => {}, children: null },
};

const MANY_TABS: MetricTab[] = [
  { key: 't0', label: 'Metric 1' }, { key: 't1', label: 'Metric 2' }, { key: 't2', label: 'Metric 3' },
  { key: 't3', label: 'Metric 4' }, { key: 't4', label: 'Metric 5' }, { key: 't5', label: 'Metric 6' }, { key: 't6', label: 'Metric 7' },
];
export const DropdownMode: Story = {
  render: () => <InteractiveWrapper tabs={MANY_TABS} />,
  args: { tabs: [], active: 't0', onChange: () => {}, children: null },
};
