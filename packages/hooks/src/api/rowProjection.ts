export type Field<T> = (row: Record<string, unknown>, index: number) => T;
const absent = Symbol('absent-field');
type Spec = Record<string, Field<unknown>>;
type Output<S extends Spec> = { [K in keyof S]: Exclude<ReturnType<S[K]>, typeof absent> };

export function finiteValue(value: unknown): number | undefined {
  if (typeof value !== 'number' && (typeof value !== 'string' || value.trim().length === 0)) return undefined;
  const converted = Number(value);
  return Number.isFinite(converted) ? converted : undefined;
}

export function numeric<T extends number | null>(fallback: T, ...keys: string[]): Field<number | T> {
  return (row) => {
    const candidate = keys.map((key) => finiteValue(row[key])).find((value) => value !== undefined);
    return candidate ?? fallback;
  };
}

export function textual<T extends string | null>(fallback: T | (() => T), ...keys: string[]): Field<string | T> {
  return (row) => {
    for (const key of keys) if (typeof row[key] === 'string') return row[key];
    return typeof fallback === 'function' ? fallback() : fallback;
  };
}

export function optionalText(key: string, allowed?: readonly string[]): Field<string | typeof absent> {
  return (row) => typeof row[key] === 'string' && (!allowed || allowed.includes(row[key])) ? row[key] : absent;
}

export function project<S extends Spec>(schema: S) {
  const entries = Object.entries(schema);
  return (row: Record<string, unknown>, index = 0): Output<S> => {
    const values = entries.map(([key, decode]) => [key, decode(row, index)] as const);
    return Object.fromEntries(values.filter(([, value]) => value !== absent)) as Output<S>;
  };
}
