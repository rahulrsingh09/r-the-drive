import Link from '@docusaurus/Link';
import Layout from '@theme/Layout';
import useBaseUrl from '@docusaurus/useBaseUrl';
import type {ReactElement} from 'react';
import styles from './index.module.css';

const entryPoints = [
  {
    eyebrow: 'Self-hosters',
    title: 'Install R:The Drive',
    description: 'Move from host prerequisites through a verified, private Compose installation.',
    to: '/docs/getting-started/',
  },
  {
    eyebrow: 'Owners and operators',
    title: 'Use and operate R:The Drive',
    description: 'Customize the product, manage integrations, update safely, and stay recoverable.',
    to: '/docs/using-panda/',
  },
  {
    eyebrow: 'Contributors',
    title: 'Understand the system',
    description: 'Review architecture, implementation guidance, governance, and exact references.',
    to: '/docs/development/',
  },
];

const quickLinks = [
  ['Features', '/docs/overview/features/'],
  ['Dashboard customization', '/docs/using-panda/dashboard-customization/'],
  ['Operations', '/docs/operations/'],
  ['Backup and restore', '/docs/operations/backup-and-restore/'],
  ['Privacy', '/docs/privacy/'],
  ['Reference', '/docs/reference/'],
];

export default function Home(): ReactElement {
  const darkLogo = useBaseUrl('/logo-lockup-dark.png');
  const lightLogo = useBaseUrl('/logo-lockup-light.png');
  const darkOverview = useBaseUrl('/overview-desktop-dark.png');
  const lightOverview = useBaseUrl('/overview-desktop-light.png');

  return (
    <Layout
      title="Welcome"
      description="The complete guide to installing, operating, understanding, and contributing to R:The Drive."
    >
      <main>
        <section className={styles.hero}>
          <div className={styles.heroGlow} aria-hidden="true" />
          <div className={styles.heroCopy}>
            <picture>
              <source media="(prefers-color-scheme: dark)" srcSet={darkLogo} />
              <img className={styles.logo} src={lightLogo} alt="R:The Drive" />
            </picture>
            <p className={styles.eyebrow}>Private Rivian telemetry, on hardware you control</p>
            <h1>R:The Drive</h1>
            <p className={styles.lede}>
              One source for installing R:The Drive, operating it safely, and reviewing how the system works.
            </p>
            <div className={styles.actions}>
              <Link className="button button--primary button--lg" to="/docs/getting-started/">
                Get started
              </Link>
              <Link className="button button--secondary button--lg" to="/docs/development/">
                Development docs
              </Link>
            </div>
          </div>
          <div className={styles.heroVisual}>
            <picture>
              <source media="(prefers-color-scheme: dark)" srcSet={darkOverview} />
              <img
                src={lightOverview}
                alt="R:The Drive vehicle overview dashboard showing battery, range, charging, and vehicle status"
              />
            </picture>
          </div>
        </section>

        <section className={styles.pathSection} aria-labelledby="choose-path">
          <div className={styles.sectionHeading}>
            <p className={styles.eyebrow}>Documentation paths</p>
            <h2 id="choose-path">Start with what you need to accomplish</h2>
          </div>
          <div className={styles.cardGrid}>
            {entryPoints.map((entry) => (
              <Link className={styles.pathCard} to={entry.to} key={entry.title}>
                <span className={styles.cardEyebrow}>{entry.eyebrow}</span>
                <h3>{entry.title}</h3>
                <p>{entry.description}</p>
                <span className={styles.cardLink}>Open guide <span aria-hidden="true">→</span></span>
              </Link>
            ))}
          </div>
        </section>

        <section className={styles.quickSection} aria-labelledby="quick-reference">
          <div>
            <p className={styles.eyebrow}>Quick reference</p>
            <h2 id="quick-reference">Everything remains reviewable in the repository</h2>
            <p>
              The documentation is built directly from <code>docs/</code>. Run
              <code> pnpm docs:build</code> to build it locally.
            </p>
          </div>
          <nav className={styles.quickLinks} aria-label="Popular documentation">
            {quickLinks.map(([label, to]) => (
              <Link key={to} to={to}>
                {label}<span aria-hidden="true">→</span>
              </Link>
            ))}
          </nav>
        </section>
      </main>
    </Layout>
  );
}
