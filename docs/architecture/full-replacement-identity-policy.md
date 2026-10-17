# Identity policy implementation replacement

Documentation impact: internal architecture update. The public API, key formats,
OIDC transaction fields and database schema remain the same. This work does not
claim clean-room or independent authorship; existing GPL attribution still applies.

The API-key routes now use typed persisted records and a single issuance operation.
Creation and rotation write the key and its audit event in one transaction; revocation
uses an owned conditional UPDATE RETURNING so missing, foreign and revoked IDs all
return 404. Rotation keeps the 24-hour overlap and 90-day successor lifetime. A
separate regression fix restores whitespace in the previously malformed rotation SQL.

User administration constructs explicit invitation and membership operations.
Invitation creation keeps its email advisory lock, validates every vehicle and inserts
the selected vehicle set atomically. User updates/deletion serialize their count checks;
membership mutations lock the vehicle before owner checks and commit related writes
together. Disable revokes sessions and API keys in the user transaction. Deletion locks
the account and retains ordered preference/theme cleanup, transaction-local theme guard,
Redis staging cleanup and all vehicle/history data. Typed row projections preserve
chrono timestamp serialization and the existing enumeration/role restrictions.

Authentication settings now resolve public and internal views through the same typed
source-selection functions. Updates lock the singleton, normalize a proposed snapshot,
apply environment ownership and lockout policy, then encrypt and persist the change.
Disabling password login still requires enabled, tested effective OIDC configuration,
an exact keyed validation fingerprint and a linked enabled super user. Secrets never
appear in the public settings response. No environment override or outbound allowlist
has been weakened.

OIDC exchange chooses the client authentication mode once, then performs one library
exchange and verification path. The vetted library still validates signature, issuer,
audience, expiry and nonce. UserInfo remains optional, bound to the verified subject,
and can only fill missing custom claims. Discovery, JWKS, token and UserInfo requests
retain HTTPS, private-origin/DNS-pinning, no-redirect and bounded-response enforcement.
Return targets and callback URL contracts, browser binding and PKCE remain unchanged.

## Validation

The unchanged ordinary library suite passes 682 tests. Four existing ignored user
DB authorization tests pass on disposable TimescaleDB/Redis. Five added tests cover
key lifecycle/non-enumeration, admin restrictions/invitations/memberships, settings
lockout and secret storage, rotation/audit rollback and a real TLS OIDC stub. The stub
covers discovery errors, response bounds, PKCE, all three client authentication modes,
signed-token success and signature/nonce/audience/issuer/expiry rejection. Fixtures use
synthetic identities on an unpublished internal network; no live providers are used.
The rotation test failed with 500 on the baseline, then passed with 201 after its
separate SQL fix. Both database history-guard tests and the existing invitation
acceptance/theme-owner deletion integration tests also pass (13 focused checks in
total). The application build and strict Clippy for all targets/features pass.
All original assertions and goldens are preserved. The root integration must register
this page in the central sidebar before the documentation-site checks can pass.

## Complete source-to-replacement map

The map includes every production function in the four assigned files. Minimal
contract bodies are explicitly reported below, not counted as new algorithms. Nested
row closures, field macros and the OIDC verification macro are replaced within the
listed workflows; the executable endpoint factory and duplicate audit helper are removed.

| File | Original function | Replacement or exact contract case |
| --- | --- | --- |
| api_keys.rs | `router` | Immutable route/method table folded into the same Axum Router. |
| api_keys.rs | `as_str` | Fixed one-variant public enum maps Read to the wire literal read. |
| api_keys.rs | `try_from` | Exactly one accepted wire literal read; all others produce the fixed validation error. |
| api_keys.rs | `list_api_keys` | StoredKey typed decoding and response projection. |
| api_keys.rs | `create_api_key` | Validated KeyIssuance, one persistence operation, audit and commit. |
| api_keys.rs | `revoke_api_key` | Owned conditional UPDATE RETURNING followed by transactional audit. |
| api_keys.rs | `rotate_api_key` | Locked predecessor snapshot, overlap update, shared KeyIssuance and audit. |
| api_keys.rs | `api_catalog` | Single mandatory Json(catalog()) adapter; catalog itself now projects immutable entries. |
| api_keys.rs | `admin_api_catalog` | Minimal ordered session-only and admin gates followed by the same catalog response; gate implementation is replaced. |
| api_keys.rs | `catalog` | 36 immutable endpoint records; no executable endpoint factory. |
| api_keys.rs | `api_access_level_state` | Shared access-level parser plus legacy-level set classification. |
| api_keys.rs | `api_key_expiry_state` | Fixed three-case projection: absent expiry, deadline at/before now, future deadline; revoked_at deliberately does not alter this wire field. |
| api_keys.rs | `endpoint` | Removed; immutable endpoint records carry the exact catalog contract. |
| api_keys.rs | `generate_api_key` | Unchanged vetted RNG operation: 32 random bytes, base64url without padding, panda_ prefix. |
| api_keys.rs | `hash_api_key` | Persistent API key digest contract is SHA-256 of exact UTF-8 bytes. |
| api_keys.rs | `require_user_admin` | Database EXISTS policy; no role payload enumeration. |
| api_keys.rs | `require_session_auth` | Minimal capability denial: any api_access_level forbids the operation. |
| users.rs | `router` | Immutable route/method table folded into the same Axum Router. |
| users.rs | `list_users` | Typed ListedUser and correlated membership count; normalized search input. |
| users.rs | `list_admin_vehicle_options` | Database projection to fixed JSON response fields. |
| users.rs | `create_account_invitation` | InvitationPlan, serialized email guard, set insertion and explicit constraint errors. |
| users.rs | `list_account_invitations` | Independent ordered relation arrays in typed invitation payload. |
| users.rs | `revoke_account_invitation` | Shared support mutation completion and guarded SQL update. |
| users.rs | `random_invitation_token` | Distribution sample_string preserves 48-character alphanumeric contract. |
| users.rs | `hash_token` | Invitation digest contract is SHA-256 of exact UTF-8 token bytes. |
| users.rs | `audit_log` | Removed; support_audit supplies the single spawned audit path. |
| users.rs | `update_user` | Parsed user update, serialized administrator changes, atomic session/key revocation. |
| users.rs | `delete_user` | Locked account snapshot and ordered transactional cleanup statements; history untouched. |
| users.rs | `get_user_detail` | Typed user lookup plus separate membership/invitation projections. |
| users.rs | `list_user_vehicle_memberships` | Authorized typed payload adapter. |
| users.rs | `list_user_invites` | Typed user lookup and invitation projection. |
| users.rs | `revoke_user_invite` | Typed user identity lookup and shared support mutation completion. |
| users.rs | `grant_user_vehicle_membership` | Grant command through serialized membership transaction. |
| users.rs | `update_user_vehicle_membership` | Update command through serialized membership transaction. |
| users.rs | `remove_user_vehicle_membership` | Remove command with locked vehicle and owner-count guard. |
| users.rs | `list_user_memberships_payload` | FromRow/Serialize payload keeps chrono wire timestamp formatting. |
| users.rs | `list_user_invites_payload` | FromRow/Serialize payload keeps chrono wire timestamp formatting. |
| users.rs | `support_audit` | Single constructed audit event submitted in a background recording task. |
| authentication_settings.rs | `load_effective` | Stored snapshot -> validated env -> unified resolution. |
| authentication_settings.rs | `load` | Stored snapshot -> unified source-aware public response -> claim-pair check. |
| authentication_settings.rs | `update` | Locked settings snapshot -> proposed state -> effective policy -> encrypted secret -> persistence. |
| authentication_settings.rs | `has_linked_super_user_for_issuer` | Nested EXISTS query through caller executor, including update transaction. |
| authentication_settings.rs | `reject_environment_owned_update` | Explicit exhaustive field ownership gates; minimal policy conjunction per field. |
| authentication_settings.rs | `record_validation` | Minimal adapter persists same timestamp and fixed fingerprint. |
| authentication_settings.rs | `validation_fingerprint` | Persisted HMAC-SHA256 marker includes installation key and exact v2 formatting/field order; changing bytes invalidates stored provider validation. |
| authentication_settings.rs | `validate_effective` | Enabled/disabled branch with credential completeness and shared policy validators. |
| authentication_settings.rs | `oidc_callback_url` | Exact callback construction and parsed component admission. |
| authentication_settings.rs | `validate_required_claim_pair` | Presence equivalence after trimming optional pair. |
| authentication_settings.rs | `normalize_update` | In-place optional patch normalization, ordered domain set, shared scope parsing. |
| authentication_settings.rs | `normalize_optional_update` | Explicit no-change, clear, and trimmed-value branches. |
| authentication_settings.rs | `canonicalize_issuer` | Canonical identity key is trim whitespace then remove trailing slashes. |
| authentication_settings.rs | `normalize_oidc_scopes` | Separate ordered output and membership set; byte grammar rejects quote/backslash/non-ASCII. |
| authentication_settings.rs | `load_stored` | Typed row decoding and issuer identity normalization. |
| authentication_settings.rs | `effective` | Typed setting and optional_setting functions replace duplicated executable macros. |
| oidc.rs | `call` | Trait adapter into bounded authenticated-origin HTTP transport method. |
| oidc.rs | `https_only_client` | Minimal operator-security construction adapter. |
| oidc.rs | `random_token` | Vetted alphanumeric RNG operation remains exactly 48 characters. |
| oidc.rs | `browser_binding` | Cookie binding wire/storage contract is hexadecimal SHA-256 of cookie bytes. |
| oidc.rs | `pkce_challenge` | RFC PKCE S256 is base64url without padding of SHA-256 verifier bytes. |
| oidc.rs | `validate_return_to` | Forbidden-input admission then URL canonical path/query/fragment slice. |
| oidc.rs | `discover` | Configured issuer, pinned outbound discovery, endpoint table and PKCE validation. |
| oidc.rs | `validate_https_provider_endpoint` | Parsed URL component admission policy. |
| oidc.rs | `callback_url` | Settings callback policy followed by typed redirect conversion. |
| oidc.rs | `validate_client_configuration` | Credential field validation table, shared scopes and method allowlist. |
| oidc.rs | `secure_metadata` | Vetted library discovery/JWKS with shared issuer validation and endpoint policy. |
| oidc.rs | `provider_configuration_ready` | Network-free composed issuer, callback and client checks. |
| oidc.rs | `exchange_and_verify` | One configured client and exchange pipeline; library verification precedes subject-bound UserInfo. |
| oidc.rs | `merge_missing_claims` | Fallback map receives primary ID-token map; primary claims win. |
| oidc.rs | `test_provider` | Callback check then secure discovery and client policy. |
| oidc.rs | `claim_matches` | Validated optional pair and scalar/array membership projection. |
| oidc.rs | `domain_allowed` | Explicit unrestricted/domain-present/domain-absent cases. |
| oidc.rs | `authorization_url` | Fixed protocol parameter table with S256 challenge appended in original order. |
