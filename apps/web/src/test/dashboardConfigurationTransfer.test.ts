import { afterEach, describe, expect, it, vi } from 'vitest';
import { parse, stringify } from 'yaml';
import {
  downloadDashboardYaml, exportDashboardYaml, importDashboardYaml,
} from '../../../../packages/dashboards/src/yaml';
import {
  downloadChartJson, exportChartJson, importChartJson, MAX_CHART_EXPORT_BYTES, parseChartJson,
} from '../../../../packages/dashboards/src/charts/portable';
import { configurationChart, configurationDashboard, configurationWidget } from './dashboardConfigurationFixtures';

describe('dashboard configuration transfer', () => {
  it('exports the ordered transfer fields, never server metadata', () => {
    const config = configurationDashboard();
    const original = structuredClone(config);
    const yaml = exportDashboardYaml(config);
    expect(yaml.startsWith('# Panda dashboard v2\n')).toBe(true);
    expect(Object.keys(parse(yaml))).toEqual(['schemaVersion', 'slug', 'name', 'description', 'controls', 'widgets']);
    expect(parse(yaml)).toEqual({
      schemaVersion: 2, slug: config.slug, name: config.name, description: config.description,
      controls: config.controls, widgets: config.widgets,
    });
    expect(config).toEqual(original);
  });

  it('imports JSON-as-YAML and resets identity even when a file supplies server fields', () => {
    const config = configurationDashboard();
    const imported = importDashboardYaml(JSON.stringify(config));
    expect(imported).toEqual({ ...config, id: expect.any(String), ownerId: null, isDefault: false, isLocked: false });
    expect(imported.id).not.toBe(config.id);
    expect(importDashboardYaml(JSON.stringify(config)).id).not.toBe(imported.id);
  });

  it.each([
    'true', 'null', '001', '# comment', 'name: value', 'line 1\nline 2\n',
    '"quoted" \\ path', '<script>alert("synthetic")</script>', '🚙 café\u2028line',
  ])('round-trips quoted, multiline and Unicode strings: %s', (text) => {
    const config = configurationDashboard();
    config.name = text;
    config.description = text;
    config.widgets[0]!.title = text;
    config.widgets[0]!.options = { text };
    const imported = importDashboardYaml(exportDashboardYaml(config));
    expect(imported.name).toBe(text);
    expect(imported.description).toBe(text);
    expect(imported.widgets).toEqual(config.widgets);
    expect(parse(exportDashboardYaml(imported))).toEqual(parse(exportDashboardYaml(config)));
    expect(exportDashboardYaml(importDashboardYaml(exportDashboardYaml(imported)))).toBe(exportDashboardYaml(imported));
  });

  it('strips unknown schema keys and preserves opaque options through import', () => {
    const config = {
      ...configurationDashboard(), extension: 'discarded',
      controls: { dateRange: false, extension: true },
      widgets: [{ ...configurationWidget(), extension: true, managed: true, managedKey: 'synthetic' }],
    };
    const imported = importDashboardYaml(stringify(config));
    expect(imported).not.toHaveProperty('extension');
    expect(imported.controls).toEqual({ dateRange: false });
    expect(imported.widgets[0]).toEqual({ ...configurationWidget(), managed: true, managedKey: 'synthetic' });
  });

  it('retains legacy migration ordering and unrelated options', () => {
    const config = configurationDashboard();
    config.widgets = [{
      ...configurationWidget(), componentType: 'custom', definitionId: 'trips.stat',
      visibility: undefined, options: { stat: 'miles', chargingConnectionVisibility: 'unplugged', label: 'keep' },
    }];
    const imported = importDashboardYaml(exportDashboardYaml(config));
    expect(imported.widgets[0]).toMatchObject({
      componentType: 'sensor', definitionId: 'trip_miles',
      options: { metric: 'trip_miles', valueMode: 'sum', tripSelectionAware: true, label: 'keep' },
      visibility: [{ type: 'vehicle-connection', value: 'unplugged' }],
    });
    expect(imported.widgets[0]!.options).not.toHaveProperty('stat');
    expect(imported.widgets[0]!.options).not.toHaveProperty('chargingConnectionVisibility');
    expect(importDashboardYaml(exportDashboardYaml(imported)).widgets).toEqual(imported.widgets);
  });

  it.each(['', 'null', '42', '[]', '- widget', 'bad: [', 'name: a\nname: b', 'alias: *missing'])('rejects malformed/incomplete YAML: %s', (text) => {
    expect(() => importDashboardYaml(text)).toThrow();
  });

  it.each([
    { schemaVersion: 1 }, { controls: undefined }, { widgets: undefined },
    { slug: 'Uppercase' }, { name: '' }, { name: 'x'.repeat(101) },
    { widgets: [{ ...configurationWidget(), layout: { x: 12, y: 0, w: 3, h: 2 } }] },
    { widgets: [{ ...configurationWidget(), visibility: [{ type: 'future-rule', value: true }] }] },
  ])('does not let normalization defaults repair invalid imports: %j', (patch) => {
    expect(() => importDashboardYaml(JSON.stringify({ ...configurationDashboard(), ...patch }))).toThrow();
  });

  it('accepts empty dashboards and the maximum name length without inserting defaults', () => {
    const config = { ...configurationDashboard(), name: 'x'.repeat(100), widgets: [] };
    const imported = importDashboardYaml(exportDashboardYaml(config));
    expect(imported.name).toBe(config.name);
    expect(imported.controls).toEqual({ dateRange: false });
    expect(imported.widgets).toEqual([]);
  });
});

describe('portable chart configuration transfer', () => {
  it('round-trips synthetic definitions while excluding record ownership and baseline fields', () => {
    const record = configurationChart();
    const original = structuredClone(record);
    const text = exportChartJson(record);
    const document = parseChartJson(text);
    expect(document).toEqual({
      kind: 'panda-chart', version: 1,
      chart: {
        slug: record.slug, name: record.name, description: record.description,
        enabled: false, definition: record.config,
      },
    });
    const imported = importChartJson(text);
    expect(imported).toEqual({
      slug: record.slug, name: record.name, description: record.description,
      isDefault: false, isLocked: false, isEnabled: false, config: record.config,
    });
    expect(exportChartJson({ ...imported, id: 'unused', ownerId: null })).toBe(text);
    expect(record).toEqual(original);
  });

  it.each([undefined, '', '   ', '  padded  ', '"quoted"\\\n🚙'])('preserves the export/parse/import description boundaries: %j', (description) => {
    const record = configurationChart();
    if (description === undefined) delete record.description;
    else record.description = description;
    const text = exportChartJson(record);
    const raw = JSON.parse(text);
    expect(raw.chart.description).toBe(description || undefined);
    const parsed = parseChartJson(text);
    expect(parsed.chart.description).toBe(description ? description.trim() : undefined);
    const imported = importChartJson(text);
    expect(imported.description).toBe(description?.trim() || undefined);
    expect(Object.hasOwn(imported, 'description')).toBe(Boolean(description?.trim()));
  });

  it('exports without validation but normalizes and validates on parse', () => {
    const record = configurationChart();
    record.name = '  padded  ';
    record.config.series[0]!.label = '  value  ';
    record.config.series[0]!.transforms = [{ kind: 'expression', formula: '  [value]  ' }];
    expect(JSON.parse(exportChartJson(record)).chart.name).toBe('  padded  ');
    const parsed = parseChartJson(exportChartJson(record));
    expect(parsed.chart.name).toBe('padded');
    expect(parsed.chart.definition.series[0]!.label).toBe('value');
    expect(parsed.chart.definition.series[0]!.transforms).toEqual([{ kind: 'expression', formula: '[value]' }]);
    record.name = '';
    expect(() => exportChartJson(record)).not.toThrow();
    expect(() => parseChartJson(exportChartJson(record))).toThrow();
  });

  it('strips unknown structured keys but retains source params and renderer identifiers', () => {
    const raw = JSON.parse(exportChartJson(configurationChart()));
    raw.extra = 'discarded';
    raw.chart.ownerId = 'discarded';
    raw.chart.definition.extra = 'discarded';
    raw.chart.definition.series[0].extra = 'discarded';
    raw.chart.definition.sources[0].extra = 'discarded';
    expect(parseChartJson(JSON.stringify(raw))).toEqual(parseChartJson(exportChartJson(configurationChart())));
  });

  it('applies schema defaults for source params, filters and series transforms', () => {
    const raw = JSON.parse(exportChartJson(configurationChart()));
    delete raw.chart.definition.sources[0].params;
    delete raw.chart.definition.sources[0].filters;
    delete raw.chart.definition.series[0].transforms;
    const definition = parseChartJson(JSON.stringify(raw)).chart.definition;
    expect(definition.sources[0]).toMatchObject({ params: {}, filters: [] });
    expect(definition.series[0]!.transforms).toEqual([]);
  });

  it.each(['url', 'remote_url', 'SQL', 'java_script', 'script', 'template_html', 'css'])('rejects forbidden keys retained in opaque params: %s', (key) => {
    const record = configurationChart();
    record.config.sources[0]!.params = { nested: [{ [key]: 'synthetic' }] };
    expect(() => parseChartJson(exportChartJson(record))).toThrow(/Executable, remote, SQL, HTML, or CSS fields are not allowed/);
  });

  it('leaves validation of stripped unknown fields and executable-looking strings at the existing boundary', () => {
    const raw = JSON.parse(exportChartJson(configurationChart()));
    raw.chart.definition.javascript = 'discarded by the schema';
    raw.chart.definition.sources[0].params.label = '<script>synthetic</script>';
    const definition = parseChartJson(JSON.stringify(raw)).chart.definition;
    expect(definition).not.toHaveProperty('javascript');
    expect(definition.sources[0]!.params.label).toBe('<script>synthetic</script>');
  });

  it.each([
    ['duplicate source', (record: ReturnType<typeof configurationChart>) => record.config.sources.push(record.config.sources[0]!)],
    ['duplicate series', (record: ReturnType<typeof configurationChart>) => record.config.series.push(record.config.series[0]!)],
    ['unknown reference', (record: ReturnType<typeof configurationChart>) => { record.config.x.field.sourceBindingId = 'absent'; }],
    ['missing right axis', (record: ReturnType<typeof configurationChart>) => { record.config.series[0]!.yAxis = 'y2'; }],
    ['inverted domain', (record: ReturnType<typeof configurationChart>) => { record.config.axes.y.domain = { mode: 'fixed', min: 2, max: 1 }; }],
    ['nonpositive log domain', (record: ReturnType<typeof configurationChart>) => { record.config.axes.y.scale = 'log'; }],
    ['missing filter value', (record: ReturnType<typeof configurationChart>) => { record.config.sources[0]!.filters = [{ field: 'value', operator: 'eq' }]; }],
    ['unexpected filter value', (record: ReturnType<typeof configurationChart>) => { record.config.sources[0]!.filters = [{ field: 'value', operator: 'is_null', value: null }]; }],
    ['long formula', (record: ReturnType<typeof configurationChart>) => { record.config.series[0]!.transforms = [{ kind: 'expression', formula: '1'.repeat(513) }]; }],
  ])('retains definition validation for %s', (_label, change) => {
    const record = configurationChart();
    change(record);
    expect(() => parseChartJson(exportChartJson(record))).toThrow();
    expect(() => importChartJson(exportChartJson(record))).toThrow();
  });

  it('does not validate source availability, bundled renderer capabilities or formula syntax during import', () => {
    const record = configurationChart();
    record.slug = 'soc-history';
    record.config.series[0]!.transforms = [{ kind: 'expression', formula: 'unknown(1)' }];
    expect(importChartJson(exportChartJson(record)).config).toEqual(record.config);
  });

  it.each(['', 'null', '[]', '42', '{', '{"kind":"panda-chart","version":2}', '{"kind":"other","version":1}'])('rejects malformed documents: %s', (text) => {
    expect(() => parseChartJson(text)).toThrow();
  });

  it('counts UTF-8 bytes, accepts exactly 256 KiB and rejects one byte more before JSON parsing', () => {
    const record = configurationChart();
    record.description = '🚙 café';
    const text = exportChartJson(record);
    const bytes = new TextEncoder().encode(text).byteLength;
    expect(bytes).toBeGreaterThan(text.length);
    const atLimit = text + ' '.repeat(MAX_CHART_EXPORT_BYTES - bytes);
    expect(parseChartJson(atLimit)).toEqual(parseChartJson(text));
    expect(() => parseChartJson(`${atLimit} `)).toThrow('Chart export exceeds the 256 KiB portable file limit');
    expect(() => parseChartJson('!'.repeat(MAX_CHART_EXPORT_BYTES + 1))).toThrow('Chart export exceeds the 256 KiB portable file limit');
  });
});

describe('configuration download adapters', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it.each([
    ['dashboard', downloadDashboardYaml, configurationDashboard, 'configuration-test.yaml', 'text/yaml'],
    ['chart', downloadChartJson, configurationChart, 'configuration-test.panda-chart.json', 'application/json'],
  ] as const)('downloads %s using the persisted filename and MIME type', (_kind, download, fixture, filename, type) => {
    const createObjectURL = vi.fn((_blob: Blob) => 'blob:synthetic');
    const revokeObjectURL = vi.fn();
    vi.stubGlobal('URL', { createObjectURL, revokeObjectURL });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    if (_kind === 'dashboard') download(fixture());
    else download(fixture());
    const anchor = click.mock.instances[0] as HTMLAnchorElement;
    expect(anchor.download).toBe(filename);
    expect(anchor.href).toBe('blob:synthetic');
    expect(createObjectURL.mock.calls[0]![0]).toBeInstanceOf(Blob);
    expect((createObjectURL.mock.calls[0]![0] as Blob).type).toBe(type);
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:synthetic');
    expect(click.mock.invocationCallOrder[0]).toBeLessThan(revokeObjectURL.mock.invocationCallOrder[0]!);
  });
});
