import { expect, it } from 'vitest';
import { canManageSystemDashboards } from '../components/dashboard/DashboardPage';

it('treats super users as system-dashboard managers', () => {
    expect(canManageSystemDashboards('super_user')).toBe(true);
    expect(canManageSystemDashboards('admin')).toBe(true);
    expect(canManageSystemDashboards('user')).toBe(false);
  });
