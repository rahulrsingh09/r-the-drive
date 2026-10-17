import type { BrandAssetKind } from './internal-types';
import type { ChartColorToken, ThemePalette } from '@panda/types';

export type { BrandAssetKind };
export type SemanticToken = keyof typeof SEMANTIC_TOKEN_CATALOG;
/** Serialized color/effect value. Runtime validation restricts persisted overrides. */
export type ThemeColor = string;
export interface ThemeColorPair { light: ThemeColor; dark: ThemeColor; }
export interface BrandPaintSlots { accent: ThemeColorPair; accentMuted: ThemeColorPair; mark: ThemeColorPair; }

export interface ThemeDefinition {
  schemaVersion: 1;
  id: string;
  legacyPalette: ThemePalette;
  name: string;
  tokens: { light: Record<SemanticToken, ThemeColor>; dark: Record<SemanticToken, ThemeColor> };
  series: Record<Extract<ChartColorToken, `series-${string}`>, ThemeColorPair>;
  chartAliases: Record<string, ThemeColorPair | ChartColorToken>;
  brandAssets: Record<BrandAssetKind, { light: string; dark: string }>;
  brandPaints: BrandPaintSlots;
}

export interface ThemeOverride {
  theme?: ThemePalette;
  tokens?: Partial<Record<SemanticToken, Partial<ThemeColorPair>>>;
  series?: Partial<Record<Extract<ChartColorToken, `series-${string}`>, Partial<ThemeColorPair>>>;
  brandPaints?: Partial<{ [K in keyof BrandPaintSlots]: Partial<ThemeColorPair> }>;
}

export interface ResolvedTheme extends ThemeDefinition { sourceTheme: ThemePalette; }

export const SEMANTIC_TOKEN_CATALOG = {
  'bg-page': 0, 'bg-surface': 0, 'bg-elevated': 0, 'bg-glass': 0, 'bg-overlay': 0,
  'text-primary': 0, 'text-secondary': 0, 'text-tertiary': 0, 'text-disabled': 0, 'text-on-accent': 0,
  'border-default': 0, 'border-strong': 0, 'border-accent': 0,
  'status-positive': 0, 'status-warning': 0, 'status-danger': 0, 'status-info': 0,
  'charging-active': 0, 'charging-done': 0, 'charging-limited': 0, 'charging-ac': 0, 'charging-dc': 0, 'charging-dcfc': 0,
  'dm-everyday': 0, 'dm-conserve': 0, 'dm-terrain': 0, 'dm-sand': 0, 'dm-rock': 0, 'dm-rally': 0, 'dm-drift': 0, 'dm-towing': 0, 'dm-unknown': 0,
  'glow-sm': 0, 'glow-md': 0, 'glow-lg': 0, 'glow-button': 0,
  accent: 0, 'accent-hover': 0, 'accent-active': 0, 'accent-muted': 0,
  'shadow-sm': 0, 'shadow-md': 0, 'shadow-lg': 0, 'shadow-xl': 0,
  'chart-grid': 0, 'chart-hover-bg': 0, 'chart-accent': 0, 'chart-success': 0, 'chart-warning': 0, 'chart-danger': 0, 'chart-muted': 0,
  'chart-amber': 0, 'chart-yellow': 0, 'chart-sky': 0, 'chart-emerald': 0, 'chart-violet': 0, 'chart-rose': 0, 'chart-teal': 0, 'chart-orange': 0, 'chart-indigo': 0,
  'map-route-0': 0, 'map-route-1': 0, 'map-route-2': 0, 'map-route-3': 0, 'map-route-4': 0, 'map-route-5': 0,
  'value-halo': 0,
} as const;

export const CLASSIC_THEME: ThemeDefinition = {
  "schemaVersion": 1,
  "id": "classic",
  "legacyPalette": "classic",
  "name": "Classic",
  "tokens": {
    "light": {
      "bg-page": "#FAFAF7",
      "bg-surface": "#FFFFFF",
      "bg-elevated": "#F4F4EE",
      "bg-glass": "rgba(255, 255, 255, 0.7)",
      "bg-overlay": "rgba(20, 20, 24, 0.4)",
      "text-primary": "#18181B",
      "text-secondary": "#52525B",
      "text-tertiary": "#71717A",
      "text-disabled": "#A1A1A1",
      "text-on-accent": "#FAFAFA",
      "border-default": "rgba(20, 20, 24, 0.08)",
      "border-strong": "rgba(20, 20, 24, 0.18)",
      "border-accent": "rgba(253, 131, 4, 0.35)",
      "accent": "#FD8304",
      "accent-hover": "#EA580C",
      "accent-active": "#C2410C",
      "accent-muted": "rgba(253, 131, 4, 0.10)",
      "status-positive": "#059669",
      "status-warning": "#D97706",
      "status-danger": "#EF4444",
      "status-info": "#3B82F6",
      "chart-accent": "#C96F08",
      "chart-success": "#047857",
      "chart-warning": "#B45309",
      "chart-danger": "#DC2626",
      "chart-muted": "#71717A",
      "chart-amber": "#C96F08",
      "chart-yellow": "#A16207",
      "chart-sky": "#2563EB",
      "chart-emerald": "#047857",
      "chart-violet": "#7C3AED",
      "chart-rose": "#DC2626",
      "chart-teal": "#047857",
      "chart-orange": "#C2410C",
      "chart-indigo": "#4F46E5",
      "chart-grid": "rgba(20, 20, 24, 0.08)",
      "chart-hover-bg": "rgba(20, 20, 24, 0.03)",
      "charging-active": "#FD8304",
      "charging-done": "#059669",
      "charging-limited": "#EF4444",
      "charging-ac": "#3B82F6",
      "charging-dc": "#A78BFA",
      "charging-dcfc": "#C084FC",
      "dm-everyday": "#059669",
      "dm-conserve": "#3B82F6",
      "dm-terrain": "#22D3EE",
      "dm-sand": "#FBBF24",
      "dm-rock": "#A78BFA",
      "dm-rally": "#E879F9",
      "dm-drift": "#FB7185",
      "dm-towing": "#FB923C",
      "map-route-0": "#38BDF8",
      "map-route-1": "#34D399",
      "map-route-2": "#A78BFA",
      "map-route-3": "#F472B6",
      "map-route-4": "#F59E0B",
      "map-route-5": "#F87171",
      "dm-unknown": "#6B7C7C",
      "glow-sm": "0 0 0 1px rgba(253, 131, 4, 0.10)",
      "glow-md": "0 1px 2px rgba(253, 131, 4, 0.15)",
      "glow-lg": "0 2px 8px rgba(253, 131, 4, 0.20)",
      "glow-button": "0 4px 12px rgba(253, 131, 4, 0.25)",
      "shadow-sm": "0 1px 2px rgba(20, 20, 24, 0.06)",
      "shadow-md": "0 2px 6px rgba(20, 20, 24, 0.08)",
      "shadow-lg": "0 8px 20px rgba(20, 20, 24, 0.10)",
      "shadow-xl": "0 16px 32px rgba(20, 20, 24, 0.12)",
      "value-halo": "-2px -2px 0 #FFFFFF, 0 -2px 0 #FFFFFF, 2px -2px 0 #FFFFFF, 2px 0 0 #FFFFFF, 2px 2px 0 #FFFFFF, 0 2px 0 #FFFFFF, -2px 2px 0 #FFFFFF, -2px 0 0 #FFFFFF"
    },
    "dark": {
      "bg-page": "#0A0A0F",
      "bg-surface": "#12121A",
      "bg-elevated": "#1A1A24",
      "bg-glass": "rgba(26, 26, 36, 0.6)",
      "bg-overlay": "rgba(0, 0, 0, 0.7)",
      "text-primary": "#FAFAFA",
      "text-secondary": "#A1A1A1",
      "text-tertiary": "#71717A",
      "text-disabled": "#52525B",
      "text-on-accent": "#0A0A0F",
      "border-default": "rgba(255, 255, 255, 0.08)",
      "border-strong": "rgba(255, 255, 255, 0.15)",
      "border-accent": "rgba(253, 131, 4, 0.3)",
      "accent": "#FD8304",
      "accent-hover": "#FDBA74",
      "accent-active": "#EA580C",
      "accent-muted": "rgba(253, 131, 4, 0.15)",
      "status-positive": "#10B981",
      "status-warning": "#F59E0B",
      "status-danger": "#F87171",
      "status-info": "#60A5FA",
      "chart-accent": "#FD8304",
      "chart-success": "#10B981",
      "chart-warning": "#F59E0B",
      "chart-danger": "#F87171",
      "chart-muted": "#A1A1A1",
      "chart-amber": "#FD8304",
      "chart-yellow": "#FACC15",
      "chart-sky": "#60A5FA",
      "chart-emerald": "#10B981",
      "chart-violet": "#A78BFA",
      "chart-rose": "#F87171",
      "chart-teal": "#34D399",
      "chart-orange": "#FB923C",
      "chart-indigo": "#818CF8",
      "chart-grid": "rgba(255, 255, 255, 0.06)",
      "chart-hover-bg": "rgba(255, 255, 255, 0.02)",
      "charging-active": "#FD8304",
      "charging-done": "#10B981",
      "charging-limited": "#F87171",
      "charging-ac": "#60A5FA",
      "charging-dc": "#A78BFA",
      "charging-dcfc": "#C084FC",
      "dm-everyday": "#10B981",
      "dm-conserve": "#60A5FA",
      "dm-terrain": "#22D3EE",
      "dm-sand": "#FBBF24",
      "dm-rock": "#A78BFA",
      "dm-rally": "#E879F9",
      "dm-drift": "#FB7185",
      "dm-towing": "#FB923C",
      "map-route-0": "#60A5FA",
      "map-route-1": "#34D399",
      "map-route-2": "#A78BFA",
      "map-route-3": "#F472B6",
      "map-route-4": "#F59E0B",
      "map-route-5": "#F87171",
      "dm-unknown": "#7A8B89",
      "glow-sm": "0 0 20px rgba(253, 131, 4, 0.15)",
      "glow-md": "0 0 40px rgba(253, 131, 4, 0.20)",
      "glow-lg": "0 0 60px rgba(253, 131, 4, 0.25)",
      "glow-button": "0 0 20px rgba(253, 131, 4, 0.40)",
      "shadow-sm": "0 1px 2px rgba(0, 0, 0, 0.3)",
      "shadow-md": "0 4px 6px rgba(0, 0, 0, 0.3)",
      "shadow-lg": "0 10px 15px rgba(0, 0, 0, 0.3)",
      "shadow-xl": "0 20px 25px rgba(0, 0, 0, 0.4)",
      "value-halo": "-2px -2px 0 #12121A, 0 -2px 0 #12121A, 2px -2px 0 #12121A, 2px 0 0 #12121A, 2px 2px 0 #12121A, 0 2px 0 #12121A, -2px 2px 0 #12121A, -2px 0 0 #12121A"
    }
  },
  "series": {
    "series-01": {
      "dark": "#FD8304",
      "light": "#C96F08"
    },
    "series-02": {
      "dark": "#FACC15",
      "light": "#A16207"
    },
    "series-03": {
      "dark": "#60A5FA",
      "light": "#2563EB"
    },
    "series-04": {
      "dark": "#10B981",
      "light": "#047857"
    },
    "series-05": {
      "dark": "#A78BFA",
      "light": "#7C3AED"
    },
    "series-06": {
      "dark": "#F87171",
      "light": "#DC2626"
    },
    "series-07": {
      "dark": "#34D399",
      "light": "#047857"
    },
    "series-08": {
      "dark": "#FB923C",
      "light": "#C2410C"
    },
    "series-09": {
      "dark": "#818CF8",
      "light": "#4F46E5"
    },
    "series-10": {
      "dark": "#FD8304",
      "light": "#C96F08"
    },
    "series-11": {
      "dark": "#FACC15",
      "light": "#A16207"
    },
    "series-12": {
      "dark": "#60A5FA",
      "light": "#2563EB"
    },
    "series-13": {
      "dark": "#10B981",
      "light": "#047857"
    },
    "series-14": {
      "dark": "#A78BFA",
      "light": "#7C3AED"
    },
    "series-15": {
      "dark": "#F87171",
      "light": "#DC2626"
    },
    "series-16": {
      "dark": "#34D399",
      "light": "#047857"
    }
  },
  "chartAliases": {
    "accent": {
      "dark": "#FD8304",
      "light": "#FD8304"
    },
    "success": {
      "dark": "#10B981",
      "light": "#047857"
    },
    "warning": {
      "dark": "#F59E0B",
      "light": "#B45309"
    },
    "danger": {
      "dark": "#F87171",
      "light": "#DC2626"
    },
    "muted": {
      "dark": "#A1A1A1",
      "light": "#71717A"
    },
    "emerald": {
      "dark": "#10B981",
      "light": "#047857"
    },
    "amber": {
      "dark": "#FD8304",
      "light": "#FD8304"
    },
    "sky": {
      "dark": "#60A5FA",
      "light": "#2563EB"
    },
    "violet": {
      "dark": "#A78BFA",
      "light": "#7C3AED"
    },
    "rose": {
      "dark": "#F87171",
      "light": "#DC2626"
    },
    "teal": {
      "dark": "#34D399",
      "light": "#047857"
    },
    "indigo": {
      "dark": "#818CF8",
      "light": "#4F46E5"
    },
    "yellow": {
      "dark": "#FACC15",
      "light": "#A16207"
    },
    "orange": {
      "dark": "#FB923C",
      "light": "#C2410C"
    }
  },
  "brandAssets": {
    "wordmark": {
      "dark": "/text_white.svg",
      "light": "/text_black.svg"
    },
    "logo": {
      "dark": "/logo_color_lighter.svg",
      "light": "/logo_color_lighter.svg"
    },
    "icon": {
      "dark": "/icon_color_lighter.svg",
      "light": "/icon_color_lighter.svg"
    },
    "favicon": {
      "dark": "/favicon.svg",
      "light": "/favicon.svg"
    }
  },
  "brandPaints": {
    "accent": {
      "light": "#FD8304",
      "dark": "#FD8304"
    },
    "accentMuted": {
      "light": "rgba(253, 131, 4, 0.10)",
      "dark": "rgba(253, 131, 4, 0.15)"
    },
    "mark": {
      "light": "#FD8304",
      "dark": "#FD8304"
    }
  }
};
export const RAD_THEME: ThemeDefinition = {
  "schemaVersion": 1,
  "id": "rad",
  "legacyPalette": "rad",
  "name": "RAD",
  "tokens": {
    "light": {
      "bg-page": "#FCFCFA",
      "bg-surface": "#FFFFFF",
      "bg-elevated": "#F8FAF9",
      "bg-glass": "rgba(255, 255, 255, 0.88)",
      "bg-overlay": "rgba(18, 59, 67, 0.42)",
      "text-primary": "#123B43",
      "text-secondary": "#355F60",
      "text-tertiary": "#527278",
      "text-disabled": "#A0AEAE",
      "text-on-accent": "#FFFFFF",
      "border-default": "rgba(18, 59, 67, 0.10)",
      "border-strong": "rgba(18, 59, 67, 0.18)",
      "border-accent": "rgba(194, 65, 12, 0.34)",
      "accent": "#C2410C",
      "accent-hover": "#A83B0F",
      "accent-active": "#8E2E0A",
      "accent-muted": "rgba(194, 65, 12, 0.14)",
      "status-positive": "#007A4B",
      "status-warning": "#9C5D00",
      "status-danger": "#D02D30",
      "status-info": "#007DA3",
      "chart-accent": "#F26A2E",
      "chart-success": "#007A4B",
      "chart-warning": "#9C5D00",
      "chart-danger": "#D02D30",
      "chart-muted": "#527278",
      "chart-amber": "#F26A2E",
      "chart-yellow": "#B87900",
      "chart-sky": "#007DA3",
      "chart-emerald": "#007A4B",
      "chart-violet": "#7945D8",
      "chart-rose": "#D02D30",
      "chart-teal": "#007C73",
      "chart-orange": "#DE571E",
      "chart-indigo": "#435BD0",
      "chart-grid": "rgba(18, 59, 67, 0.10)",
      "chart-hover-bg": "rgba(18, 59, 67, 0.035)",
      "charging-active": "#C2410C",
      "charging-done": "#007A4B",
      "charging-limited": "#D02D30",
      "charging-ac": "#007DA3",
      "charging-dc": "#7945D8",
      "charging-dcfc": "#B02A91",
      "dm-everyday": "#007A4B",
      "dm-conserve": "#007DA3",
      "dm-terrain": "#007C73",
      "dm-sand": "#8D6A00",
      "dm-rock": "#7945D8",
      "dm-rally": "#B02A91",
      "dm-drift": "#D02D30",
      "dm-towing": "#B84D00",
      "map-route-0": "#DE571E",
      "map-route-1": "#007DA3",
      "map-route-2": "#B87900",
      "map-route-3": "#7945D8",
      "map-route-4": "#007A4B",
      "map-route-5": "#B02A91",
      "dm-unknown": "#527278",
      "glow-sm": "0 0 0 1px rgba(194, 65, 12, 0.12)",
      "glow-md": "0 1px 2px rgba(194, 65, 12, 0.16)",
      "glow-lg": "0 2px 8px rgba(194, 65, 12, 0.21)",
      "glow-button": "0 4px 12px rgba(194, 65, 12, 0.28)",
      "shadow-sm": "0 1px 2px rgba(18, 59, 67, 0.07)",
      "shadow-md": "0 2px 6px rgba(18, 59, 67, 0.09)",
      "shadow-lg": "0 8px 20px rgba(18, 59, 67, 0.12)",
      "shadow-xl": "0 16px 32px rgba(18, 59, 67, 0.14)",
      "value-halo": "-2px -2px 0 #FFFFFF, 0 -2px 0 #FFFFFF, 2px -2px 0 #FFFFFF, 2px 0 0 #FFFFFF, 2px 2px 0 #FFFFFF, 0 2px 0 #FFFFFF, -2px 2px 0 #FFFFFF, -2px 0 0 #FFFFFF"
    },
    "dark": {
      "bg-page": "#082529",
      "bg-surface": "#10363B",
      "bg-elevated": "#18464B",
      "bg-glass": "rgba(24, 70, 75, 0.68)",
      "bg-overlay": "rgba(7, 13, 16, 0.74)",
      "text-primary": "#FFFFFF",
      "text-secondary": "#C4E3DF",
      "text-tertiary": "#9CBDB9",
      "text-disabled": "#658B88",
      "text-on-accent": "#082529",
      "border-default": "rgba(214, 239, 230, 0.10)",
      "border-strong": "rgba(214, 239, 230, 0.19)",
      "border-accent": "rgba(255, 176, 0, 0.36)",
      "accent": "#FFB000",
      "accent-hover": "#FFD34E",
      "accent-active": "#F59A00",
      "accent-muted": "rgba(255, 176, 0, 0.17)",
      "status-positive": "#38E8A5",
      "status-warning": "#FFC43D",
      "status-danger": "#FF7054",
      "status-info": "#36CEFF",
      "chart-accent": "#FFB000",
      "chart-success": "#38E8A5",
      "chart-warning": "#FFC43D",
      "chart-danger": "#FF7054",
      "chart-muted": "#9AA9AA",
      "chart-amber": "#FFB000",
      "chart-yellow": "#FFD34E",
      "chart-sky": "#36CEFF",
      "chart-emerald": "#38E8A5",
      "chart-violet": "#B59AFF",
      "chart-rose": "#FF7054",
      "chart-teal": "#2AE0CB",
      "chart-orange": "#FF9850",
      "chart-indigo": "#839FFF",
      "chart-grid": "rgba(214, 239, 230, 0.09)",
      "chart-hover-bg": "rgba(214, 239, 230, 0.035)",
      "charging-active": "#FFB000",
      "charging-done": "#38E8A5",
      "charging-limited": "#FF7054",
      "charging-ac": "#36CEFF",
      "charging-dc": "#B59AFF",
      "charging-dcfc": "#F077DA",
      "dm-everyday": "#38E8A5",
      "dm-conserve": "#36CEFF",
      "dm-terrain": "#2AE0CB",
      "dm-sand": "#FFD34E",
      "dm-rock": "#B59AFF",
      "dm-rally": "#F077DA",
      "dm-drift": "#FF7054",
      "dm-towing": "#FF9850",
      "map-route-0": "#FF7054",
      "map-route-1": "#36CEFF",
      "map-route-2": "#FFD34E",
      "map-route-3": "#B59AFF",
      "map-route-4": "#38E8A5",
      "map-route-5": "#F077DA",
      "dm-unknown": "#9CBDB9",
      "glow-sm": "0 0 20px rgba(255, 176, 0, 0.16)",
      "glow-md": "0 0 40px rgba(255, 176, 0, 0.21)",
      "glow-lg": "0 0 60px rgba(255, 176, 0, 0.26)",
      "glow-button": "0 0 20px rgba(255, 176, 0, 0.42)",
      "shadow-sm": "0 1px 2px rgba(0, 0, 0, 0.34)",
      "shadow-md": "0 4px 6px rgba(0, 0, 0, 0.34)",
      "shadow-lg": "0 10px 15px rgba(0, 0, 0, 0.34)",
      "shadow-xl": "0 20px 25px rgba(0, 0, 0, 0.44)",
      "value-halo": "-2px -2px 0 #10363B, 0 -2px 0 #10363B, 2px -2px 0 #10363B, 2px 0 0 #10363B, 2px 2px 0 #10363B, 0 2px 0 #10363B, -2px 2px 0 #10363B, -2px 0 0 #10363B"
    }
  },
  "series": {
    "series-01": {
      "dark": "#FFB000",
      "light": "#FFB000"
    },
    "series-02": {
      "dark": "#FF7054",
      "light": "#F26A2E"
    },
    "series-03": {
      "dark": "#2AE0CB",
      "light": "#008F8C"
    },
    "series-04": {
      "dark": "#36CEFF",
      "light": "#435BD0"
    },
    "series-05": {
      "dark": "#B59AFF",
      "light": "#7945D8"
    },
    "series-06": {
      "dark": "#FF9850",
      "light": "#B84D00"
    },
    "series-07": {
      "dark": "#38E8A5",
      "light": "#00A86B"
    },
    "series-08": {
      "dark": "#FF93A7",
      "light": "#B83461"
    },
    "series-09": {
      "dark": "#839FFF",
      "light": "#007C73"
    },
    "series-10": {
      "dark": "#F4F36B",
      "light": "#697900"
    },
    "series-11": {
      "dark": "#7AE0FF",
      "light": "#16769B"
    },
    "series-12": {
      "dark": "#FFAFC7",
      "light": "#A74773"
    },
    "series-13": {
      "dark": "#8AE36C",
      "light": "#478100"
    },
    "series-14": {
      "dark": "#DAA1FF",
      "light": "#9744B8"
    },
    "series-15": {
      "dark": "#FFD34E",
      "light": "#8D6A00"
    },
    "series-16": {
      "dark": "#A6C7C2",
      "light": "#486A64"
    }
  },
  "chartAliases": {
    "accent": {
      "dark": "#FFB000",
      "light": "#F26A2E"
    },
    "success": {
      "dark": "#38E8A5",
      "light": "#007A4B"
    },
    "warning": {
      "dark": "#FFC43D",
      "light": "#9C5D00"
    },
    "danger": {
      "dark": "#FF7054",
      "light": "#D02D30"
    },
    "muted": {
      "dark": "#9AA9AA",
      "light": "#527278"
    },
    "emerald": {
      "dark": "#38E8A5",
      "light": "#007A4B"
    },
    "amber": {
      "dark": "#FFB000",
      "light": "#B87900"
    },
    "sky": {
      "dark": "#36CEFF",
      "light": "#007DA3"
    },
    "violet": {
      "dark": "#B59AFF",
      "light": "#7945D8"
    },
    "rose": {
      "dark": "#FF7054",
      "light": "#D02D30"
    },
    "teal": {
      "dark": "#2AE0CB",
      "light": "#007C73"
    },
    "indigo": {
      "dark": "#839FFF",
      "light": "#435BD0"
    },
    "yellow": {
      "dark": "#F4F36B",
      "light": "#697900"
    },
    "orange": {
      "dark": "#FF9850",
      "light": "#DE571E"
    }
  },
  "brandAssets": {
    "wordmark": {
      "dark": "/rad-text_white.svg",
      "light": "/rad-text_black.svg"
    },
    "logo": {
      "dark": "/rad-logo.svg",
      "light": "/rad-logo.svg"
    },
    "icon": {
      "dark": "/rad-icon.svg",
      "light": "/rad-icon.svg"
    },
    "favicon": {
      "dark": "/rad-favicon.svg",
      "light": "/rad-favicon.svg"
    }
  },
  "brandPaints": {
    "accent": {
      "light": "#C2410C",
      "dark": "#FFB000"
    },
    "accentMuted": {
      "light": "rgba(194, 65, 12, 0.14)",
      "dark": "rgba(255, 176, 0, 0.17)"
    },
    "mark": {
      "light": "#C2410C",
      "dark": "#FFB000"
    }
  }
};
export const BUILT_IN_THEMES = { classic: CLASSIC_THEME, rad: RAD_THEME } as const;
export const LEGACY_CHART_ALIASES = Object.freeze({ ...CLASSIC_THEME.chartAliases });

const colorPattern = /^#[0-9a-f]{6}([0-9a-f]{2})?$/i;
export const CUSTOMIZABLE_SEMANTIC_TOKENS = [
  'bg-page','bg-surface','bg-elevated','bg-glass','bg-overlay','text-primary','text-secondary','text-tertiary','text-disabled','text-on-accent',
  'border-default','border-strong','border-accent','status-positive','status-warning','status-danger','status-info',
  'charging-active','charging-done','charging-limited','charging-ac','charging-dc','charging-dcfc','dm-everyday','dm-conserve','dm-terrain','dm-sand','dm-rock','dm-rally','dm-drift','dm-towing','dm-unknown',
  'accent','accent-hover','accent-active','accent-muted','chart-grid','chart-hover-bg','chart-accent','chart-success','chart-warning','chart-danger','chart-muted','chart-amber','chart-yellow','chart-sky','chart-emerald','chart-violet','chart-rose','chart-teal','chart-orange','chart-indigo',
  'map-route-0','map-route-1','map-route-2','map-route-3','map-route-4','map-route-5',
] as const satisfies readonly SemanticToken[];
const modes = ['light', 'dark'] as const;
const overrideGroups = {
  tokens: { keys: new Set<string>(CUSTOMIZABLE_SEMANTIC_TOKENS), error: 'Token is not customizable', prefix: '' },
  series: { keys: new Set(Object.keys(CLASSIC_THEME.series)), error: 'Unknown series slot', prefix: '' },
  brandPaints: { keys: new Set(Object.keys(CLASSIC_THEME.brandPaints)), error: 'Unknown brand paint', prefix: 'brandPaints.' },
};

export function validateThemeOverride(override: ThemeOverride): string[] {
  const errors: string[] = [];
  Object.keys(override).forEach((property) => {
    if (property !== 'theme' && !Object.hasOwn(overrideGroups, property)) errors.push(`Unsupported theme property: ${property}`);
  });
  if (override.theme && !(override.theme in BUILT_IN_THEMES)) errors.push(`Unknown built-in theme: ${override.theme}`);
  for (const group of Object.keys(overrideGroups) as Array<keyof typeof overrideGroups>) {
    const contract = overrideGroups[group];
    const entries = Object.entries(override[group] ?? {});
    for (const [slot, pair] of entries) {
      if (!contract.keys.has(slot)) errors.push(`${contract.error}: ${slot}`);
      for (const [mode, value] of Object.entries(pair ?? {})) {
        const recognizedMode = mode === 'light' || mode === 'dark';
        if (!recognizedMode || (value && !colorPattern.test(value))) errors.push(`Invalid custom color for ${contract.prefix}${slot}.${mode}`);
      }
    }
  }
  return errors;
}

export function resolveTheme(override: ThemeOverride = {}): ResolvedTheme {
  const errors = validateThemeOverride(override);
  if (errors.length) throw new Error(errors.join('; '));
  const base = BUILT_IN_THEMES[override.theme ?? 'classic'];
  const tokens = { light: { ...base.tokens.light }, dark: { ...base.tokens.dark } };
  for (const mode of modes) {
    const patch = Object.entries(override.tokens ?? {}).filter(([, pair]) => pair?.[mode]);
    Object.assign(tokens[mode], Object.fromEntries(patch.map(([key, pair]) => [key, pair![mode]])));
  }
  const overlay = <T extends { [K in keyof T]: ThemeColorPair }>(original: T, changes: Partial<Record<keyof T, Partial<ThemeColorPair>>> | undefined): T => {
    const next = { ...original };
    for (const key of Object.keys(changes ?? {}) as Array<keyof T>) next[key] = Object.assign({}, original[key], changes![key]);
    return next;
  };
  return { ...base, tokens, series: overlay(base.series, override.series), brandPaints: overlay(base.brandPaints, override.brandPaints), sourceTheme: base.legacyPalette };
}

export function createThemeRegistry(definitions: readonly ThemeDefinition[]): ReadonlyMap<string, ThemeDefinition> {
  const entries = definitions.map((definition) => [definition.id, definition] as const);
  const first = new Map<string, number>();
  for (let index = 0; index < entries.length; index++) {
    const id = entries[index]![0];
    if (first.has(id)) throw new Error(`Duplicate theme id: ${id}`);
    first.set(id, index);
  }
  return new Map(entries);
}
export const THEME_REGISTRY = createThemeRegistry([CLASSIC_THEME, RAD_THEME]);
export function registryManifest() {
  return { schemaVersion: 1, registryHash: registryHash(), builtins: Array.from(THEME_REGISTRY.values()) };
}

export function stableThemeJson(value: unknown): string {
  type Part = { value: unknown; empty?: string } | { text: string };
  const pending: Part[] = [{ value }];
  const output: string[] = [];
  while (pending.length) {
    const part = pending.pop()!;
    if ('text' in part) { output.push(part.text); continue; }
    const current = part.value;
    if (!current || typeof current !== 'object') { output.push((JSON.stringify(current) ?? part.empty) as string); continue; }
    const array = Array.isArray(current);
    const keys = array ? Array.from({ length: current.length }, (_, index) => String(index)) : Object.keys(current).sort();
    const pieces: Part[] = [{ text: array ? '[' : '{' }];
    keys.forEach((key, index) => {
      if (index) pieces.push({ text: ',' });
      if (!array) pieces.push({ text: `${JSON.stringify(key)}:` });
      const item = (current as Record<string, unknown>)[key];
      pieces.push({ value: item, empty: array ? '' : 'undefined' });
    });
    pieces.push({ text: array ? ']' : '}' });
    pending.push(...pieces.reverse());
  }
  // The public helper follows JSON.stringify for a primitive undefined value.
  return output.length === 1 && output[0] === undefined ? undefined as unknown as string : output.join('');
}
export function themeHash(theme: ThemeDefinition): string {
  const serialized = stableThemeJson(theme).replace(/"definitionHash":(?:"[^"]*"|null),?/, '');
  const hash = Array.from(serialized).reduce((value, char) => Math.imul(value ^ char.charCodeAt(0), 16777619), 2166136261);
  return (hash >>> 0).toString(16).padStart(8, '0');
}
export function registryHash(): string {
  return [CLASSIC_THEME, RAD_THEME].map(themeHash).join('');
}
