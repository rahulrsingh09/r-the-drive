import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { isMain, walkFiles } from './lib/validation-files.mjs';

const requiredFiles = [
  'README.md',
  'AGENTS.md',
  'CLAUDE.md',
  'docs/index.md',
  'docs/branding.md',
  'docs/contributing.md',
  'docs/decision-log.md',
  'docs/security.md',
  'docs/security-audit.md',
  'docs/roadmap.md',
  'docs/privacy.md',
  'docs/environment-variables.md',
  'docs/architecture/overview.md',
  'docs/architecture/backend-data-flow.md',
  'docs/development.md',
  'docs/using-panda.md',
  'docs/operations.md',
  'docs/reference.md',
  'docs/runbooks/README.md',
  'docs/runbooks/documentation-maintenance.md',
  'docs/guides/README.md',
  'docs/guides/verify-installation.md',
  'apps/docs/docusaurus.config.ts',
  'apps/docs/sidebars.ts',
  'apps/docs/src/pages/index.tsx',
];

const routeSlugs = [
  'index',
  'battery',
  'battery.phantom-drain',
  'charging',
  'charging.$sessionId',
  'connect',
  'connect.otp',
  'd.$slug',
  'efficiency',
  'health',
  'login',
  'settings',
  'trips',
  'trips.$tripId',
  'users',
];

const requiredApiRouteFiles = [
  'auth.rs',
  'vehicles.rs',
  'battery.rs',
  'trips.rs',
  'charging.rs',
  'efficiency.rs',
  'health.rs',
];

const requiredGuides = [
  'README.md',
  'features.md',
  'getting-started.md',
  'prerequisites.md',
  'configuration.md',
  'deployment.md',
  'rivian-account.md',
  'backup-and-restore.md',
  'secure-deployment.md',
  'verify-installation.md',
  'dashboard-customization.md',
  'external-connections.md',
  'synology.md',
];

export function documentationFailures(root) {
  const failures = [];
  const cache = new Map();
  const exists = (file) => existsSync(join(root, file));
  const read = (file) => {
    if (!cache.has(file)) cache.set(file, readFileSync(join(root, file), 'utf8'));
    return cache.get(file);
  };
  const markdown = [
    ...walkFiles(join(root, 'docs'), { accept: (file) => file.endsWith('.md') }),
  ].map((file) => relative(root, file).replaceAll('\\', '/'));
  const allDocs = ['README.md', 'AGENTS.md', 'CLAUDE.md', ...markdown];
  const missing = (items, available, message) => {
    for (const item of items) if (!available.has(item)) failures.push(message(item));
  };
  const snippets = (file, required, forbidden, requiredMessage, forbiddenMessage) => {
    const text = read(file);
    for (const snippet of required)
      if (!text.includes(snippet)) failures.push(requiredMessage(snippet));
    for (const snippet of forbidden)
      if (text.includes(snippet)) failures.push(forbiddenMessage(snippet));
  };
  for (const file of requiredFiles)
    if (!exists(file)) failures.push(`missing required file: ${file}`);
  for (const file of allDocs) {
    for (const match of read(file).matchAll(/\[[^\]]+\]\(([^)]+)\)/g)) {
      const target = match[1].trim();
      if (!target || /^(?:https?:\/\/|mailto:|#)/.test(target)) continue;
      const destination = resolve(root, dirname(file), target.split('#', 1)[0]);
      if (!existsSync(destination)) failures.push(`broken markdown link in ${file}: ${target}`);
    }
  }
  const guides = new Set(readdirSync(join(root, 'docs/guides')));
  missing(requiredGuides, guides, (file) => `missing required guide: docs/guides/${file}`);
  // Published names are the literal filenames; the filesystem enforces uniqueness.
  const publishing = [
    "url: 'https://docs.r-the-drive.invalid'",
    "baseUrl: '/'",
    "path: '../../docs'",
    "routeBasePath: 'docs'",
    "onBrokenLinks: 'throw'",
    "onBrokenAnchors: 'throw'",
    '@cmfcmf/docusaurus-search-local',
    "label: 'Using R:The Drive'",
    "label: 'Operations'",
    "label: 'Reference'",
  ];
  snippets(
    'apps/docs/docusaurus.config.ts',
    publishing,
    [],
    (text) => `Docusaurus config is missing required publishing contract: ${text}`
  );
  const sidebars = read('apps/docs/sidebars.ts');
  for (const section of [
    'overviewSidebar',
    'gettingStartedSidebar',
    'usingPandaSidebar',
    'operationsSidebar',
    'developmentSidebar',
    'referenceSidebar',
  ]) {
    if (!sidebars.includes(section))
      failures.push(`Docusaurus sidebars are missing required functional section: ${section}`);
  }
  const sidebarReferences = new Map();
  for (const match of sidebars.matchAll(/["']([^"']+)["']/g))
    sidebarReferences.set(match[1], (sidebarReferences.get(match[1]) ?? 0) + 1);
  for (const file of markdown) {
    const id = file.slice(5, -3);
    const count = sidebarReferences.get(id) ?? 0;
    if (count !== 1)
      failures.push(`expected ${id} to appear in exactly one Docusaurus sidebar; found ${count}`);
  }
  for (const file of allDocs)
    snippets(
      file,
      [],
      [
        'github.com/your-org/r-the-drive/wiki',
        'scripts/publish-wiki.sh',
        '.github/workflows/publish-wiki.yml',
        'Wiki publishing',
        'publish to the Wiki',
        'publish through the Wiki',
      ],
      undefined,
      (text) => `legacy Wiki publishing reference in ${file}: ${text}`
    );
  const webRoutes = new Set(
    readdirSync(join(root, 'apps/web/src/routes'))
      .filter((file) => file.endsWith('.tsx'))
      .map((file) => file.slice(0, -4))
  );
  const apiRoutes = new Set(
    readdirSync(join(root, 'apps/api/src/routes')).filter((file) => file.endsWith('.rs'))
  );
  missing(
    routeSlugs,
    webRoutes,
    (slug) => `expected route slug missing from apps/web/src/routes: ${slug}`
  );
  missing(
    requiredApiRouteFiles,
    apiRoutes,
    (file) => `expected API route module missing from apps/api/src/routes: ${file}`
  );
  const config = read('apps/api/src/config.rs');
  const knownEnv = new Set(['PATH', 'HOME', 'CI']);
  for (const file of ['compose/.env.example', 'compose/.env.full.example']) {
    if (exists(file))
      for (const match of read(file).matchAll(/([A-Z][A-Z0-9_]+)=/g)) knownEnv.add(match[1]);
  }
  for (const match of config.matchAll(/"([A-Z0-9_]+)"/g)) knownEnv.add(match[1]);
  for (const file of [
    'README.md',
    'AGENTS.md',
    'CLAUDE.md',
    'docs/index.md',
    'docs/contributing.md',
    'docs/runbooks/documentation-maintenance.md',
    'docs/guides/configuration.md',
    'docs/environment-variables.md',
  ]) {
    for (const match of read(file).matchAll(/`([A-Z][A-Z0-9_]+)`/g)) {
      if (!knownEnv.has(match[1]))
        failures.push(`unknown env var referenced in ${file}: ${match[1]}`);
    }
  }
  const supported = new Set(
    [...read('compose/.env.full.example').matchAll(/^#?\s*([A-Z][A-Z0-9_]+)=/gm)].map(
      (match) => match[1]
    )
  );
  const reference = read('docs/environment-variables.md');
  for (const variable of supported)
    if (!reference.includes('`' + variable + '`'))
      failures.push(`environment reference is missing supported variable: ${variable}`);
  const sections = [
    [
      'pub struct Config',
      'pub struct RateLimitConfig',
      '',
      new Set(['RATE_LIMIT', 'RECOVERY', 'ORIGIN_BIND', 'SECURITY']),
    ],
    ['pub struct SecurityConfig', 'impl Default for SecurityConfig', '', new Set()],
    ['pub struct RateLimitConfig', 'pub struct RecoveryConfig', 'RATE_LIMIT_', new Set()],
    ['pub struct RecoveryConfig', 'pub struct OriginBindConfig', 'RECOVERY_', new Set()],
    ['pub struct OriginBindConfig', 'fn default_port', '', new Set()],
  ];
  const runtime = ['RIVIAN_GRAPHQL_GATEWAY_URL', 'RUST_LOG'];
  for (const [start, end, prefix, ignored] of sections) {
    const block = config.slice(config.indexOf(start), config.indexOf(end));
    for (const match of block.matchAll(/pub ([a-z][a-z0-9_]+):/g)) {
      const field = match[1].toUpperCase();
      if (!ignored.has(field)) runtime.push(prefix + field);
    }
  }
  missing(
    runtime,
    supported,
    (variable) => `compose/.env.full.example is missing runtime variable: ${variable}`
  );
  for (const match of read('compose/docker-compose.yml').matchAll(/\$\{([A-Z][A-Z0-9_]+)/g)) {
    if (!supported.has(match[1]))
      failures.push(
        `compose/.env.full.example is missing production Compose variable: ${match[1]}`
      );
  }
  if (/^PANDA_SETUP_TOKEN=/m.test(read('compose/.env.example')))
    failures.push('compact production env must not ship an active first-owner setup token');
  snippets(
    'compose/docker-compose.yml',
    [
      '"${PANDA_HOST_BIND_ADDRESS:-127.0.0.1}:${PANDA_ORIGIN_PORT:-8080}:8080"',
      'PANDA_IMAGE:?Set PANDA_IMAGE',
      'PANDA_DB_SOURCE',
      'PANDA_BACKUPS_SOURCE',
      'PANDA_CACHE_SOURCE',
      'start_period: 300s',
      'redis:',
    ],
    ['"80:80"', '"443:443"', 'COOKIE_INSECURE:', '  nginx:'],
    (text) => `production compose is missing required secure-deployment contract: ${text}`,
    (text) => `production compose must not include direct-public deployment setting: ${text}`
  );
  snippets(
    'compose/docker-compose.build.yml',
    ['context: ..', 'dockerfile: compose/Dockerfile'],
    [],
    (text) => `build overlay is missing required source-build contract: ${text}`
  );
  if (read('compose/docker-compose.yml').includes('build:'))
    failures.push('standard Compose must pull published images instead of defining build contexts');
  snippets(
    'compose/nginx/nginx.conf',
    ['http://127.0.0.1:3001', 'listen 8080;'],
    [],
    (text) => `nginx must use the internal secure-deployment topology: ${text}`
  );
  if (
    [
      'compose/docker-compose.synology.yml',
      'compose/.env.synology.example',
      'compose/synology/docker-compose.yml',
      'compose/synology/.env.synology.example',
    ].some(exists)
  )
    failures.push(
      'Synology must use the universal Compose file instead of a second deployment definition'
    );
  return failures;
}

if (isMain(import.meta.url)) {
  const failures = documentationFailures(process.cwd());
  if (!failures.length) console.log('docs:check passed');
  else {
    failures.forEach((message) => console.error(`docs:check failed: ${message}`));
    process.exitCode = 1;
  }
}
