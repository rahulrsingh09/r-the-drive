# Repository privacy review

Reviewed locally on 2026-10-16.

The initial scaffold contained personal GitHub references in source, documentation, workflow templates, CODEOWNERS and an account-specific backup receiver. Its two initial commits also contained personal author/committer metadata. No personal account email was found in the source. These identity references were removed from the publication scaffold, which is exported into a fresh repository with neutral author metadata and no remote.

The review covers the complete tracked source inventory, filenames, binary asset bytes and Git history. Known personal emails, usernames and the maintainer name are checked case-insensitively, including UTF-16, Base64 and percent-encoded forms. Gitleaks scans both the source tree and all reachable commits with redacted output. Its retained exceptions are restricted to synthetic test fixtures.

No live passwords, API tokens or private keys were detected. Test credentials and example account addresses remain intentionally synthetic. Generated `.env` files, deployment state, databases, logs, backups and private key files are excluded from the publication snapshot. License text and third-party attribution remain intact.

Repository links use an unconfigured placeholder; repository/workflow IDs are zero. CODEOWNERS requires a real owning team before enforcement. The backup receiver uses a non-routable `.invalid` address until a new receiver is configured. Hosted deployment and release automation remain unconfigured.

Old project branding is absent from filenames and literal source/asset bytes. The bootstrap check retains character-code definitions solely to detect reintroduced identifiers. Two inherited geocoding User-Agent defaults were changed to a neutral application identifier. Upstream implementation credit, technical references to other maintainers and third-party license text remain intentionally; they are not account credentials or configured access grants.

This is a credential and personal-identity review, not a penetration test or proof that an unknown password can never occur. It does not certify unrelated repositories, local account configuration, earlier clones, GitHub retention or production databases. Repeat secret scanning before publication and after any identity or deployment configuration changes.
