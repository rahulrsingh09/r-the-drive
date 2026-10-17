export function statusLabel(value: string, separators: RegExp, prefix?: string) {
  const input = prefix && value.startsWith(prefix) ? value.slice(prefix.length) : value;
  const spaced = input.split(separators).join(' ');
  const boundaries = [...spaced.matchAll(/\b\w/g)];
  let result = spaced;
  for (const match of boundaries) {
    const index = match.index!;
    result = result.slice(0, index) + match[0].toUpperCase() + result.slice(index + 1);
  }
  return result;
}
