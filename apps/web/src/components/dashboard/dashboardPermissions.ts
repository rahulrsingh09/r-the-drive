export function canManageSystemDashboards(role: string | null | undefined) {
  return role === 'admin' || role === 'super_user';
}
