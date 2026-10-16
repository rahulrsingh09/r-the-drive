import React from 'react';
import { CircleHelp } from 'lucide-react';

export function RLockIcon({ locked }: { locked: boolean | null }) {
  if (locked === null) return <CircleHelp className="r-lock-icon" aria-hidden="true" />;
  return <svg className="r-lock-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor"
    strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"
    data-locked={locked}>
    <path className="r-lock-shackle" d="M8 10V6a4 4 0 0 1 8 0v4" />
    <rect x="5" y="10" width="14" height="11" rx="2" />
    <path d="M12 15v2" />
  </svg>;
}
