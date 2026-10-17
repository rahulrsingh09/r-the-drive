/** Only declared option keys participate; nullish values fall through to the next default. */
export function resolveWidgetOptions<T extends object>(defaults: T, ...layers: object[]): T {
  const result = { ...defaults };
  for (const key of Object.keys(defaults) as Array<keyof T>) {
    for (let index = layers.length - 1; index >= 0; index--) {
      const value = (layers[index] as Partial<T>)[key];
      if (value != null) {
        result[key] = value as T[keyof T];
        break;
      }
    }
  }
  return result;
}
