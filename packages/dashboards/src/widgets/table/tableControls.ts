export const TABLE_PAGE_SIZES = [15, 25, 50, 100] as const;
export const TABLE_PAGE_OPTIONS = TABLE_PAGE_SIZES.map((size) => ({
  label: String(size),
  value: String(size),
}));

export function pageNavigation(page: number, response?: { total: number; per_page: number }) {
  const pages = response ? Math.ceil(response.total / response.per_page) : 1;
  return {
    pages: Math.max(pages, 1),
    total: response?.total ?? 0,
    previous: { page: page - 1, disabled: page <= 1 },
    next: { page: page + 1, disabled: page >= pages || pages <= 1 },
  };
}

export function tripBatchActions(
  selectedCount: number,
  staged: string[],
  common: string[],
  pending: 'clear' | 'add' | 'save' | null,
  updating: boolean
) {
  const existing = new Set(common);
  const additions = staged.filter((id) => !existing.has(id));
  const definitions = [
    { key: 'clear', verb: 'Clear tags from', mobile: 'Clear', tooltip: 'Clear all tags from selected trips', variant: 'danger', mode: 'replace', tags: [] },
    { key: 'add', verb: 'Add staged tags to', mobile: 'Add', tooltip: 'Add staged tags without overwriting existing tags', variant: 'secondary', mode: 'add', tags: additions },
    { key: 'save', verb: 'Replace tags on', mobile: 'Replace', tooltip: 'Replace all selected-trip tags with the staged tags', variant: 'primary', mode: 'replace', tags: staged },
  ] as const;
  return definitions.map((action) => ({
    ...action,
    tags: [...action.tags],
    label: `${action.verb} ${selectedCount} selected trips`,
    disabled: pending !== null || updating || (action.key !== 'clear' && action.tags.length === 0),
    loading: pending === action.key,
  }));
}
