import { tagChoices, tagBadgeWindow, commonTripTags, changeTagSelection, staleTagSelection, tagCatalog, createOrRecoverTag, tagKeyIntent, cyclePickerFocus } from './tagPickerModel';
import { useWidgetMediaQuery } from '../widgetLifecycle';
import React from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown, Plus, Search, Tag, X } from 'lucide-react';
import { Button, Input, Badge } from '@panda/ui/primitives';
import { useCreateTripTag, useTripTags } from '@panda/hooks';
import type { TripTag } from '@panda/types';
import { formatTripTagName, TripTagBadge } from '@panda/ui/tables';

export interface TripTagPickerProps {
  vehicleId: string | null;
  canManage: boolean;
  selectedIds: string[];
  onChange: (tagIds: string[]) => void;
  label?: string;
  disabled?: boolean;
  mode?: 'popover' | 'inline';
  inlineClassName?: string;
  mixed?: boolean;
}

export function deriveCommonTagIds(trips: Array<{ tags?: Array<Pick<TripTag, 'id'>> | undefined }>): string[] {
  return commonTripTags(trips);
}

function useMobilePicker() {
  return useWidgetMediaQuery('(max-width: 639px)');
}

/** Compact multi-select for shared vehicle tags. It is deliberately local-filtered so every keystroke is instant. */
export function TripTagPicker({ vehicleId, canManage, selectedIds, onChange, label = 'Filter tags', disabled, mode, inlineClassName, mixed = false }: TripTagPickerProps) {
  const inline = mode === 'inline';
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState('');
  const [activeIndex, setActiveIndex] = React.useState(0);
  const triggerRef = React.useRef<HTMLButtonElement>(null);
  const searchRef = React.useRef<HTMLInputElement>(null);
  const panelRef = React.useRef<HTMLDivElement>(null);
  const pendingCatalogIds = React.useRef(new Set<string>());
  const cleanedStaleSignature = React.useRef<string | null>(null);
  const isMobile = useMobilePicker();
  const tagsQuery = useTripTags(vehicleId);
  const createTag = useCreateTripTag(vehicleId);
  const tags = tagsQuery.data ?? [];
  const { normalizedQuery, filtered, exact, selected } = React.useMemo(() => tagCatalog(tags, selectedIds, query), [tags, selectedIds, query]);
  const [catalogNotice, setCatalogNotice] = React.useState('');
  const [createError, setCreateError] = React.useState('');

  const wasOpen = React.useRef(false);
  React.useEffect(() => {
    const opening = open && !wasOpen.current;
    wasOpen.current = open;
    if (opening) {
      setActiveIndex(0);
      requestAnimationFrame(() => searchRef.current?.focus());
    }
    const subscriptions: Array<[string, EventListener]> = [];
    if (open && !isMobile) subscriptions.push(['mousedown', (event) => {
      const inside = triggerRef.current?.parentElement?.contains(event.target as Node);
      if (inside) return;
      setOpen(false);
    }]);
    for (const [event, listener] of subscriptions) document.addEventListener(event, listener);
    return () => {
      for (const [event, listener] of subscriptions) document.removeEventListener(event, listener);
    };
  }, [isMobile, open]);

  React.useEffect(() => {
    if (!tagsQuery.isSuccess || selectedIds.length === 0) return;
    const { stale, retained } = staleTagSelection(tags, selectedIds, pendingCatalogIds.current);
    const signature = stale.length ? stale.join(',') : null;
    if (signature === null) { cleanedStaleSignature.current = null; return; }
    if (cleanedStaleSignature.current === signature) return;
    cleanedStaleSignature.current = signature;
    onChange(retained);
    setCatalogNotice(stale.length + ' unavailable tag' + (stale.length === 1 ? ' was' : 's were') + ' removed.');
  }, [onChange, selectedIds, tags, tagsQuery.isSuccess]);

  const select = (id: string) => onChange(changeTagSelection(selectedIds, id, 'toggle'));
  const remove = (id: string) => onChange(changeTagSelection(selectedIds, id, 'remove'));
  const addExisting = (id: string) => { onChange(changeTagSelection(selectedIds, id, 'add')); setQuery(''); };
  const create = async () => {
    if (!canManage || !query.trim() || exact) return;
    setCreateError('');
    const result = await createOrRecoverTag(() => createTag.mutateAsync({ name: formatTripTagName(query) }), () => tagsQuery.refetch(), normalizedQuery);
    if (!result) { setCreateError('Couldn’t create this tag. Check your connection and try again.'); return; }
    if (result.created) pendingCatalogIds.current.add(result.tag.id);
    onChange([...selectedIds, result.tag.id].sort());
    setQuery('');
  };
  const choices = tagChoices(filtered, selectedIds, activeIndex);
  const optionCount = filtered.length + (canManage && query.trim() && !exact ? 1 : 0);

  function handleKeys(event: React.KeyboardEvent<HTMLElement>) {
    if (!inline && isMobile) cyclePickerFocus(event, panelRef.current);
    const intent = tagKeyIntent(event.key, inline, query, activeIndex, filtered, selectedIds, canManage && Boolean(query.trim()) && !exact, optionCount);
    if (!intent) return;
    if (intent.type !== 'remove') event.preventDefault();
    switch (intent.type) {
      case 'index': setActiveIndex(intent.value); break;
      case 'close': setOpen(false); triggerRef.current?.focus(); break;
      case 'pick': (inline ? addExisting : select)(intent.id); break;
      case 'remove': remove(intent.id); break;
      case 'create': void create(); break;
    }
  }

  if (inline) {
    return (
      <div className="relative min-w-0">
        <div className={`flex min-h-11 flex-wrap items-center gap-1 transition-colors ${inlineClassName ?? 'rounded-lg border border-border bg-bg-surface px-2 py-1 focus-within:border-accent focus-within:ring-1 focus-within:ring-accent'}`}>
          {selected.map((tag) => (
            <button
              type="button"
              key={tag.id}
              onClick={() => remove(tag.id)}
              className="inline-flex min-h-8 items-center gap-1 rounded-md border border-accent/20 bg-accent-muted px-1.5 text-sm text-accent focus:outline-none focus-visible:ring-1 focus-visible:ring-accent"
              aria-label={`Remove ${formatTripTagName(tag.name)}`}
              disabled={disabled}
            >
              <Tag className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
              <span className="max-w-[8rem] truncate" title={formatTripTagName(tag.name)}>{formatTripTagName(tag.name)}</span>
              <X className="h-3.5 w-3.5" aria-hidden="true" />
            </button>
          ))}
          <input
            ref={searchRef}
            value={query}
            disabled={disabled}
            onChange={(event) => { setQuery(event.target.value); setCreateError(''); }}
            onKeyDown={handleKeys}
            placeholder={mixed ? 'Mixed tags · add or replace' : selected.length ? 'Add another tag' : label}
            aria-label={mixed ? `${label} (mixed tags across selected trips)` : label}
            aria-autocomplete="list"
            aria-controls="trip-tag-inline-suggestions"
            className="h-8 min-w-[8rem] flex-1 border-0 bg-transparent px-1 text-base text-fg outline-none placeholder:text-fg-tertiary focus:ring-0 focus-visible:outline-none focus-visible:outline-offset-0 sm:text-sm"
            style={{ outline: 'none', boxShadow: 'none' }}
          />
        </div>
        {catalogNotice ? <p className="sr-only" role="status" aria-live="polite">{catalogNotice}</p> : null}
        {query.trim() ? (
          <div id="trip-tag-inline-suggestions" className="mt-1 max-h-48 overflow-y-auto rounded-lg border border-border bg-bg-surface p-1 shadow-sm" role="listbox" aria-label={`${label} suggestions`}>
            {tagsQuery.isLoading ? <p className="px-2 py-2 text-sm text-fg-tertiary">Loading tags…</p> : null}
            {tagsQuery.isError ? <div className="px-2 py-2 text-sm text-status-danger">Couldn’t load tags. <button type="button" className="font-medium underline underline-offset-2" onClick={() => void tagsQuery.refetch()}>Retry</button></div> : null}
            {!tagsQuery.isLoading && !tagsQuery.isError && choices.map(({ tag, selected, active }) => (
              <button key={tag.id} type="button" role="option" aria-selected={selected} onClick={() => addExisting(tag.id)} className={`flex min-h-10 w-full items-center rounded-md px-2 text-left text-sm hover:bg-bg-elevated ${active ? 'bg-bg-elevated' : ''}`}>
                <TripTagBadge tag={tag} />
                {selected ? <Check className="ml-auto h-4 w-4 text-accent" aria-hidden="true" /> : null}
              </button>
            ))}
            {!tagsQuery.isLoading && !tagsQuery.isError && filtered.length === 0 && !canManage ? <p className="px-2 py-2 text-sm text-fg-tertiary">No matching tags.</p> : null}
            {canManage && !exact ? <button type="button" role="option" disabled={createTag.isPending} onClick={() => void create()} className={`flex min-h-10 w-full items-center gap-2 rounded-md px-2 text-left text-sm font-medium text-accent hover:bg-accent/10 disabled:opacity-50 ${activeIndex === filtered.length ? 'bg-accent/10' : ''}`}><Plus className="h-4 w-4" aria-hidden="true" />Create “{formatTripTagName(query)}”</button> : null}
          </div>
        ) : null}
        {createError ? <p className="mt-2 rounded-lg border border-status-danger/30 bg-status-danger/10 px-3 py-2 text-sm text-status-danger" role="alert">{createError}</p> : null}
      </div>
    );
  }

  const panel = open ? (
    <div
      ref={panelRef}
      className={isMobile
        ? 'fixed inset-x-0 bottom-0 z-50 max-h-[80vh] rounded-t-2xl border border-border bg-bg-surface p-4 shadow-xl'
        : 'absolute left-0 top-full z-40 mt-2 w-[min(24rem,calc(100vw-2rem))] rounded-xl border border-border bg-bg-surface p-3 shadow-xl'}
      role={isMobile ? 'dialog' : 'listbox'}
      aria-modal={isMobile || undefined}
      aria-label={label}
      onKeyDown={handleKeys}
    >
      {isMobile ? <div className="mb-3 flex items-center justify-between"><h2 className="text-base font-semibold text-fg">{label}</h2><Button type="button" variant="ghost" size="md" className="h-11 w-11 px-0" onClick={() => setOpen(false)} aria-label="Close tag picker"><X className="h-4 w-4" /></Button></div> : null}
      <Input ref={searchRef} value={query} onChange={(event) => { setQuery(event.target.value); setCreateError(''); }} placeholder="Search tags" iconLeft={<Search className="h-4 w-4" />} className="h-11" aria-label="Search existing tags" />
      <p className="mt-2 text-xs text-fg-tertiary" role="status" aria-live="polite">{query.trim() ? `${filtered.length} matching tag${filtered.length === 1 ? '' : 's'}` : `${tags.length} shared tag${tags.length === 1 ? '' : 's'}`}</p>
      {catalogNotice ? <p className="sr-only" role="status" aria-live="polite">{catalogNotice}</p> : null}
      <div className="mt-3 max-h-64 overflow-y-auto" role={isMobile ? 'listbox' : undefined}>
        {tagsQuery.isLoading ? <p className="px-2 py-4 text-sm text-fg-tertiary">Loading tags…</p> : null}
        {tagsQuery.isError ? <div className="px-2 py-3 text-sm text-status-danger">Couldn’t load tags. <button type="button" className="font-medium underline underline-offset-2" onClick={() => void tagsQuery.refetch()}>Retry</button></div> : null}
        {!tagsQuery.isLoading && !tagsQuery.isError && choices.map(({ tag, selected, active }) => (
          <button key={tag.id} type="button" role="option" aria-selected={selected} onClick={() => select(tag.id)} className={`flex min-h-11 w-full items-center justify-between gap-3 rounded-lg px-2 text-left transition-colors ${active ? 'bg-bg-elevated' : 'hover:bg-bg-elevated'}`}>
            <TripTagBadge tag={tag} />
            {selected ? <Check className="h-4 w-4 shrink-0 text-accent" /> : null}
          </button>
        ))}
        {!tagsQuery.isLoading && !tagsQuery.isError && filtered.length === 0 && !query.trim() ? <p className="px-2 py-4 text-sm text-fg-tertiary">No shared tags yet{canManage ? '. Create one to organize your trips.' : '.'}</p> : null}
            {!tagsQuery.isLoading && !tagsQuery.isError && filtered.length === 0 && query.trim() && !canManage ? <p className="px-2 py-4 text-sm text-fg-tertiary">No matching tags.</p> : null}
        {canManage && query.trim() && !exact ? <button type="button" disabled={createTag.isPending} onClick={() => void create()} className={`mt-1 flex min-h-11 w-full items-center gap-2 rounded-lg px-2 text-left text-sm font-medium text-accent transition-colors hover:bg-accent/10 disabled:opacity-50 ${activeIndex === filtered.length ? 'bg-accent/10' : ''}`}><Plus className="h-4 w-4" />Create “{formatTripTagName(query)}”</button> : null}
      </div>
      {createError ? <p className="mt-2 rounded-lg border border-status-danger/30 bg-status-danger/10 px-3 py-2 text-sm text-status-danger" role="alert">{createError}</p> : null}
      {isMobile ? <Button type="button" variant="secondary" size="md" className="mt-3 h-11 w-full" onClick={() => setOpen(false)}>Done</Button> : null}
    </div>
  ) : null;

  return (
    <div className="relative min-w-0">
      <button ref={triggerRef} type="button" disabled={disabled} onClick={() => setOpen((value) => !value)} className="flex min-h-11 w-full items-center gap-2 rounded-lg border border-border bg-bg-surface px-3 text-left text-sm text-fg transition-colors hover:border-border-strong focus:outline-none focus:ring-1 focus:ring-accent disabled:opacity-50" aria-expanded={open} aria-haspopup={isMobile ? 'dialog' : 'listbox'}>
        <span className="truncate">{selected.length ? `${selected.length} tag${selected.length === 1 ? '' : 's'} selected` : label}</span><ChevronDown className="ml-auto h-4 w-4 shrink-0 text-fg-tertiary" />
      </button>
      {selected.length ? <div className="mt-2 flex flex-wrap gap-1">{selected.map((tag) => <button type="button" key={tag.id} onClick={() => remove(tag.id)} className="inline-flex h-11 items-center gap-1 rounded-lg px-1.5 focus:outline-none focus:ring-1 focus:ring-accent" aria-label={`Remove ${formatTripTagName(tag.name)}`}><TripTagBadge tag={tag} /><X className="h-3.5 w-3.5 text-fg-tertiary" /></button>)}</div> : null}
      {isMobile && open ? createPortal(<div className="fixed inset-0 z-40 bg-bg-page/70" aria-hidden="true" onClick={() => setOpen(false)} />, document.body) : null}
      {isMobile && open ? createPortal(panel, document.body) : panel}
    </div>
  );
}

export function TripTagBadges({ tags }: { tags?: TripTag[] | undefined }) {
  const { visible, extra } = tagBadgeWindow(tags);
  if (!visible.length) return null;
  return <div className="mt-2 flex flex-wrap gap-1">{visible.map((tag) => <TripTagBadge key={tag.id} tag={tag} />)}{extra > 0 ? <Badge size="sm">+{extra}</Badge> : null}</div>;
}
