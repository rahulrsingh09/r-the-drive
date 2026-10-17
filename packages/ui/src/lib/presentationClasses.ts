type ClassChoice = readonly [value: string | boolean, variants: Readonly<Record<string, string>>];

/** Select class fragments in declaration order before Tailwind conflict resolution. */
export function presentationClasses(choices: readonly ClassChoice[]): string[] {
  const selected: string[] = [];
  for (const [value, variants] of choices) {
    const key = String(value);
    if (Object.hasOwn(variants, key)) selected.push(variants[key]!);
  }
  return selected;
}
