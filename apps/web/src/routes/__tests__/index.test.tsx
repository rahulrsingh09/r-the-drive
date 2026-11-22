import React from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

// "/" is the Now page, not a dashboard. The Now view has its own tests in src/features/now.
vi.mock('../../features/now/NowPage', () => ({ NowPage: () => <h1>Now page</h1> }));
vi.mock('../../components/layout/ProtectedRoute', () => ({ ProtectedRoute: ({ children }: { children: React.ReactNode }) => <>{children}</> }));
vi.mock('../../components/dashboard/DashboardPageShell', () => ({ DashboardPageShell: () => { throw new Error('Home must not render the dashboard shell'); } }));

import { indexRoute } from '../index';

describe('index route', () => {
  it('renders the Now page on "/"', () => {
    const Page = indexRoute.options.component as React.ComponentType;
    render(<Page />);
    expect((indexRoute.options as { path?: string }).path).toBe('/');
    expect(screen.getByRole('heading', { name: 'Now page' })).toBeInTheDocument();
  });
});
