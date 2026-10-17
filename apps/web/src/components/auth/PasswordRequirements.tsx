import { Check, Circle } from 'lucide-react';

export const PASSWORD_MIN_LENGTH = 12;

const PASSWORD_PROGRESS = [
  { Icon: Circle, className: 'border-border bg-bg-elevated/50 text-fg-tertiary' },
  { Icon: Circle, className: 'border-status-danger/30 bg-status-danger/10 text-status-danger' },
  { Icon: Check, className: 'border-status-positive/30 bg-status-positive/10 text-status-positive' },
] as const;

export function PasswordRequirements({ password }: { password: string }) {
  const count = password.length;
  const progress = Number(count > 0) + Number(count >= PASSWORD_MIN_LENGTH);
  const { Icon, className } = PASSWORD_PROGRESS[progress]!;

  return (
    <div
      className={`flex items-center justify-between gap-3 rounded-lg border px-3 py-2 text-xs ${className}`}
      role="status"
      aria-label={`Password requires at least ${PASSWORD_MIN_LENGTH} characters. ${count} entered.`}
    >
      <span className="flex items-center gap-2">
        <Icon className="h-3.5 w-3.5" aria-hidden="true" />
        At least {PASSWORD_MIN_LENGTH} characters
      </span>
      <span className="shrink-0 tabular-nums">{count}/{PASSWORD_MIN_LENGTH}</span>
    </div>
  );
}
