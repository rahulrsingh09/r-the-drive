import { chargingScheduleDraft } from './schedulePresentation';
import { scheduleFields, parseScheduleClock, scheduleNumber, toggleScheduleDay, useScheduleDraft } from './scheduleState';
import React from 'react';
import { Calendar, Clock, Zap, Edit2, Save, X } from 'lucide-react';
import {
  useChargingSchedule,
  useUpdateChargingSchedule,
  useVehicles,
  VEHICLE_CONTROLS_ENABLED,
} from '@panda/hooks';
import type { ChargingScheduleInput } from '@panda/hooks';
import { Button } from '@panda/ui/primitives';
import { registerWidget } from '../../registry';
import type { WidgetCtx, WidgetInstance } from '../../registry';

// ── Helpers ───────────────────────────────────────────────────────────────────

const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const DAY_SHORT = ['Su', 'Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa'];

function timeStrToMinutes(t: string): number | null {
  return parseScheduleClock(t, true);
}

export function formatDays(days: string[] | null): string {
  const length = days?.length ?? 0;
  if (!length) return 'No days set';
  if (length === 7) return 'Every day';
  const members = new Set(days);
  const named = [{ days: DAYS.slice(1, 6), label: 'Weekdays' }, { days: [DAYS[6]!, DAYS[0]!], label: 'Weekends' }];
  const match = named.find((group) => group.days.length === length && group.days.every((day) => members.has(day)));
  return match?.label ?? days!.map((day) => day.slice(0, 3)).join(', ');
}

// ── Form state ────────────────────────────────────────────────────────────────

interface FormState {
  enabled: boolean;
  startTime: string;   // "HH:MM"
  durationHours: string;
  amperage: string;
  weekDays: string[];
}

export function scheduleToForm(s: { enabled: boolean; start_time_minutes: number | null; duration_minutes: number | null; amperage: number | null; week_days: string[] | null }): FormState {
  return chargingScheduleDraft(s);
}

export function formToInput(f: FormState): ChargingScheduleInput {
  const numeric = { start_time_minutes: timeStrToMinutes(f.startTime), duration_minutes: scheduleNumber(f.durationHours, 60, true), amperage: scheduleNumber(f.amperage) };
  return { ...numeric, enabled: f.enabled, week_days: f.weekDays.length ? f.weekDays : null };
}

// ── Component ─────────────────────────────────────────────────────────────────

function ChargingScheduleEditorWidget({
  instance: _instance,
  ctx,
}: {
  instance: WidgetInstance;
  ctx: WidgetCtx;
}) {
  const vehicleId = ctx.vehicleId ?? null;
  const { data: vehicles } = useVehicles();
  const { data: schedule, isLoading } = useChargingSchedule(vehicleId);
  const update = useUpdateChargingSchedule(vehicleId);
  const isDemo = vehicles?.find((vehicle) => vehicle.id === vehicleId)?.is_demo ?? false;

  const draft = useScheduleDraft<FormState>({ enabled: false, startTime: '', durationHours: '', amperage: '', weekDays: [] });
  const { open: editing, form, setForm, show: setEditing } = draft;

  React.useEffect(() => {
    if (schedule) {
      setForm(scheduleToForm(schedule));
    }
  }, [schedule]);

  function toggleDay(day: string) { setForm((form) => ({ ...form, weekDays: toggleScheduleDay(form.weekDays, day) })); }
  async function handleSave() { await update.mutateAsync(formToInput(form)); draft.show(false); }
  function handleCancel() { draft.show(false, schedule ? scheduleToForm(schedule) : undefined); }

  if (isDemo) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center">
        <Calendar className="h-8 w-8 text-fg-tertiary" />
        <p className="text-sm font-medium text-fg">Schedules are read-only for demos</p>
        <p className="text-xs text-fg-tertiary">Demo vehicles include historical examples but do not simulate Rivian schedule changes.</p>
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <span className="text-sm text-fg-tertiary">Loading schedule…</span>
      </div>
    );
  }

  if (!schedule) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
        <Calendar className="h-8 w-8 text-fg-tertiary" />
        <p className="text-sm text-fg-tertiary">No charging schedule configured.</p>
        <p className="text-xs text-fg-tertiary">Read-only telemetry. Change schedules in the Rivian app.</p>
        {VEHICLE_CONTROLS_ENABLED && <Button size="sm" variant="secondary" onClick={() => {
          draft.show(true, { enabled: false, startTime: '22:00', durationHours: '4', amperage: '32', weekDays: DAYS });
        }}>
          Set Up Schedule
        </Button>}
        {VEHICLE_CONTROLS_ENABLED && editing && <ScheduleForm form={form} setForm={setForm} toggleDay={toggleDay} onSave={handleSave} onCancel={handleCancel} saving={update.isPending} />}
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col gap-3 p-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Calendar className="h-4 w-4 text-fg-tertiary" />
          <span className="text-sm font-medium text-fg">Charging Schedule</span>
          <span className={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ${
            schedule.enabled
              ? 'bg-[color:var(--rm-status-positive)]/15 text-[color:var(--rm-status-positive)]'
              : 'bg-bg-elevated text-fg-tertiary'
          }`}>
            {schedule.enabled ? 'Enabled' : 'Disabled'}
          </span>
        </div>
        {VEHICLE_CONTROLS_ENABLED && !editing && (
          <Button size="sm" variant="ghost" iconLeft={<Edit2 className="h-3.5 w-3.5" />} onClick={() => setEditing(true)}>
            Edit
          </Button>
        )}
      </div>

      <p className="text-xs text-fg-tertiary">Read-only telemetry. Change schedules in the Rivian app.</p>
      {VEHICLE_CONTROLS_ENABLED && editing ? (
        <ScheduleForm
          form={form}
          setForm={setForm}
          toggleDay={toggleDay}
          onSave={handleSave}
          onCancel={handleCancel}
          saving={update.isPending}
        />
      ) : (
        <ScheduleReadView schedule={schedule} form={form} />
      )}
    </div>
  );
}

function ScheduleReadView({
  schedule,
  form,
}: {
  schedule: { start_time_minutes: number | null; duration_minutes: number | null; amperage: number | null; week_days: string[] | null };
  form: FormState;
}) {
  const values = chargingScheduleDraft({ ...schedule, enabled: form.enabled });
  const readings = [
    { label: 'Start Time', value: form.startTime || '—', icon: Clock },
    { label: 'Duration', value: values.durationHours ? `${values.durationHours}h` : '—', icon: Clock },
    { label: 'Amperage', value: values.amperage ? `${values.amperage}A` : '—', icon: Zap },
    { label: 'Days', value: formatDays(schedule.week_days), icon: Calendar },
  ];
  return (
    <div className="grid grid-cols-2 gap-2">
      {readings.map(({ label, value, icon: Icon }) => <InfoRow key={label} icon={<Icon className="h-3.5 w-3.5" />} label={label} value={value} />)}
    </div>
  );
}

function InfoRow({ icon, label, value }: { icon: React.ReactNode; label: string; value: string }) {
  return (
    <div className="flex items-start gap-2 rounded-xl border border-border bg-bg-elevated px-3 py-2">
      <span className="mt-0.5 shrink-0 text-fg-tertiary">{icon}</span>
      <div className="min-w-0">
        <p className="text-xs text-fg-tertiary">{label}</p>
        <p className="truncate text-sm font-medium text-fg">{value}</p>
      </div>
    </div>
  );
}

function ScheduleForm({
  form,
  setForm,
  toggleDay,
  onSave,
  onCancel,
  saving,
}: {
  form: FormState;
  setForm: React.Dispatch<React.SetStateAction<FormState>>;
  toggleDay: (day: string) => void;
  onSave: () => void;
  onCancel: () => void;
  saving: boolean;
}) {
  const fields = scheduleFields(setForm);
  return (
    <div className="flex flex-col gap-3">
      {/* Enabled toggle */}
      <label className="flex cursor-pointer items-center gap-3">
        <input
          type="checkbox"
          checked={form.enabled}
          onChange={(e) => fields.field('enabled', e.target.checked)}
          className="h-4 w-4 accent-[color:var(--rm-accent)]"
        />
        <span className="text-sm text-fg">Enable schedule</span>
      </label>

      {/* Start time + duration */}
      <div className="grid grid-cols-2 gap-2">
        <div className="flex flex-col gap-1">
          <label className="text-xs text-fg-tertiary">Start Time</label>
          <input
            type="time"
            value={form.startTime}
            onChange={(e) => fields.field('startTime', e.target.value)}
            className="h-9 w-full rounded-lg border border-border bg-bg-elevated px-3 text-sm text-fg focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
          />
        </div>
        <div className="flex flex-col gap-1">
          <label className="text-xs text-fg-tertiary">Duration (hrs)</label>
          <input
            type="number"
            step="0.5"
            min="0.5"
            max="24"
            value={form.durationHours}
            onChange={(e) => fields.field('durationHours', e.target.value)}
            placeholder="e.g. 4"
            className="h-9 w-full rounded-lg border border-border bg-bg-elevated px-3 text-sm text-fg focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
          />
        </div>
      </div>

      {/* Amperage */}
      <div className="flex flex-col gap-1">
        <label className="text-xs text-fg-tertiary">Amperage (A)</label>
        <input
          type="number"
          step="1"
          min="1"
          max="48"
          value={form.amperage}
          onChange={(e) => fields.field('amperage', e.target.value)}
          placeholder="e.g. 32"
          className="h-9 w-full rounded-lg border border-border bg-bg-elevated px-3 text-sm text-fg focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
        />
      </div>

      {/* Days of week */}
      <div className="flex flex-col gap-1.5">
        <label className="text-xs text-fg-tertiary">Days</label>
        <div className="flex gap-1">
          {DAYS.map((day, i) => (
            <button
              key={day}
              type="button"
              onClick={() => toggleDay(day)}
              className={`flex-1 rounded-lg py-1.5 text-xs font-medium transition-colors ${
                form.weekDays.includes(day)
                  ? 'bg-accent text-fg-on-accent'
                  : 'bg-bg-elevated text-fg-tertiary hover:bg-bg-elevated/80'
              }`}
            >
              {DAY_SHORT[i]}
            </button>
          ))}
        </div>
      </div>

      {/* Actions */}
      <div className="flex gap-2">
        <Button
          size="sm"
          variant="primary"
          iconLeft={<Save className="h-3.5 w-3.5" />}
          loading={saving}
          onClick={onSave}
          className="flex-1"
        >
          Save
        </Button>
        <Button
          size="sm"
          variant="ghost"
          iconLeft={<X className="h-3.5 w-3.5" />}
          disabled={saving}
          onClick={onCancel}
        >
          Cancel
        </Button>
      </div>
    </div>
  );
}

// ── Registry ──────────────────────────────────────────────────────────────────

registerWidget({
  componentType: 'custom',
  definitionId: 'charging.schedule.editor',
  title: 'Charging Schedule',
  defaultSize: { w: 5, h: 8 },
  minSize: { w: 4, h: 6 },
  defaultOptions: {},
  component: ChargingScheduleEditorWidget,
});
