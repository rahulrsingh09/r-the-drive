import { existsSync, readFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import { parse } from 'yaml';
import { isMain } from './lib/validation-files.mjs';

const promotion = [
  [/(?:candidate-\$SOURCE_SHA-amd64|architectures = \["amd64", "arm64"\][^\n]*else \["amd64"\][\s\S]*?candidates = \{arch: f"\{image\}:candidate-\{source\}-\{arch\}" for arch in architectures\})/, 'must require the exact AMD64 commit candidate'],
  [/(?:candidate-\$SOURCE_SHA-arm64|architectures = \["amd64", "arm64"\] if os\.environ\["INCLUDE_ARM64"\] == "true" else \["amd64"\][\s\S]*?candidates = \{arch: f"\{image\}:candidate-\{source\}-\{arch\}" for arch in architectures\})/, 'must support an optional exact ARM64 candidate'],
  [/(?:imagetools create --metadata-file|"imagetools", "create", "--metadata-file", str\(metadata\))/, 'must promote candidates by immutable manifest'],
  [/containerimage\.descriptor.*containerimage\.digest/s, 'must capture the promoted digest'],
  [
    /attest-build-provenance@0f67c3f4856b2e3261c31976d6725780e5e4c373/,
    'must attest the promoted digest',
  ],
];
const promotionWorkflow = [
  [/include_arm64:[\s\S]*?default:\s*false/, 'must keep ARM64 opt-in'],
  [/needs:\s*\[validate(?:-tag)?,\s*promote-image\]/, 'downstream gates must depend on promotion'],
  [
    /needs:\s*\[validate(?:-tag)?,\s*promote-image,\s*release-smoke,\s*populated-upgrade\]/,
    'release creation must wait for promoted-image verification',
  ],
  [/needs\.promote-image\.outputs\.digest/, 'downstream jobs must consume the promoted digest'],
  [
    /verify-fresh-install\.mjs[^\n]*--image-ref "\$\{\{ env\.REGISTRY \}\}\/\$\{\{ env\.OWNER \}\}\/panda@\$\{\{ needs\.promote-image\.outputs\.digest \}\}"/,
    'smoke test must verify the promoted digest instead of a mutable tag',
  ],
];
const candidateWorkflow = [
  [/^\s{2}workflow_dispatch:/m, 'candidate workflow must remain manually dispatched'],
  [/cancel-in-progress:\s*false/, 'candidate workflow must not cancel an in-flight dev promotion'],
  [/runs-on:\s*ubuntu-24\.04(?:\s|$)/, 'AMD64 candidates must use the native AMD64 runner'],
  [/runs-on:\s*ubuntu-24\.04-arm(?:\s|$)/, 'ARM64 candidates must use the native ARM64 runner'],
  [/Build candidate \(arm64, manual\)/, 'ARM64 candidate builds must remain manual'],
  [
    /cache-from:\s*type=registry,ref=.*buildcache-(?:amd64|arm64)-v2/g,
    'candidate workflow must import one GHCR cache per platform',
    2,
  ],
  [
    /cache-to:\s*type=registry,ref=.*buildcache-(?:amd64|arm64)-v2/g,
    'candidate workflow must export one GHCR cache per platform',
    2,
  ],
  [
    /candidate-\$\{\{ needs\.resolve\.outputs\.source_sha \}\}-amd64/,
    'candidate workflow must publish an exact AMD64 commit tag',
  ],
  [
    /candidate-\$\{\{ needs\.resolve\.outputs\.source_sha \}\}-arm64/,
    'candidate workflow must publish an exact ARM64 commit tag',
  ],
  [
    /source_ref:\s*\$\{\{ steps\.source\.outputs\.source_ref \}\}/,
    'candidate workflow must retain the requested source ref',
  ],
  [
    /prune-candidates:[\s\S]*?(?:keep=10[\s\S]*?keep=3|for arch, limit in \[\("amd64", 10\), \("arm64", 3\)\]:[\s\S]*?ordered\[limit:\])/,
    'candidate workflow must bound AMD64 and ARM64 candidate retention',
  ],
  [
    /(?:gh api --method DELETE[^\n]*packages\/container\/panda\/versions\/\$id|package = f"\/users\/\{os\.environ\[\x27GITHUB_REPOSITORY_OWNER\x27\]\}\/packages\/container\/panda\/versions"[\s\S]*?\["gh", "api", "--method", "DELETE", f"\{package\}\/\{version_id\}"\])/,
    'candidate workflow must delete stale candidate versions by exact package version ID',
  ],
  [
    /(?:\^\[0-9a-f\]\{7\}-dev\$|re\.fullmatch\(r"\[0-9a-f\]\{7\}-dev", tags\))/,
    'candidate workflow must remove orphaned legacy dev candidates without matching the moving dev tag',
  ],
];
const devPromotion = [
  [
    /needs:\s*\[resolve, build-amd64, build-arm64\]/,
    'dev promotion must wait for both candidate build jobs',
  ],
  [
    /needs\.build-amd64\.result == 'success'/,
    'dev promotion must require a successful AMD64 build',
  ],
  [
    /needs\.resolve\.outputs\.source_ref == 'dev'/,
    'dev promotion must require source_ref exactly dev',
  ],
  [
    /needs\.resolve\.outputs\.platform == 'both' && needs\.build-arm64\.result == 'success'/,
    'both-platform dev promotion must require successful ARM64',
  ],
  [/git\/ref\/heads\/dev.*object\.sha/s, 'dev promotion must reread the current upstream dev SHA'],
  [
    /(?:current_dev_sha.*SOURCE_SHA|source = os\.environ\["IMAGE"\], os\.environ\["SOURCE_SHA"\][\s\S]*?current = subprocess\.check_output\([\s\S]*?if current != source:[\s\S]*?raise SystemExit)/s,
    'dev promotion must compare the upstream dev SHA to the built source',
  ],
  [
    /(?:imagetools create --metadata-file[\s\S]*--tag "\$IMAGE:dev"|"imagetools", "create", "--metadata-file", str\(metadata\), "--tag", f"\{image\}:dev", \*manifests)/,
    'dev promotion must retag candidates without rebuilding',
  ],
  [
    /(?:imagetools inspect "\$IMAGE:dev" --format.*Manifest\.Digest|"imagetools", "inspect", f"\{image\}:dev", "--format", "\{\{\.Manifest\.Digest\}\}")/s,
    'dev promotion must verify the mutable tag digest',
  ],
  [
    /(?:published_digest.*digest|if published != digest:[\s\S]*?raise SystemExit)/s,
    'dev promotion must compare the published digest to the promoted digest',
  ],
  [
    /attest-build-provenance@0f67c3f4856b2e3261c31976d6725780e5e4c373/,
    'dev promotion must attest the promoted digest',
  ],
];
const fileContracts = {
  '.github/workflows/quality.yml': [
    [
      /run:\s*pnpm release-workflows:check/,
      'quality workflow must invoke the release workflow checker',
    ],
  ],
  'compose/Dockerfile': [
    [
      /COPY packages\/themes\/package\.json packages\/themes\//,
      'production Dockerfile must include the themes workspace manifest before install',
    ],
    [
      /COPY packages\/themes\/ packages\/themes\//,
      'production Dockerfile must include the themes workspace source before the web build',
    ],
  ],
  'compose/docker-compose.yml': [
    [/image: \$\{PANDA_IMAGE:(?:-|\?)/, 'production Compose must accept an exact image reference'],
  ],
  'scripts/verify-fresh-install.mjs': [
    [
      /const fields = \[[^\]]*'image-ref'[^\]]*\];[\s\S]*?this\.options = Object\.fromEntries\(fields\.map\(name => \[name, option\(args, '--' \+ name\)\]\)\)/,
      'fresh-install verification must accept --image-ref',
    ],
    [
      /ref = this\.options\['image-ref'\][\s\S]*?const image = this\.sourceBuild \? 'panda:local' : ref \|\|[\s\S]*?if \(image\) this\.environment\.PANDA_IMAGE = image/,
      'fresh-install verification must pass the exact image reference to Compose',
    ],
    [
      /const token = await this\.owner\(origin\);[\s\S]*?this\.command\(\['restart', 'panda'\]\);[\s\S]*?await this\.dashboards\(origin, token\)/,
      'fresh-install verification must prove pre-restart access tokens survive a service restart',
    ],
  ],
  '.github/workflows/runtime.yml': [
    [
      /PANDA_ENV:\s*development/,
      'runtime workflow must explicitly classify its local dependency stack as development',
    ],
    [/(?:cargo run --bin panda-api|\["cargo", "run", "--bin", "panda-api"\])/, 'runtime workflow must select the API binary explicitly'],
    [
      /imagetools inspect "ghcr\.io\/your-org\/r-the-drive:candidate-\$\{GITHUB_SHA\}-amd64"/,
      'runtime workflow must verify the exact AMD64 commit candidate',
    ],
  ],
  '.github/workflows/fresh-install.yml': [
    [
      /pnpm verify:fresh-install -- --mode "\$\{\{ inputs\.mode \|\| 'production' \}\}" --production-env "\$fresh_env" --source-build/,
      'fresh-install workflow must exercise the production acceptance path',
    ],
  ],
  '.github/workflows/prepare-release.yml': [
    [/packages:\s*read/, 'prepare-release.yml must be able to inspect GHCR candidates'],
    [
      /docker\/login-action@c94ce9fb468520275223c153574b00df6fe4bcc9/,
      'prepare-release.yml must authenticate to GHCR before candidate verification',
    ],
    [
      /docker buildx imagetools inspect "\$candidate"/,
      'prepare-release.yml must verify the exact main AMD64 candidate before tagging',
    ],
  ],
  'scripts/verify-backup-restore-s3.mjs': [
    [
      /chown 1001:1001 \/data\/backups \/data\/cache/,
      'S3 drill must prepare disposable bind mounts for the production container user',
    ],
    [
      /stack\(name, offset\) \{[\s\S]*?this\.dataCommand\(directory, [^\n]*chown 1001:1001[^\n]*\);[\s\S]*?this\.commands\.invoke\('docker', \[\.\.\.this\.compose\(stack\.project, envFile\), 'up'[\s\S]*?const source = this\.stack\('source', 1000\)/,
      'S3 drill must prepare the source bind mount before starting its stack',
    ],
    [
      /stack\(name, offset\) \{[\s\S]*?this\.dataCommand\(directory, [^\n]*chown 1001:1001[^\n]*\);[\s\S]*?this\.commands\.invoke\('docker', \[\.\.\.this\.compose\(stack\.project, envFile\), 'up'[\s\S]*?const target = this\.stack\('target', 2000\)/,
      'S3 drill must prepare the target bind mount before starting its stack',
    ],
    [
      /await this\.backup\(source, sourceToken, backup\.run\.id\);[\s\S]*?this\.dataCommand\(source\.directory, "find \/data\/backups/,
      'S3 drill must wait for the asynchronous backup run before restoring it',
    ],
    [
      /if \(run\?\.status !== 'succeeded'\) return false;[\s\S]*?artifacts = overview\.artifacts\?\.filter\(row => row\.run_id === id\)[\s\S]*?stores = new Set\(artifacts\.map\(row => row\.storage_type\)\)[\s\S]*?if \(!stores\.has\('local'\) \|\| !stores\.has\('s3'\)\) throw/,
      'S3 drill must require both local and S3 artifacts from the completed run',
    ],
    [
      /phase === 'failed'[\s\S]*?job\.error_message/,
      'S3 drill must surface terminal restore-agent errors instead of swallowing them',
    ],
    [
      /RUST_LOG: 'panda_api=info,panda_restore_agent=error,tower_http=info'/,
      'S3 drill must retain restore-agent error diagnostics',
    ],
  ],
};

export function releaseWorkflowFailures(root) {
  const failures = [];
  const documents = new Map();
  const read = (file) => {
    if (!documents.has(file)) {
      if (!existsSync(join(root, file))) {
        failures.push(`Missing ${file}`);
        documents.set(file, '');
      } else documents.set(file, readFileSync(join(root, file), 'utf8'));
    }
    return documents.get(file);
  };
  const evaluate = (source, rules, prefix = '') => {
    for (const [expression, description, expected] of rules) {
      const matches = source.match(new RegExp(expression.source, expression.flags));
      if (expected === undefined) {
        if (!matches) failures.push(prefix + description);
      } else if ((matches?.length ?? 0) !== expected)
        failures.push(
          `${prefix}${description} (found ${matches?.length ?? 0}, expected ${expected})`
        );
    }
  };
  const forbid = (source, expression, message) => {
    if (expression.test(source)) failures.push(message);
  };
  const before = (source, first, second, message) => {
    const a = source.indexOf(first),
      b = source.indexOf(second);
    if (a < 0 || b < 0 || a > b) failures.push(message);
  };
  const workflow = (file) => {
    const source = read(file);
    const document = parse(source);
    for (const match of source.matchAll(/^\s*-?\s*uses:\s*[^\s@]+@([^\s#]+)(?:\s|$)/gim)) {
      if (!/^[0-9a-f]{40}$/i.test(match[1]))
        failures.push(`${basename(file)} contains an unpinned or short action SHA: ${match[1]}`);
    }
    const starts = [...source.matchAll(/^ {2}([\w-]+):/gm)];
    return {
      source,
      job(name) {
        const index = starts.findIndex(
          (match) => match[1] === name && Object.hasOwn(document?.jobs ?? {}, name)
        );
        if (index < 0) {
          failures.push(`${name} job is missing`);
          return '';
        }
        return source.slice(starts[index].index, starts[index + 1]?.index ?? source.length);
      },
    };
  };
  const candidate = workflow('.github/workflows/publish-candidate-image.yml');
  evaluate(candidate.source, candidateWorkflow);
  evaluate(candidate.job('promote-dev'), devPromotion);
  forbid(
    candidate.source,
    /^\s{2}push:/m,
    'candidate workflow must not build on every main or dev push'
  );
  forbid(
    candidate.source,
    /type=gha/,
    'candidate workflow must not consume GitHub Actions cache storage for BuildKit'
  );
  for (const name of ['build-amd64', 'build-arm64']) {
    const job = candidate.job(name);
    evaluate(job, [
      [
        /Apply pnpm patch compatibility overlay/g,
        `${name} must apply the pnpm patch compatibility overlay once`,
        1,
      ],
      [
        /(?:contents\.count\(old\) != 1|len\(pieces\) != 2)/g,
        `${name} must require exactly one historical install seam`,
        1,
      ],
    ]);
    before(
      job,
      job.includes('if compatibility not in contents:') ? 'if compatibility not in contents:' : 'if grep -Fq "$compatibility"',
      job.includes('pieces = contents.split(') ? 'pieces = contents.split("RUN pnpm install --frozen-lockfile")' : "grep -Fq 'RUN pnpm install --frozen-lockfile'",
      `${name} must check compatibility before the historical install seam`
    );
  }
  for (const [file, stable] of [
    ['publish-release-images.yml', true],
    ['publish-prerelease-images.yml', false],
  ]) {
    const data = workflow('.github/workflows/' + file);
    const job = data.job('promote-image');
    if (!parse(data.source)?.jobs?.['release-smoke'])
      failures.push('release-smoke job boundary is missing after promote-image');
    evaluate(data.source, promotionWorkflow, file + ' ');
    evaluate(job, promotion, file + ' ');
    forbid(
      data.source,
      /docker\/build-push-action/,
      `${file} must promote a candidate instead of rebuilding the image`
    );
    evaluate(
      job,
      stable
        ? [
            [
              /(?:--tag "\$IMAGE:\$VERSION" --tag "\$IMAGE:latest"|tags = \["--tag", f"\{image\}:\{version\}", "--tag", f"\{image\}:latest"\])/,
              'stable promotion must publish version and latest tags',
            ],
          ]
        : [
            [
              /(?:--tag "\$IMAGE:\$VERSION" "\$\{sources\[@\]\}"|tags = \["--tag", f"\{image\}:\{version\}"\][\s\S]*?\*tags, \*manifests)/,
              'preview promotion must publish only the version tag',
            ],
          ],
      file + ' '
    );
    evaluate(
      data.source,
      stable
        ? [
            [
              /(?:git merge-base --is-ancestor "\$GITHUB_SHA" origin\/main|\["git", "merge-base", "--is-ancestor", os\.environ\["GITHUB_SHA"\], "origin\/main"\], check=True)/,
              'must retain main ancestry validation',
            ],
          ]
        : [
            [/ref: dev/, 'must retain dev source semantics'],
            [/source_sha:/, 'must retain source SHA semantics'],
          ],
      file + ' '
    );
  }
  for (const [file, rules] of Object.entries(fileContracts)) evaluate(read(file), rules);
  workflow('.github/workflows/prepare-release.yml');
  if (
    JSON.parse(read('package.json')).scripts?.['release-workflows:check'] !==
    'node ./scripts/check-release-workflows.mjs'
  )
    failures.push('package.json must expose release-workflows:check');
  const dockerfile = read('compose/Dockerfile');
  before(
    dockerfile,
    'COPY packages/themes/package.json packages/themes/',
    'RUN pnpm config set allowUnusedPatches true --location project',
    'production Dockerfile must copy the themes workspace manifest before pnpm install'
  );
  before(
    dockerfile,
    'COPY packages/themes/ packages/themes/',
    'RUN pnpm --filter @panda/web build',
    'production Dockerfile must copy the themes workspace source before the web build'
  );
  forbid(
    read('.github/workflows/runtime.yml'),
    /docker compose[^\n]*\sbuild(?:\s|$)/,
    'runtime workflow must reuse the commit candidate instead of rebuilding the production image'
  );
  before(
    read('.github/workflows/prepare-release.yml'),
    'docker buildx imagetools inspect "$candidate"',
    read('.github/workflows/prepare-release.yml').includes('["git", "push", "origin", version]') ? '["git", "push", "origin", version]' : 'git push origin "$version"',
    'prepare-release.yml must verify the exact main candidate before pushing its release tag'
  );
  forbid(
    read('.github/workflows/fresh-install.yml'),
    /secrets\.FRESH_INSTALL_(?:JWT_SECRET|JWT_PUBLIC_KEY|AGE_ENCRYPTION_KEY)/,
    'fresh-install workflow must validate generated key persistence without repository key secrets'
  );
  return failures;
}

if (isMain(import.meta.url)) {
  const failures = releaseWorkflowFailures(process.cwd());
  if (!failures.length) console.log('Release workflow contract passed.');
  else {
    console.error(
      `Release workflow contract failed with ${failures.length} issue(s):\n${failures.map((message) => `- ${message}`).join('\n')}`
    );
    process.exitCode = 1;
  }
}
