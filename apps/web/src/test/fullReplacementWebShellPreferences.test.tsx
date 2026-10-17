import { expect, it } from 'vitest';
import { settingsEntryProps } from '../lib/settingsEntryProps';

it('prioritizes recovery-password feedback over SSO linking and callback errors', () => {
  expect(settingsEntryProps({ section: 'account', password: 'set', oidc: 'linked', error: 'oidc_failed' })).toEqual({
    initialSection: 'account', oidcFeedback: 'Recovery password set. Other refresh sessions were revoked.', oidcFeedbackKind: 'success',
  });
  expect(settingsEntryProps({ oidc: 'linked', error: 'oidc_failed' })).toEqual({ oidcFeedback: 'SSO identity linked.', oidcFeedbackKind: 'success' });
  expect(settingsEntryProps({})).toEqual({});
});
