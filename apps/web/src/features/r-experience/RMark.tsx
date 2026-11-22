import React from 'react';

/** The R logo (v5 trial LOGO()). Decorative: give the surrounding link or heading the name. */
export function RMark({ size = 36, className }: { size?: number; className?: string }) {
  return (
    <svg className={['r-logo', className].filter(Boolean).join(' ')} viewBox="0 0 48 48" width={size} height={size} aria-hidden="true" focusable="false">
      <rect width="48" height="48" rx="14" fill="var(--r-acc)" />
      <path d="M16 35V13h9.5a7 7 0 0 1 0 14H16" fill="none" stroke="var(--r-acc-ink)" strokeWidth="4.6" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M24.5 27c4 0 6.5 2.6 8.5 8" fill="none" stroke="var(--r-acc-ink)" strokeWidth="4.6" strokeLinecap="round" />
    </svg>
  );
}

/** The app name shown to people. */
export const R_APP_NAME = 'R';
