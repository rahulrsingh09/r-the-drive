import React from 'react';

/** Line icons from the v5 trial (head.js). Decorative: the control that holds the icon carries the name. */
type IconProps = { className?: string };
const line = { fill: 'none', stroke: 'currentColor', strokeWidth: 1.8, strokeLinecap: 'round', strokeLinejoin: 'round' } as const;
const svg = (children: React.ReactNode, props: React.SVGProps<SVGSVGElement> = line) =>
  function Icon({ className }: IconProps) {
    return <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false" className={className} {...props}>{children}</svg>;
  };

export const RIconNow = svg(<><path d="M3 15.5 5 10a3 3 0 0 1 2.8-2h8.4A3 3 0 0 1 19 10l2 5.5V18a1 1 0 0 1-1 1h-1.5a1 1 0 0 1-1-1v-1H6.5v1a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z" /><path d="M6.5 13.5h.01M17.5 13.5h.01" /></>);
export const RIconActivity = svg(<><path d="M4 6h10M4 12h16M4 18h7" /><circle cx="18" cy="6" r="2" /><circle cx="15" cy="18" r="2" /></>);
export const RIconInsights = svg(<path d="M4 20V10M10 20V4M16 20v-7M22 20H2" />);
export const RIconCar = svg(<><rect x="6" y="2.5" width="12" height="19" rx="4" /><path d="M8 8h8M8 16h8M3.5 7v3M20.5 7v3M3.5 14v3M20.5 14v3" /></>);
export const RIconGear = svg(<><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" /></>);
export const RIconSun = svg(<><circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" /></>, { ...line, strokeWidth: 2 });
export const RIconMoon = svg(<path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z" />, { ...line, strokeWidth: 2 });
export const RIconOffline = svg(<path d="M2 2l20 20M8.5 16.4a5 5 0 0 1 7 0M5 12.9a10 10 0 0 1 5.1-2.8M14.8 10.3A10 10 0 0 1 19 12.9M12 20h.01" />, { ...line, strokeWidth: 2 });
export const RIconWarn = svg(<><path d="M12 3 2 20h20z" /><path d="M12 10v4M12 17v.01" /></>, { ...line, strokeWidth: 2 });
export const RIconInfo = svg(<><circle cx="12" cy="12" r="9" /><path d="M12 11v6M12 7.5v.01" /></>, { ...line, strokeWidth: 2 });
export const RIconCheck = svg(<path d="m5 12 5 5 9-10" />, { ...line, strokeWidth: 2.4 });
export const RIconBack = svg(<path d="m15 18-6-6 6-6" />, { ...line, strokeWidth: 2.2 });
export const RIconChevron = svg(<path d="m6 9 6 6 6-6" />, { ...line, strokeWidth: 2.2 });
