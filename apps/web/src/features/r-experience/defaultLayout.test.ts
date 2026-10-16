import { describe, expect, it } from 'vitest';
import { getDefaultBySlug } from '@panda/dashboards';
import { usesBundledLayout } from './defaultLayout';

describe('R bundled layout admission', () => {
  it('compacts untouched analytics but preserves personal and saved system geometry', () => {
    const original = getDefaultBySlug('charging')!;
    expect(usesBundledLayout(original)).toBe(true);
    expect(usesBundledLayout({ ...original, isDefault: false })).toBe(false);
    const edited = structuredClone(original);
    edited.widgets[0]!.layout.h += 2;
    expect(edited.isDefault).toBe(true);
    expect(usesBundledLayout(edited)).toBe(false);
    const moved = structuredClone(original);
    moved.widgets[0]!.layout.y += 2;
    expect(usesBundledLayout(moved)).toBe(false);
    expect(usesBundledLayout(getDefaultBySlug('dashboard'))).toBe(true);
    const overview = structuredClone(getDefaultBySlug('dashboard')!);
    overview.widgets[0]!.layout.h += 1;
    expect(usesBundledLayout(overview)).toBe(false);
  });
});
