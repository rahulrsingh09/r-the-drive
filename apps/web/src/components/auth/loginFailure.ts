const STATUS_MESSAGES: Record<number, string> = {
  401: 'Incorrect email or password. Please try again.',
  429: 'Too many sign-in attempts. Please wait a moment and try again.',
};
const SETUP_MESSAGES: Record<string, string> = {
  SETUP_PROOF_REQUIRED: 'Enter the instance setup token before creating the owner account.',
  SETUP_PROOF_INVALID: 'The instance setup token is invalid. Check it and try again.',
};

export function loginFailureMessage(cause: unknown): string {
  const failure = cause as { status?: number; code?: string; detail?: { message?: string } };
  const status = failure.status;
  if (status === 401 || status === 429) return STATUS_MESSAGES[status]!;
  if (failure.code && Object.hasOwn(SETUP_MESSAGES, failure.code)) return SETUP_MESSAGES[failure.code]!;
  if (status === 422) return failure.detail?.message ?? 'Check the password requirements and try again.';
  return status != null && status >= 500
    ? 'Something went wrong on our end. Please try again later.'
    : 'Unable to sign in. Please check your connection and try again.';
}
