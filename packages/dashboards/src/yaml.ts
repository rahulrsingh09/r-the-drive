import { parse, stringify } from 'yaml';
import { v4 as uuidv4 } from 'uuid';
import { SCHEMA_VERSION } from './schema';
import { sanitizeDashboardConfig } from './layout';
import type { DashboardConfig, DashboardExport } from './schema';
import { normalizeDashboardConfig } from './api';

const YAML_HEADER = `# Panda dashboard v${SCHEMA_VERSION}\n`;

function toDashboardExport(config: DashboardConfig): DashboardExport {
  return {
    schemaVersion: config.schemaVersion,
    slug: config.slug,
    name: config.name,
    description: config.description,
    controls: config.controls,
    widgets: config.widgets,
  };
}

export function exportDashboardYaml(config: DashboardConfig): string {
  return YAML_HEADER + stringify(toDashboardExport(config), { indent: 2 });
}

function createImportedDashboard(document: unknown): DashboardConfig {
  return sanitizeDashboardConfig({
    ...(document as DashboardExport),
    id: uuidv4(),
    isDefault: false,
    isLocked: false,
    ownerId: null,
  });
}

/** Parse YAML text → DashboardConfig ready to POST (new id, no owner). */
export function importDashboardYaml(text: string): DashboardConfig {
  const document: unknown = parse(text);
  const validated = createImportedDashboard(document);
  return normalizeDashboardConfig(validated);
}

export function downloadDashboardYaml(config: DashboardConfig) {
  const blob = new Blob([exportDashboardYaml(config)], { type: 'text/yaml' });
  const url = URL.createObjectURL(blob);
  try {
    const anchor = Object.assign(document.createElement('a'), { href: url, download: `${config.slug}.yaml` });
    anchor.click();
  } finally {
    URL.revokeObjectURL(url);
  }
}
