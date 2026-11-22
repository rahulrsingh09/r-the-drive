import { createRoute, redirect } from '@tanstack/react-router';
import { z } from 'zod';
import { rootRoute } from './__root';
import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { InsightsHub, isInsightTab } from '../features/insights/InsightsHub';

const insightsParentRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/insights',
});

/** "/insights" opens the Cost tab. "/insights?tab=efficiency" (the Now cards and older links) opens that tab. */
const insightsIndexRoute = createRoute({
  getParentRoute: () => insightsParentRoute,
  path: '/',
  validateSearch: z.object({ tab: z.string().optional().catch(undefined) }),
  beforeLoad: ({ search }) => {
    throw redirect({ to: '/insights/$tab', params: { tab: isInsightTab(search.tab) ? search.tab : 'cost' }, replace: true });
  },
});

/** "/insights/cost", "/insights/charging", "/insights/battery", "/insights/efficiency", "/insights/lifetime". */
export const insightsTabRoute = createRoute({
  getParentRoute: () => insightsParentRoute,
  path: '$tab',
  beforeLoad: ({ params }) => {
    if (!isInsightTab(params.tab)) throw redirect({ to: '/insights/$tab', params: { tab: 'cost' }, replace: true });
  },
  component: function InsightsTabPage() {
    const { tab } = insightsTabRoute.useParams();
    return <ProtectedRoute><InsightsHub tab={isInsightTab(tab) ? tab : 'cost'} /></ProtectedRoute>;
  },
});

export const insightsRoute = insightsParentRoute.addChildren([insightsIndexRoute, insightsTabRoute]);
