export interface RgbColor { r: number; g: number; b: number }
export interface HslColor { h: number; s: number; l: number }
export interface OklchColor { l: number; c: number; h: number }

type Triple = readonly [number, number, number];
type Matrix = readonly [Triple, Triple, Triple];
const clamp = (value: number, min = 0, max = 1) => Math.min(max, Math.max(min, value));
const round = (value: number, digits = 3) => Number(value.toFixed(digits));
const wrapHue = (hue: number) => ((hue % 360) + 360) % 360;
const rgbVector = ({ r, g, b }: RgbColor): Triple => [r, g, b];
const fromVector = ([r, g, b]: Triple): RgbColor => ({ r, g, b });
function transform(vector: Triple, matrix: Matrix): Triple {
  return matrix.map(row => row[0] * vector[0] + row[1] * vector[1] + row[2] * vector[2]) as unknown as Triple;
}
const RGB_TO_LMS: Matrix = [
  [0.4122214708, 0.5363325363, 0.0514459929],
  [0.2119034982, 0.6806995451, 0.1073969566],
  [0.0883024619, 0.2817188376, 0.6299787005],
];
const LMS_TO_LAB: Matrix = [
  [0.2104542553, 0.793617785, -0.0040720468],
  [1.9779984951, -2.428592205, 0.4505937099],
  [0.0259040371, 0.7827717662, -0.808675766],
];
const LAB_TO_LMS: Matrix = [
  [1, 0.3963377774, 0.2158037573],
  [1, -0.1055613458, -0.0638541728],
  [1, -0.0894841775, -1.291485548],
];
const LMS_TO_RGB: Matrix = [
  [4.0767416621, -3.3077115913, 0.2309699292],
  [-1.2684380046, 2.6097574011, -0.3413193965],
  [-0.0041960863, -0.7034186147, 1.707614701],
];
function decodeHex(value: string): RgbColor | null {
  const text = value.trim();
  if (!/^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i.test(text)) return null;
  const packed = Number.parseInt(text.slice(1), 16);
  const short = text.length === 4;
  const mask = short ? 15 : 255;
  const bits = short ? 4 : 8;
  const factor = short ? 17 : 1;
  return { r: ((packed >> (bits * 2)) & mask) * factor, g: ((packed >> bits) & mask) * factor, b: (packed & mask) * factor };
}
export function normalizeHex(value: string): `#${string}` | null {
  const rgb = decodeHex(value);
  return rgb ? rgbToHex(rgb) : null;
}
export function hexToRgb(value: string): RgbColor | null {
  return decodeHex(value);
}
export function rgbToHex(rgb: RgbColor): `#${string}` {
  const encoded = rgbVector(rgb).map(value => Math.round(clamp(value, 0, 255)).toString(16).padStart(2, '0'));
  return `#${encoded.join('')}`;
}
const CSS_CHANNELS = {
  rgb: /^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)(?:\s*[,/]\s*[\d.]+%?)?\s*\)$/i,
  hsl: /^hsla?\(\s*([\d.-]+)(?:deg)?[,\s]+([\d.]+)%[,\s]+([\d.]+)%(?:\s*[,/]\s*[\d.]+%?)?\s*\)$/i,
};
export function parseCssColor(value: string): RgbColor | null {
  const text = value.trim();
  if (text.startsWith('#')) return decodeHex(text);
  const kind = text.slice(0, 3).toLowerCase();
  if (kind !== 'rgb' && kind !== 'hsl') return null;
  const match = CSS_CHANNELS[kind].exec(text);
  if (!match) return null;
  const channels = match.slice(1, 4).map(Number) as [number, number, number];
  return kind === 'hsl'
    ? hslToRgb({ h: channels[0], s: channels[1], l: channels[2] })
    : fromVector(channels.map(channel => clamp(channel, 0, 255)) as [number, number, number]);
}
export function rgbToHsl(rgb: RgbColor): HslColor {
  const channels = rgbVector(rgb).map(value => value / 255) as [number, number, number];
  const high = Math.max(...channels), low = Math.min(...channels);
  const delta = high - low, lightness = (high + low) / 2;
  const dominant = channels.indexOf(high);
  const differences = [channels[1] - channels[2], channels[2] - channels[0], channels[0] - channels[1]];
  const sector = delta ? differences[dominant]! / delta + dominant * 2 : 0;
  const rawHue = 60 * (dominant === 0 ? sector % 6 : sector);
  const hue = rawHue < 0 ? rawHue + 360 : rawHue;
  return { h: round(hue, 1), s: round((delta === 0 ? 0 : delta / (1 - Math.abs(2 * lightness - 1))) * 100, 1), l: round(lightness * 100, 1) };
}
export function hslToRgb({ h, s, l }: HslColor): RgbColor {
  const hue = wrapHue(h), lightness = clamp(l / 100);
  const chroma = (1 - Math.abs(2 * lightness - 1)) * clamp(s / 100);
  const x = chroma * (1 - Math.abs(((hue / 60) % 2) - 1));
  const offset = lightness - chroma / 2;
  const sectors: Triple[] = [[chroma, x, 0], [x, chroma, 0], [0, chroma, x], [0, x, chroma], [x, 0, chroma], [chroma, 0, x]];
  const channels = sectors[Math.floor(hue / 60)] ?? sectors[5]!;
  return fromVector(channels.map(value => (value + offset) * 255) as [number, number, number]);
}
const srgbToLinear = (value: number) => {
  const channel = value / 255;
  return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
};
const linearToSrgb = (value: number) => 255 * (value <= 0.0031308 ? 12.92 * value : 1.055 * value ** (1 / 2.4) - 0.055);
export function rgbToOklch(rgb: RgbColor): OklchColor {
  const linear = rgbVector(rgb).map(srgbToLinear) as [number, number, number];
  const roots = transform(linear, RGB_TO_LMS).map(Math.cbrt) as [number, number, number];
  const [lightness, a, b] = transform(roots, LMS_TO_LAB);
  const hue = Math.atan2(b, a) * 180 / Math.PI;
  return { l: round(lightness * 100, 2), c: round(Math.sqrt(a * a + b * b), 4), h: round(hue < 0 ? hue + 360 : hue, 2) };
}
function rawOklchToRgb({ l, c, h }: OklchColor): RgbColor {
  const radians = h * Math.PI / 180;
  const lab: Triple = [l / 100, c * Math.cos(radians), c * Math.sin(radians)];
  const cubes = transform(lab, LAB_TO_LMS).map(value => value ** 3) as [number, number, number];
  return fromVector(transform(cubes, LMS_TO_RGB).map(linearToSrgb) as [number, number, number]);
}
function inGamut(rgb: RgbColor) {
  return rgb.r >= 0 && rgb.r <= 255 && rgb.g >= 0 && rgb.g <= 255 && rgb.b >= 0 && rgb.b <= 255;
}
export function oklchToHex(value: OklchColor): `#${string}` {
  const target = { l: clamp(value.l, 0, 100), c: Math.max(0, value.c), h: wrapHue(value.h) };
  let accepted = rawOklchToRgb(target);
  if (inGamut(accepted)) return rgbToHex(accepted);
  let minimum = 0, maximum = target.c;
  // The 18-step chroma boundary is part of saved-color compatibility.
  for (let remaining = 18; remaining > 0; remaining -= 1) {
    const middle = (minimum + maximum) / 2;
    const candidate = rawOklchToRgb({ ...target, c: middle });
    if (inGamut(candidate)) { minimum = middle; accepted = candidate; }
    else maximum = middle;
  }
  return rgbToHex(accepted);
}
export function canonicalColor(value: string, fallback = '#fd8304'): `#${string}` {
  return rgbToHex(parseCssColor(value) ?? decodeHex(fallback)!);
}
export function contrastRatio(first: string, second: string): number {
  const luminances = [first, second].map(value => {
    const channels = rgbVector(parseCssColor(value) ?? { r: 0, g: 0, b: 0 }).map(srgbToLinear);
    return channels[0]! * 0.2126 + channels[1]! * 0.7152 + channels[2]! * 0.0722;
  }).sort((a, b) => a - b);
  return (luminances[1]! + 0.05) / (luminances[0]! + 0.05);
}
