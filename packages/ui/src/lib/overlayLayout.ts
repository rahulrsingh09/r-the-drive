export function nextEnabledOption(enabled: readonly number[], count: number, start: number, direction: 1 | -1): number {
  if (!enabled.length) return -1;
  const origin = start < 0 ? count - 1 : start >= count ? 0 : start;
  const sequence = direction === 1 ? enabled : [...enabled].reverse();
  return sequence.find(index => direction === 1 ? index >= origin : index <= origin) ?? sequence[0]!;
}
const limit = (value: number, low: number, high: number) => Math.min(Math.max(value, low), Math.max(low, high));
export function floatingPosition(trigger: DOMRect, content: DOMRect, side: 'top' | 'bottom', align: 'start' | 'center' | 'end', flipAboveViewport = true) {
  const padding = 8, gap = 8;
  const candidates = { top: trigger.top - content.height - gap, bottom: trigger.bottom + gap };
  const preferred = candidates[side];
  const outside = (flipAboveViewport && preferred < padding) || preferred + content.height > window.innerHeight - padding;
  const top = outside ? candidates[side === 'top' ? 'bottom' : 'top'] : preferred;
  const positions = { start: trigger.left, center: trigger.left + trigger.width / 2 - content.width / 2, end: trigger.right - content.width };
  return { top: limit(top, padding, window.innerHeight - padding - content.height), left: limit(positions[align], padding, window.innerWidth - padding - content.width), visibility: 'visible' as const };
}
export function selectPosition(trigger: DOMRect, naturalWidth: number, scrollHeight: number, align: 'left' | 'right', small: boolean) {
  const viewport = window.visualViewport;
  const bounds = { left: (viewport?.offsetLeft ?? 0) + 12, top: (viewport?.offsetTop ?? 0) + 12, width: (viewport?.width ?? window.innerWidth) - 24, height: (viewport?.height ?? window.innerHeight) - 24 };
  const width = Math.min(Math.max(trigger.width, naturalWidth), bounds.width);
  const gap = small ? 4 : 6;
  const space = { down: bounds.top + bounds.height - trigger.bottom - gap, up: trigger.top - bounds.top - gap };
  const down = space.down >= Math.min(288, scrollHeight) || space.down >= space.up;
  const maxHeight = Math.max(32, Math.min(288, down ? space.down : space.up));
  return {
    left: Math.max(bounds.left, Math.min(align === 'right' ? trigger.right - width : trigger.left, bounds.left + bounds.width - width)),
    top: Math.max(bounds.top, down ? trigger.bottom + gap : trigger.top - gap - Math.min(scrollHeight, maxHeight)),
    width, maxHeight,
  };
}

export function rovingIndex(key: string, current: number, count: number): number | null {
  const commands: Record<string, readonly [base: number, delta: number]> = {
    Home: [0, 0], End: [count - 1, 0],
    ArrowLeft: [current, -1], ArrowUp: [current, -1],
    ArrowRight: [current, 1], ArrowDown: [current, 1],
  };
  const command = commands[key];
  return command ? (command[0] + command[1] + count) % count : null;
}
