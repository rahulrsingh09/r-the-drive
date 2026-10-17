type CalendarUnit = 'utc-day' | 'month' | 'hour' | 'second';

/** Evaluate a fixed story calendar independently of its telemetry recipe. */
export function storyTimeline(origin: string, count: number, unit: CalendarUnit, stride: number) {
  const start = new Date(origin);
  const startTime = start.getTime();
  const originField = unit === 'month' ? start.getMonth() : unit === 'second' ? start.getSeconds() : 0;
  return Array.from({ length: count }, (_, index) => {
    if (unit === 'utc-day') return new Date(startTime + index * stride * 86400000).toISOString();
    const date = new Date(startTime);
    const offset = originField + index * stride;
    switch (unit) {
      case 'month': date.setMonth(offset); break;
      case 'hour': date.setHours(offset); break;
      case 'second': date.setSeconds(offset); break;
    }
    return date.toISOString();
  });
}
