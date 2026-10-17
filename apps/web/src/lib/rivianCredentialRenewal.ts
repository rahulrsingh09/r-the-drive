import type { RivianCredentialRenewalState } from '@panda/types';

export interface RivianCredentialRenewalStatus {
  renewal_state?: RivianCredentialRenewalState | null;
  expected_renewal_at?: string | null;
}

export interface RivianCredentialRenewalNotice {
  label: string;
  message: string;
  due: boolean;
}

export function formatRivianRenewalDate(value?: string | null): string | null {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  }).format(date);
}

const RENEWAL_NOTICES = {
  renewal_due: {
    due: true,
    label: 'Rivian renewal recommended',
    message: 'The estimated 180-day Rivian connection window has passed. Refresh the Rivian login to avoid an interruption.',
  },
  renewal_soon: {
    due: false,
    label: 'Renew Rivian',
    message: 'The Rivian connection is approaching its estimated 180-day renewal date',
  },
} as const;

export function getRivianCredentialRenewalNotice(
  status?: RivianCredentialRenewalStatus | null,
): RivianCredentialRenewalNotice | null {
  const state = status?.renewal_state;
  if (state !== 'renewal_due' && state !== 'renewal_soon') return null;
  const notice = RENEWAL_NOTICES[state];
  if (notice.due) return { ...notice };
  const date = formatRivianRenewalDate(status?.expected_renewal_at);
  return {
    due: notice.due,
    label: notice.label + (date ? ` by ${date}` : ' soon'),
    message: notice.message + (date ? ` on ${date}.` : '.'),
  };
}
