import React, { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '@iconify/react';
import { Search, X } from 'lucide-react';
import { api } from '@panda/hooks';
import { isAbortError, reportClientError } from '@panda/ui/lib/clientDiagnostics';
import { resolveIconId } from './iconMigration';
import '../iconPolicy';

interface IconPickerProps {
  value: string | undefined;
  onChange: (next: string) => void;
}

const RECENT_KEY = 'panda:icon-picker:recent';
const RECENT_MAX = 12;
const COLLECTIONS = [
  { id: '', label: 'All' },
  { id: 'lucide', label: 'Lucide' },
  { id: 'mdi', label: 'Material' },
  { id: 'heroicons', label: 'Heroicons' },
  { id: 'ph', label: 'Phosphor' },
  { id: 'tabler', label: 'Tabler' },
];

function recentStorage(next?: string[]): string[] {
  if (typeof window === 'undefined') return [];
  try {
    if (next) {
      window.localStorage.setItem(RECENT_KEY, JSON.stringify(next.slice(0, RECENT_MAX)));
      return next;
    }
    const stored: unknown = JSON.parse(window.localStorage.getItem(RECENT_KEY) ?? 'null');
    const icons: string[] = [];
    if (Array.isArray(stored)) for (const icon of stored) if (typeof icon === 'string') icons.push(icon);
    return icons;
  } catch {
    return [];
  }
}

async function searchIcons(query: string, collection: string, signal: AbortSignal): Promise<string[]> {
  const params = new URLSearchParams({ query, limit: '40' });
  if (collection) params.set('prefix', collection);
  const response = await api.proxyFetch(`/v1/external/iconify/search?${params}`, { signal });
  if (!response.ok) return [];
  const data = await response.json() as { icons?: string[] };
  return Array.isArray(data.icons) ? data.icons : [];
}

function useIconSearch(open: boolean, query: string, collection: string) {
  const [state, dispatch] = React.useReducer((current: { results: string[]; loading: boolean }, action:
    { kind: 'begin' } | { kind: 'clear' } | { kind: 'finish'; results: string[] }) => {
    switch (action.kind) {
      case 'begin': return { ...current, loading: true };
      case 'clear': return { ...current, results: [] };
      case 'finish': return { results: action.results, loading: false };
    }
  }, { results: [], loading: false });
  useEffect(() => {
    if (!open) return;
    const normalized = query.trim();
    if (normalized.length < 2) { dispatch({ kind: 'clear' }); return; }
    const controller = new AbortController();
    dispatch({ kind: 'begin' });
    const timer = window.setTimeout(() => {
      void searchIcons(normalized, collection, controller.signal).catch(error => {
        if (!isAbortError(error)) reportClientError(error, {
          event: 'iconify.search_failed', area: 'asset', operation: 'icon-search', provider: 'iconify', severity: 'warn',
        });
        return [];
      }).then(results => {
        if (!controller.signal.aborted) dispatch({ kind: 'finish', results });
      });
    }, 200);
    return () => { window.clearTimeout(timer); controller.abort(); };
  }, [open, query, collection]);
  return state;
}

interface PopoverPos {
  top?: number;
  bottom?: number;
  right: number;
}

export function IconPicker({ value, onChange }: IconPickerProps) {
  const resolved = resolveIconId(value);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [collection, setCollection] = useState('');
  const { results, loading } = useIconSearch(open, query, collection);
  const [recent, setRecent] = useState<string[]>(() => recentStorage());
  const [popoverPos, setPopoverPos] = useState<PopoverPos | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    setPopoverPos(null);
    if (!open) return;
    const trigger = triggerRef.current;
    if (trigger) {
      const rect = trigger.getBoundingClientRect();
      const below = window.innerHeight - rect.bottom - 8;
      const position: PopoverPos = { right: Math.max(8, window.innerWidth - rect.right) };
      if (below >= 440 || below >= rect.top - 8) position.top = rect.bottom + 6;
      else position.bottom = window.innerHeight - rect.top + 6;
      setPopoverPos(position);
    }
    const dismiss = (event: MouseEvent | KeyboardEvent) => {
      if ('key' in event) {
        if (event.key === 'Escape') setOpen(false);
        return;
      }
      const target = event.target as Node;
      const inside = [popoverRef.current, triggerRef.current].some(element => element?.contains(target));
      if (!inside) setOpen(false);
    };
    document.addEventListener('mousedown', dismiss);
    document.addEventListener('keydown', dismiss);
    return () => {
      document.removeEventListener('mousedown', dismiss);
      document.removeEventListener('keydown', dismiss);
    };
  }, [open]);

  const recentList = useMemo(() => recent.slice(0, RECENT_MAX), [recent]);

  function selectIcon(id: string) {
    onChange(id);
    const next = [id, ...recent.filter((e) => e !== id)].slice(0, RECENT_MAX);
    setRecent(next);
    recentStorage(next);
    setOpen(false);
  }

  const popover = open && popoverPos ? createPortal(
    <div
      ref={popoverRef}
      style={{
        position: 'fixed',
        top: popoverPos.top,
        bottom: popoverPos.bottom,
        right: popoverPos.right,
        width: '23rem',
        zIndex: 9999,
        backgroundColor: 'var(--rm-bg-elevated)',
        border: '1px solid var(--rm-border-default)',
        borderRadius: '12px',
        boxShadow: 'var(--rm-shadow-xl)',
        display: 'flex',
        flexDirection: 'column',
        maxHeight: '460px',
        overflow: 'hidden',
      }}
    >
      {/* Search bar */}
      <div
        style={{ backgroundColor: 'var(--rm-bg-surface)', borderRadius: '12px 12px 0 0' }}
        className="flex shrink-0 items-center gap-2 border-b border-border p-2.5"
      >
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-fg-tertiary" />
          <input
            autoFocus
            type="text"
            placeholder="Search icons…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="w-full rounded-lg border border-border bg-bg pl-8 pr-3 py-1.5 text-xs text-fg outline-none placeholder:text-fg-tertiary focus:border-accent"
          />
        </div>
        <button
          type="button"
          onClick={() => setOpen(false)}
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-border text-fg-tertiary hover:border-border-strong hover:text-fg"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Collection pills */}
      <div className="flex shrink-0 flex-wrap gap-1.5 border-b border-border px-3 py-2">
        {COLLECTIONS.map((c) => (
          <button
            key={c.id || 'all'}
            type="button"
            onClick={() => setCollection(c.id)}
            className={`rounded-full border px-2.5 py-0.5 text-[10px] font-medium uppercase tracking-wider transition-colors ${
              collection === c.id
                ? 'border-accent bg-accent/15 text-accent'
                : 'border-border text-fg-tertiary hover:border-border-strong hover:text-fg'
            }`}
          >
            {c.label}
          </button>
        ))}
      </div>

      {/* Icon grid (scrollable) */}
      <div className="flex-1 overflow-y-auto p-3">
        {recentList.length > 0 && !query.trim() ? (
          <div className="mb-3">
            <p className="mb-2 text-[10px] font-medium uppercase tracking-wider text-fg-tertiary">
              Recent
            </p>
            <IconGrid icons={recentList} selected={resolved} onSelect={selectIcon} />
          </div>
        ) : null}

        {query.trim().length >= 2 ? (
          loading ? (
            <p className="py-8 text-center text-xs text-fg-tertiary">Searching…</p>
          ) : results.length === 0 ? (
            <p className="py-8 text-center text-xs text-fg-tertiary">No icons found</p>
          ) : (
            <>
              <p className="mb-2 text-[10px] font-medium uppercase tracking-wider text-fg-tertiary">
                Results
              </p>
              <IconGrid icons={results} selected={resolved} onSelect={selectIcon} />
            </>
          )
        ) : query.trim().length === 1 ? (
          <p className="py-8 text-center text-xs text-fg-tertiary">Type at least 2 characters</p>
        ) : recentList.length === 0 ? (
          <p className="py-8 text-center text-xs text-fg-tertiary">Search for an icon above</p>
        ) : null}
      </div>
    </div>,
    document.body
  ) : null;

  return (
    <div className="relative">
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((prev) => !prev)}
        className="flex w-full items-center gap-2.5 rounded-lg border border-border bg-bg-elevated px-3 py-2 text-sm text-fg outline-none transition-colors hover:border-border-strong focus:border-accent"
      >
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-border bg-bg-surface">
          <Icon icon={resolved} className="h-4 w-4 text-accent" />
        </span>
        <span className="min-w-0 flex-1 truncate text-left font-mono text-xs text-fg-secondary">
          {resolved}
        </span>
        <span className="shrink-0 text-[11px] text-fg-tertiary">Change</span>
      </button>
      {popover}
    </div>
  );
}

function IconGrid({
  icons,
  selected,
  onSelect,
}: {
  icons: string[];
  selected: string;
  onSelect: (id: string) => void;
}) {
  return (
    <div className="grid grid-cols-4 gap-2">
      {icons.map((id) => {
        const isSelected = id === selected;
        return (
          <button
            key={id}
            type="button"
            title={id}
            onClick={() => onSelect(id)}
            className={`flex aspect-square w-full flex-col items-center justify-center gap-1.5 rounded-xl border transition-all ${
              isSelected
                ? 'border-accent bg-accent/15 text-accent'
                : 'border-border bg-bg-surface text-fg-secondary hover:border-accent/60 hover:bg-accent/8 hover:text-accent'
            }`}
          >
            <Icon icon={id} className="h-7 w-7" />
            <span className="max-w-full truncate px-1 text-[8px] leading-tight text-fg-tertiary">
              {id.split(':')[1] ?? id}
            </span>
          </button>
        );
      })}
    </div>
  );
}
