import { act, renderHook } from '@testing-library/react';
import { StrictMode, useLayoutEffect } from 'react';
import { describe, expect, it, vi } from 'vitest';
import type { DashboardConfig } from '@panda/dashboards';
import {
  useDashboardEditDraft,
  type UseDashboardEditDraftArgs,
} from '../components/dashboard/useDashboardEditDraft';

function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === 'object') {
    for (const child of Object.values(value)) deepFreeze(child);
    Object.freeze(value);
  }
  return value;
}

function dashboard(overrides: Partial<DashboardConfig> = {}): DashboardConfig {
  return deepFreeze({
    schemaVersion: 2,
    id: '10000000-0000-4000-8000-000000000001',
    slug: 'battery',
    name: 'Battery',
    isDefault: false,
    isLocked: false,
    ownerId: '20000000-0000-4000-8000-000000000001',
    controls: { dateRange: true },
    widgets: [{
      id: '30000000-0000-4000-8000-000000000001',
      componentType: 'sensor',
      definitionId: 'total_miles',
      title: 'Distance',
      layout: { x: 0, y: 0, w: 3, h: 2 },
      options: { showSparkline: true },
    }],
    ...overrides,
  });
}

function renderDraft(args: UseDashboardEditDraftArgs) {
  return renderHook((props: UseDashboardEditDraftArgs) => useDashboardEditDraft(props), {
    initialProps: args,
  });
}

describe('dashboard edit draft contracts', () => {
  it('shows the latest saved configuration in view mode without creating a draft', () => {
    const saved = dashboard();
    const refreshed = dashboard({ name: 'Updated battery' });
    const { result, rerender } = renderDraft({ savedConfig: saved, slug: saved.slug });

    expect(result.current).toMatchObject({
      activeConfig: saved,
      localConfig: null,
      isEditMode: false,
      isDirty: false,
      saveError: null,
    });

    rerender({ savedConfig: refreshed, slug: refreshed.slug });

    expect(result.current.activeConfig).toEqual(refreshed);
    expect(result.current.localConfig).toBeNull();
    expect(result.current.isDirty).toBe(false);
  });

  it('enters editing from saved data and discards changes and errors when exiting', () => {
    const saved = dashboard();
    const original = structuredClone(saved);
    const { result } = renderDraft({ savedConfig: saved, slug: saved.slug });

    act(() => result.current.setSaveError('Previous save failed'));
    act(() => result.current.enterEdit());

    expect(result.current).toMatchObject({
      activeConfig: saved,
      localConfig: saved,
      isEditMode: true,
      isDirty: false,
      saveError: null,
    });

    act(() => result.current.setLocalConfig(current => current && ({
      ...current,
      name: 'My battery',
      widgets: current.widgets.map(widget => ({
        ...widget,
        layout: { ...widget.layout, w: 6 },
        options: { ...widget.options, showSparkline: false },
      })),
    })));

    expect(result.current.activeConfig?.name).toBe('My battery');
    expect(result.current.activeConfig?.widgets[0]).toMatchObject({
      layout: { w: 6 },
      options: { showSparkline: false },
    });
    expect(result.current.isDirty).toBe(true);
    expect(saved).toEqual(original);

    act(() => result.current.setSaveError('Could not save dashboard'));
    expect(result.current.isEditMode).toBe(true);
    expect(result.current.isDirty).toBe(true);
    expect(result.current.saveError).toBe('Could not save dashboard');

    act(() => result.current.exitEdit());

    expect(result.current).toMatchObject({
      activeConfig: saved,
      localConfig: null,
      isEditMode: false,
      isDirty: false,
      saveError: null,
    });

    act(() => result.current.enterEdit());
    expect(result.current.activeConfig).toEqual(original);
    expect(result.current.isDirty).toBe(false);
    expect(saved).toEqual(original);
  });

  it.each(['replacement', 'updater'] as const)(
    'clears a save error after a %s edit without losing the draft',
    (setter) => {
      const saved = dashboard();
      const draft = dashboard({ name: 'My battery' });
      const { result } = renderDraft({ savedConfig: saved, slug: saved.slug });
      act(() => result.current.enterEdit());
      act(() => result.current.setLocalConfig(draft));
      act(() => result.current.setSaveError('Save failed'));

      if (setter === 'replacement') {
        act(() => result.current.setLocalConfig(dashboard({
          ...draft,
          description: 'Daily driving',
        })));
      } else {
        act(() => result.current.setLocalConfig(current => current && ({
          ...current,
          description: 'Daily driving',
        })));
      }

      expect(result.current.saveError).toBeNull();
      expect(result.current.isEditMode).toBe(true);
      expect(result.current.isDirty).toBe(true);
      expect(result.current.activeConfig).toMatchObject({
        name: 'My battery',
        description: 'Daily driving',
      });
      expect(saved.name).toBe('Battery');
      expect(draft.description).toBeUndefined();
    },
  );

  it('treats equal data as clean, nested changes as dirty, and reverting as clean', () => {
    const saved = dashboard();
    const { result } = renderDraft({ savedConfig: saved, slug: saved.slug });
    act(() => result.current.enterEdit());
    act(() => result.current.setLocalConfig(structuredClone(saved)));
    expect(result.current.isDirty).toBe(false);

    act(() => result.current.setLocalConfig(current => current && ({
      ...current,
      widgets: current.widgets.map(widget => ({
        ...widget,
        layout: { ...widget.layout, y: 4 },
      })),
    })));
    expect(result.current.isDirty).toBe(true);

    act(() => result.current.setLocalConfig(structuredClone(saved)));
    expect(result.current.activeConfig).toEqual(saved);
    expect(result.current.isDirty).toBe(false);
  });

  it('requests controlled transitions and waits for the parent to change edit mode', () => {
    const saved = dashboard();
    const onEditModeChange = vi.fn();
    const args = { savedConfig: saved, slug: saved.slug, onEditModeChange };
    const { result, rerender } = renderDraft({ ...args, controlledEditMode: false });

    act(() => result.current.enterEdit());
    expect(onEditModeChange).toHaveBeenLastCalledWith(true);
    expect(result.current.isEditMode).toBe(false);

    rerender({ ...args, controlledEditMode: true });
    expect(result.current.isEditMode).toBe(true);
    expect(result.current.activeConfig).toEqual(saved);
    expect(result.current.isDirty).toBe(false);

    act(() => result.current.setLocalConfig(dashboard({ name: 'My battery' })));
    act(() => result.current.setSaveError('Save failed'));
    act(() => result.current.exitEdit());

    expect(onEditModeChange).toHaveBeenLastCalledWith(false);
    expect(result.current.isEditMode).toBe(true);
    rerender({ ...args, controlledEditMode: false });

    expect(result.current).toMatchObject({
      activeConfig: saved,
      localConfig: null,
      isEditMode: false,
      isDirty: false,
      saveError: null,
    });
    expect(onEditModeChange.mock.calls).toEqual([[true], [false]]);
  });

  it('discards a draft on an external controlled exit and starts fresh on reentry', () => {
    const saved = dashboard();
    const onEditModeChange = vi.fn();
    const args = { savedConfig: saved, slug: saved.slug, onEditModeChange };
    const { result, rerender } = renderDraft({ ...args, controlledEditMode: true });
    expect(result.current.localConfig).toEqual(saved);
    expect(result.current.isDirty).toBe(false);

    act(() => result.current.setLocalConfig(dashboard({ name: 'Unsaved title' })));
    act(() => result.current.setSaveError('Save failed'));
    rerender({ ...args, controlledEditMode: false });

    expect(result.current.activeConfig).toEqual(saved);
    expect(result.current.localConfig).toBeNull();
    expect(result.current.isDirty).toBe(false);
    expect(result.current.saveError).toBeNull();

    rerender({ ...args, controlledEditMode: true });
    expect(result.current.localConfig).toEqual(saved);
    expect(result.current.isDirty).toBe(false);
    expect(onEditModeChange).not.toHaveBeenCalled();
  });

  it.each(['uncontrolled', 'controlled'] as const)(
    'initializes a pending dashboard when data arrives during %s editing',
    (mode) => {
      const saved = dashboard();
      const args: UseDashboardEditDraftArgs = {
        savedConfig: undefined,
        slug: saved.slug,
        controlledEditMode: mode === 'controlled' ? true : undefined,
      };
      const { result, rerender } = renderDraft(args);
      if (mode === 'uncontrolled') act(() => result.current.enterEdit());

      expect(result.current.isEditMode).toBe(true);
      expect(result.current.activeConfig).toBeUndefined();
      expect(result.current.localConfig).toBeNull();
      expect(result.current.isDirty).toBe(false);

      rerender({ ...args, savedConfig: saved });

      expect(result.current.isEditMode).toBe(true);
      expect(result.current.activeConfig).toEqual(saved);
      expect(result.current.localConfig).toEqual(saved);
      expect(result.current.isDirty).toBe(false);
      act(() => result.current.setLocalConfig(dashboard({ name: 'Loaded draft' })));
      expect(result.current.isDirty).toBe(true);
    },
  );

  it.each([
    {
      change: 'exact ID with the same slug',
      overrides: { id: '10000000-0000-4000-8000-000000000002' },
    },
    {
      change: 'slug with the same exact ID',
      overrides: { slug: 'charging' },
    },
    {
      change: 'both exact ID and slug',
      overrides: { id: '10000000-0000-4000-8000-000000000002', slug: 'charging' },
    },
  ])('scopes drafts and save errors when changing $change', ({ overrides }) => {
    const saved = dashboard();
    const next = dashboard({ ...overrides, name: 'Another dashboard' });
    const args = { controlledEditMode: true };
    const { result, rerender } = renderDraft({ ...args, savedConfig: saved, slug: saved.slug });
    act(() => result.current.setLocalConfig(dashboard({ name: 'Unsaved title' })));
    act(() => result.current.setSaveError('Save failed'));
    expect(result.current.isDirty).toBe(true);

    rerender({ ...args, savedConfig: next, slug: next.slug });

    expect(result.current).toMatchObject({
      activeConfig: next,
      localConfig: next,
      isEditMode: true,
      isDirty: false,
      saveError: null,
    });

    rerender({ ...args, savedConfig: saved, slug: saved.slug });
    expect(result.current.activeConfig).toEqual(saved);
    expect(result.current.isDirty).toBe(false);
    expect(saved.name).toBe('Battery');
    expect(next.name).toBe('Another dashboard');
  });

  it('keeps a dirty draft through a saved-data refresh and uses the refresh after discard', () => {
    const saved = dashboard();
    const draft = dashboard({ name: 'Local title' });
    const refreshed = dashboard({ name: 'Server title', description: 'Server revision' });
    const originals = structuredClone({ saved, draft, refreshed });
    const { result, rerender } = renderDraft({ savedConfig: saved, slug: saved.slug });
    act(() => result.current.enterEdit());
    act(() => result.current.setLocalConfig(draft));
    act(() => result.current.setSaveError('Save failed'));

    rerender({ savedConfig: refreshed, slug: refreshed.slug });

    expect(result.current.activeConfig).toEqual(draft);
    expect(result.current.localConfig).toEqual(draft);
    expect(result.current.isEditMode).toBe(true);
    expect(result.current.isDirty).toBe(true);
    expect(result.current.saveError).toBe('Save failed');

    act(() => result.current.exitEdit());
    expect(result.current.activeConfig).toEqual(refreshed);
    expect(result.current.isDirty).toBe(false);
    act(() => result.current.enterEdit());
    expect(result.current.localConfig).toEqual(refreshed);
    expect(result.current.isDirty).toBe(false);
    expect({ saved, draft, refreshed }).toEqual(originals);
  });

  it.each(['uncontrolled', 'controlled'] as const)(
    'follows saved refreshes without inventing changes in a pristine %s draft',
    (mode) => {
      const saved = dashboard();
      const refreshed = dashboard({
        name: 'Updated battery',
        widgets: saved.widgets.map(widget => ({
          ...widget,
          layout: { ...widget.layout, y: 4 },
        })),
      });
      const args = {
        slug: saved.slug,
        controlledEditMode: mode === 'controlled' ? true : undefined,
      };
      const { result, rerender } = renderDraft({ ...args, savedConfig: saved });
      if (mode === 'uncontrolled') act(() => result.current.enterEdit());
      expect(result.current.isDirty).toBe(false);

      rerender({ ...args, savedConfig: refreshed });

      expect(result.current.activeConfig).toEqual(refreshed);
      expect(result.current.localConfig).toEqual(refreshed);
      expect(result.current.isEditMode).toBe(true);
      expect(result.current.isDirty).toBe(false);
      expect(saved.name).toBe('Battery');
      expect(saved.widgets[0]?.layout.y).toBe(0);
    },
  );

  it('follows a refresh after local edits have been reverted', () => {
    const saved = dashboard();
    const refreshed = dashboard({ name: 'Server revision' });
    const { result, rerender } = renderDraft({ savedConfig: saved, slug: saved.slug });
    act(() => result.current.enterEdit());
    act(() => result.current.setLocalConfig(dashboard({ name: 'Temporary title' })));
    expect(result.current.isDirty).toBe(true);
    act(() => result.current.setLocalConfig(structuredClone(saved)));
    expect(result.current.isDirty).toBe(false);

    rerender({ savedConfig: refreshed, slug: refreshed.slug });

    expect(result.current.activeConfig).toEqual(refreshed);
    expect(result.current.isDirty).toBe(false);
  });

  it('keeps an explicitly cleared draft empty when saved data refreshes', () => {
    const saved = dashboard();
    const refreshed = dashboard({ name: 'Updated battery' });
    const { result, rerender } = renderDraft({ savedConfig: saved, slug: saved.slug });
    act(() => result.current.enterEdit());
    act(() => result.current.setLocalConfig(null));

    rerender({ savedConfig: refreshed, slug: refreshed.slug });

    expect(result.current.localConfig).toBeNull();
    expect(result.current.activeConfig).toEqual(refreshed);
    expect(result.current.isDirty).toBe(false);
    act(() => result.current.enterEdit());
    expect(result.current.localConfig).toEqual(refreshed);
  });

  it('becomes clean when refreshed saved data matches the existing draft', () => {
    const saved = dashboard();
    const draft = dashboard({ name: 'Saved title' });
    const { result, rerender } = renderDraft({ savedConfig: saved, slug: saved.slug });
    act(() => result.current.enterEdit());
    act(() => result.current.setLocalConfig(draft));
    expect(result.current.isDirty).toBe(true);

    rerender({ savedConfig: structuredClone(draft), slug: draft.slug });

    expect(result.current.activeConfig).toEqual(draft);
    expect(result.current.isEditMode).toBe(true);
    expect(result.current.isDirty).toBe(false);

    const refreshed = dashboard({ name: 'Later server update' });
    rerender({ savedConfig: refreshed, slug: refreshed.slug });
    expect(result.current.activeConfig).toEqual(refreshed);
    expect(result.current.isDirty).toBe(false);
  });

  it('applies queued functional edits and error updates to the latest session state', () => {
    const saved = dashboard();
    const { result } = renderDraft({ savedConfig: saved, slug: saved.slug });
    act(() => result.current.enterEdit());
    act(() => {
      result.current.setLocalConfig(current => current && ({ ...current, name: 'New title' }));
      result.current.setLocalConfig(current => current && ({
        ...current,
        description: `${current.name} details`,
      }));
      result.current.setSaveError('Save failed');
      result.current.setSaveError(error => `${error}; retry`);
    });
    expect(result.current.localConfig).toMatchObject({
      name: 'New title',
      description: 'New title details',
    });
    expect(result.current.saveError).toBe('Save failed; retry');
    expect(result.current.isDirty).toBe(true);
  });

  it('accepts equivalent saved objects recreated during each render without resetting edits', () => {
    const saved = dashboard();
    const { result, rerender } = renderHook(() => useDashboardEditDraft({
      savedConfig: structuredClone(saved),
      slug: saved.slug,
    }), { wrapper: StrictMode });
    act(() => result.current.enterEdit());
    expect(result.current.isDirty).toBe(false);
    act(() => result.current.setLocalConfig(dashboard({ name: 'Local title' })));

    rerender();

    expect(result.current.activeConfig?.name).toBe('Local title');
    expect(result.current.isDirty).toBe(true);
  });

  it('never commits the previous dashboard draft to children after an identity switch', () => {
    const saved = dashboard();
    const next = dashboard({ id: '10000000-0000-4000-8000-000000000002', name: 'Second copy' });
    const commits: Array<string | undefined> = [];
    const { result, rerender } = renderHook((props: UseDashboardEditDraftArgs) => {
      const draft = useDashboardEditDraft(props);
      useLayoutEffect(() => { commits.push(draft.activeConfig?.id); });
      return draft;
    }, {
      initialProps: { savedConfig: saved, slug: saved.slug, controlledEditMode: true },
      wrapper: StrictMode,
    });
    act(() => result.current.setLocalConfig(dashboard({ name: 'Unsaved title' })));
    commits.length = 0;

    rerender({ savedConfig: next, slug: next.slug, controlledEditMode: true });

    expect(commits.length).toBeGreaterThan(0);
    expect(commits.every(id => id === next.id)).toBe(true);
    expect(result.current.isDirty).toBe(false);
  });
});
