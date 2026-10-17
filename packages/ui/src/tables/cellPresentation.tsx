import type { ReactNode } from 'react';
import { PiArrowFatLinesRight } from 'react-icons/pi';
import { formatPercent } from '../lib/utils';

export function cellText(value: ReactNode, className: string, empty = '-', missing = value == null) {
  const content = missing ? { className: 'text-fg-tertiary', children: empty } : { className, children: value };
  return <span {...content} />;
}

export function socRange(start: number | null, end: number | null, compact = false) {
  const missing = compact ? start == null || end == null : start === null || end === null;
  if (missing) return cellText(null, '');
  const values = [start, end].map(value => formatPercent(value!, 0));
  const presentation = compact
    ? ['inline-flex items-center gap-1 whitespace-nowrap font-mono text-fg', 'h-3.5 w-3.5 shrink-0 text-fg-tertiary']
    : ['inline-flex items-center gap-1.5 whitespace-nowrap font-mono text-fg', 'h-3.5 w-3.5 text-fg-tertiary'];
  return <span className={presentation[0]}>{values[0]}<PiArrowFatLinesRight className={presentation[1]} />{values[1]}</span>;
}
