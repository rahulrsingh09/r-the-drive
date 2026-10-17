export function scatterBuckets(values: Array<number | null>, count: number) {
  const finite = values.filter((value): value is number => value !== null && Number.isFinite(value));
  const extent = finite.reduce<[number, number]>(
    ([low, high], value) => [Math.min(low, value), Math.max(high, value)],
    [Infinity, -Infinity]
  );
  const [low, high] = extent as [number, number];
  const groups = Array.from({ length: count }, () => [] as number[]);
  values.forEach((value, index) => {
    if (value === null || !Number.isFinite(value)) return;
    const bucket = high > low ? Math.min(count - 1, Math.floor(((value - low) / (high - low)) * count)) : count - 1;
    groups[bucket]!.push(index);
  });
  return groups;
}

export function scatterPalette(start: string, end: string, count: number) {
  const channels = [1, 3, 5].map((offset) => ({
    from: Number.parseInt(start.slice(offset, offset + 2), 16),
    to: Number.parseInt(end.slice(offset, offset + 2), 16),
  }));
  return Array.from({ length: count }, (_, index) => {
    const ratio = index / (count - 1);
    return '#' + channels.map(({ from, to }) =>
      Math.round(from + (to - from) * ratio).toString(16).padStart(2, '0')
    ).join('');
  });
}
