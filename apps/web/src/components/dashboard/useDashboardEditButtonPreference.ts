const ignoreLegacyPreference = (_next: boolean) => {};

/** Inert compatibility port until the separately owned settings caller is removed. */
export function useDashboardEditButtonPreference(_userId: string | null | undefined) {
  return [false, ignoreLegacyPreference] as const;
}
