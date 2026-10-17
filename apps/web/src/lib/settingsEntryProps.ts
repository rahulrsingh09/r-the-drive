import type { ComponentProps } from 'react';
import type { SettingsContent } from '../features/settings/SettingsPage';

type SettingsProps = NonNullable<ComponentProps<typeof SettingsContent>>;

export function settingsEntryProps(search: {
  section?: SettingsProps['initialSection'];
  password?: string | undefined;
  oidc?: string | undefined;
  error?: string | undefined;
}): SettingsProps {
  const props: SettingsProps = {};
  if (search.section) props.initialSection = search.section;
  const feedback: Array<[boolean, string, 'success' | 'error']> = [
    [Boolean(search.password), 'Recovery password set. Other refresh sessions were revoked.', 'success'],
    [Boolean(search.oidc), 'SSO identity linked.', 'success'],
    [Boolean(search.error), 'SSO connection was cancelled or could not be completed.', 'error'],
  ];
  const selected = feedback.find(([enabled]) => enabled);
  if (selected) {
    props.oidcFeedback = selected[1];
    props.oidcFeedbackKind = selected[2];
  }
  return props;
}
