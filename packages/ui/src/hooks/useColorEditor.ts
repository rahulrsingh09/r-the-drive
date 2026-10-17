import * as React from 'react';
import type { ThemeColorPair } from '@panda/themes';
import { canonicalColor, hexToRgb, hslToRgb, oklchToHex, rgbToHex, rgbToHsl, rgbToOklch } from '../lib/color';

type Mode = 'light' | 'dark';
type Format = 'hex' | 'rgb' | 'hsl' | 'oklch';
type State = { draft: ThemeColorPair; mode: Mode; linked: boolean; format: Format; input: string; error: string | null };
type Action =
  | { type: 'reset'; pair: ThemeColorPair }
  | { type: 'color'; color: string }
  | { type: 'mode'; mode: Mode }
  | { type: 'format'; format: Format }
  | { type: 'input'; input: string }
  | { type: 'link'; linked: boolean }
  | { type: 'copy'; source: Mode }
  | { type: 'invalid' };
function canonicalPair(pair: ThemeColorPair) {
  return { light: canonicalColor(pair.light), dark: canonicalColor(pair.dark) };
}
function transition(state: State, action: Action): State {
  switch (action.type) {
    case 'reset': {
      const draft = canonicalPair(action.pair);
      return { ...state, draft, linked: draft.light === draft.dark, error: null };
    }
    case 'color': {
      const color = canonicalColor(action.color);
      const draft = state.linked ? { light: color, dark: color } : { ...state.draft, [state.mode]: color };
      return { ...state, draft, error: null };
    }
    case 'mode': return { ...state, mode: action.mode };
    case 'format': return { ...state, format: action.format };
    case 'input': return { ...state, input: action.input };
    case 'link': return { ...state, linked: action.linked, draft: action.linked ? { light: state.draft[state.mode], dark: state.draft[state.mode] } : state.draft };
    case 'copy': return { ...state, draft: { light: state.draft[action.source], dark: state.draft[action.source] } };
    case 'invalid': return { ...state, error: `Enter a valid ${state.format.toUpperCase()} color.` };
  }
}
function parseColorInput(format: Format, input: string): string | null {
  if (format === 'hex') return /^#[0-9a-f]{3,6}$/i.test(input.trim()) ? canonicalColor(input) : null;
  const values = input.split(/[\s,]+/).filter(Boolean).map(Number);
  if (values.length !== 3 || values.some(value => !Number.isFinite(value))) return null;
  const [first, second, third] = values as [number, number, number];
  switch (format) {
    case 'rgb': return rgbToHex({ r: first, g: second, b: third });
    case 'hsl': return rgbToHex(hslToRgb({ h: first, s: second, l: third }));
    case 'oklch': return oklchToHex({ l: first, c: second, h: third });
  }
}
export function useColorEditor(open: boolean, value: ThemeColorPair) {
  const [state, send] = React.useReducer(transition, value, (pair): State => ({ draft: canonicalPair(pair), mode: 'dark', linked: pair.light === pair.dark, format: 'hex', input: '', error: null }));
  React.useEffect(() => { if (open) send({ type: 'reset', pair: { light: value.light, dark: value.dark } }); }, [open, value.light, value.dark]);
  const active = state.draft[state.mode];
  const rgb = hexToRgb(active)!;
  const hsl = rgbToHsl(rgb), oklch = rgbToOklch(rgb);
  const formatted = { hex: active, rgb: [rgb.r, rgb.g, rgb.b].map(Math.round).join(', '), hsl: [hsl.h, hsl.s, hsl.l].join(', '), oklch: [oklch.l, oklch.c, oklch.h].join(', ') }[state.format];
  React.useEffect(() => { send({ type: 'input', input: formatted }); }, [active, state.format, formatted]);
  function parseInput() {
    const color = parseColorInput(state.format, state.input);
    send(color ? { type: 'color', color } : { type: 'invalid' });
  }
  return { state, send, active, oklch, parseInput };
}
