import { departurePresentation } from './schedulePresentation';
import { scheduleFields, parseScheduleClock, scheduleNumber, toggleScheduleDay, useScheduleDraft } from './scheduleState';
import React from 'react';
import { AlarmClock, Plus, Trash2, ChevronDown, ChevronUp, Thermometer, Wind } from 'lucide-react';
import {
  useDepartureSchedules,
  useCreateDepartureSchedule,
  useUpdateDepartureSchedule,
  useDeleteDepartureSchedule,
  useVehicles,
  VEHICLE_CONTROLS_ENABLED,
} from '@panda/hooks';
import type { DepartureSchedule, DepartureScheduleInput, DepartureComfortSettings } from '@panda/hooks';
import { Button, SelectPicker } from '@panda/ui/primitives';
import { registerWidget } from '../../registry';
import type { WidgetCtx, WidgetInstance } from '../../registry';

// ── Constants ────────────────────────────────────────────────────────────────

const DAYS_FULL = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const DAYS_SHORT = ['Su', 'Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa'];
const HEAT_LABELS = ['Off', 'Low', 'Med', 'High'];

// ── Helpers ───────────────────────────────────────────────────────────────────

function timeToMinutes(t: string): number {
  return parseScheduleClock(t, false)!;
}

export function formatOccurrence(sched: DepartureSchedule): string {
  return departurePresentation(sched).occurrence;
}

// ── New schedule form state ────────────────────────────────────────────────────

interface NewForm {
  name: string;
  enabled: boolean;
  days: string[];
  time: string;
  seatFlHeat: number;
  seatFrHeat: number;
  cabinTempC: string;
  defrost: boolean;
}

function emptyForm(): NewForm {
  return {
    name: '',
    enabled: true,
    days: ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday'],
    time: '07:30',
    seatFlHeat: 0,
    seatFrHeat: 0,
    cabinTempC: '',
    defrost: false,
  };
}

export function formToInput(f: NewForm): DepartureScheduleInput {
  const comfortSettings: DepartureComfortSettings = {};
  const heat = { seat_fl_heat: f.seatFlHeat, seat_fr_heat: f.seatFrHeat };
  for (const key of ['seat_fl_heat', 'seat_fr_heat'] as const) if (heat[key] > 0) comfortSettings[key] = heat[key];
  const cabin = scheduleNumber(f.cabinTempC);
  if (cabin !== null) comfortSettings.cabin_temp_c = cabin;
  if (f.defrost) comfortSettings.defrost = true;
  const occurrence = { type: 'RepeatsWeekly' as const, days: f.days, time_minutes: timeToMinutes(f.time) };
  return { name: f.name || null, enabled: f.enabled, occurrence, comfort_settings: Object.keys(comfortSettings).length ? comfortSettings : null };
}

// ── Sub-components ───────────────────────────────────────────────────────────

function ScheduleCard({
  schedule,
  onToggle,
  onDelete,
  toggling,
  deleting,
}: {
  schedule: DepartureSchedule;
  onToggle: () => void;
  onDelete: () => void;
  toggling: boolean;
  deleting: boolean;
}) {
  const [expanded, setExpanded] = React.useState(false);
  const { hasComfort, pills, selectedDays } = departurePresentation(schedule);

  return (
    <div className="rounded-xl border border-border bg-bg-elevated overflow-hidden">
      {/* Main row */}
      <div className="flex items-center gap-3 px-3 py-2.5">
        {/* Enabled toggle */}
        <button
          type="button"
          onClick={onToggle}
          disabled={!VEHICLE_CONTROLS_ENABLED || toggling}
          aria-label={VEHICLE_CONTROLS_ENABLED ? (schedule.enabled ? 'Disable schedule' : 'Enable schedule') : (schedule.enabled ? 'Schedule enabled (read-only)' : 'Schedule disabled (read-only)')}
          className={`relative h-5 w-9 shrink-0 rounded-full transition-colors ${
            schedule.enabled ? 'bg-accent' : 'bg-border'
          } ${toggling ? 'opacity-50' : ''}`}
        >
          <span
            className={`absolute top-0.5 h-4 w-4 rounded-full bg-bg-surface shadow transition-transform ${
              schedule.enabled ? 'translate-x-4' : 'translate-x-0.5'
            }`}
          />
        </button>

        {/* Info */}
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium text-fg">
            {schedule.name || 'Departure Schedule'}
          </p>
          <p className="truncate text-xs text-fg-tertiary">{formatOccurrence(schedule)}</p>
        </div>

        {/* Actions */}
        <div className="flex shrink-0 items-center gap-1">
          {hasComfort && (
            <button
              type="button"
              onClick={() => setExpanded((e) => !e)}
              className="rounded-lg p-1.5 text-fg-tertiary hover:bg-bg-page hover:text-fg"
              aria-label="Show comfort settings"
            >
              {expanded ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
            </button>
          )}
          {VEHICLE_CONTROLS_ENABLED && <button
            type="button"
            onClick={onDelete}
            disabled={deleting}
            className="rounded-lg p-1.5 text-fg-tertiary hover:bg-status-danger/10 hover:text-status-danger disabled:opacity-40"
            aria-label="Delete schedule"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>}
        </div>
      </div>

      {/* Days pills */}
      {schedule.occurrence?.days && (
        <div className="flex gap-1 px-3 pb-2.5">
          {DAYS_FULL.map((day, i) => (
            <span
              key={day}
              className={`flex h-5 w-6 items-center justify-center rounded text-[10px] font-medium ${
                selectedDays.has(day)
                  ? 'bg-accent/20 text-accent'
                  : 'bg-bg-page text-fg-tertiary'
              }`}
            >
              {DAYS_SHORT[i]}
            </span>
          ))}
        </div>
      )}

      {/* Comfort settings */}
      {expanded && hasComfort && (
        <div className="flex flex-wrap gap-2 border-t border-border px-3 py-2.5">
          {pills.map((pill) => <ComfortPill
            key={pill.label}
            icon={pill.kind === 'wind' ? <Wind className="h-3 w-3" /> : <Thermometer className="h-3 w-3" />}
            label={pill.label}
          />)}
        </div>
      )}
    </div>
  );
}

function ComfortPill({ icon, label }: { icon: React.ReactNode; label: string }) {
  return (
    <span className="flex items-center gap-1 rounded-full bg-bg-page px-2 py-0.5 text-xs text-fg-secondary">
      {icon}
      {label}
    </span>
  );
}

function NewScheduleForm({
  form,
  setForm,
  onSave,
  onCancel,
  saving,
}: {
  form: NewForm;
  setForm: React.Dispatch<React.SetStateAction<NewForm>>;
  onSave: () => void;
  onCancel: () => void;
  saving: boolean;
}) {
  function toggleDay(day: string) {
    setForm((form) => ({ ...form, days: toggleScheduleDay(form.days, day) }));
}

  const fields = scheduleFields(setForm);
  return (
    <div className="flex flex-col gap-3 rounded-xl border border-accent/30 bg-bg-elevated p-3">
      <p className="text-sm font-medium text-fg">New Departure Schedule</p>

      {/* Name */}
      <div className="flex flex-col gap-1">
        <label className="text-xs text-fg-tertiary">Name (optional)</label>
        <input
          type="text"
          value={form.name}
          onChange={(e) => fields.field('name', e.target.value)}
          placeholder="e.g. Work commute"
          className="h-9 w-full rounded-lg border border-border bg-bg-page px-3 text-sm text-fg focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
        />
      </div>

      {/* Time */}
      <div className="flex flex-col gap-1">
        <label className="text-xs text-fg-tertiary">Departure Time</label>
        <input
          type="time"
          value={form.time}
          onChange={(e) => fields.field('time', e.target.value)}
          className="h-9 w-full rounded-lg border border-border bg-bg-page px-3 text-sm text-fg focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
        />
      </div>

      {/* Days */}
      <div className="flex flex-col gap-1.5">
        <label className="text-xs text-fg-tertiary">Days</label>
        <div className="flex gap-1">
          {DAYS_FULL.map((day, i) => (
            <button
              key={day}
              type="button"
              onClick={() => toggleDay(day)}
              className={`flex-1 rounded-lg py-1.5 text-xs font-medium transition-colors ${
                form.days.includes(day)
                  ? 'bg-accent text-fg-on-accent'
                  : 'bg-bg-page text-fg-tertiary hover:bg-bg-page/80'
              }`}
            >
              {DAYS_SHORT[i]}
            </button>
          ))}
        </div>
      </div>

      {/* Comfort settings */}
      <div className="flex flex-col gap-2">
        <label className="text-xs text-fg-tertiary">Comfort (optional)</label>
        <div className="grid grid-cols-2 gap-2">
          <HeatSelect
            label="FL Seat Heat"
            value={form.seatFlHeat}
            onChange={(v) => fields.field('seatFlHeat', v)}
          />
          <HeatSelect
            label="FR Seat Heat"
            value={form.seatFrHeat}
            onChange={(v) => fields.field('seatFrHeat', v)}
          />
        </div>
        <div className="grid grid-cols-2 gap-2">
          <div className="flex flex-col gap-1">
            <label className="text-xs text-fg-tertiary">Cabin Temp (°C)</label>
            <input
              type="number"
              step="0.5"
              min="15"
              max="32"
              value={form.cabinTempC}
              onChange={(e) => fields.field('cabinTempC', e.target.value)}
              placeholder="e.g. 21"
              className="h-8 w-full rounded-lg border border-border bg-bg-page px-3 text-sm text-fg focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
          </div>
          <label className="flex cursor-pointer items-center gap-2 self-end pb-0.5">
            <input
              type="checkbox"
              checked={form.defrost}
              onChange={(e) => fields.field('defrost', e.target.checked)}
              className="h-4 w-4 accent-[color:var(--rm-accent)]"
            />
            <span className="text-sm text-fg">Defrost</span>
          </label>
        </div>
      </div>

      {/* Actions */}
      <div className="flex gap-2">
        <Button size="sm" variant="primary" loading={saving} onClick={onSave} className="flex-1">
          Create
        </Button>
        <Button size="sm" variant="ghost" disabled={saving} onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

function HeatSelect({ label, value, onChange }: { label: string; value: number; onChange: (v: number) => void }) {
  return (
    <div className="flex flex-col gap-1">
      <label className="text-xs text-fg-tertiary">{label}</label>
      <SelectPicker
        className="w-full"
        value={String(value)}
        onChange={(nextValue) => onChange(Number(nextValue))}
        aria-label={label}
        options={HEAT_LABELS.map((heatLabel, index) => ({ value: String(index), label: heatLabel }))}
      />
    </div>
  );
}

// ── Main widget ───────────────────────────────────────────────────────────────

function DepartureSchedulesWidget({
  instance: _instance,
  ctx,
}: {
  instance: WidgetInstance;
  ctx: WidgetCtx;
}) {
  const vehicleId = ctx.vehicleId ?? null;
  const { data: vehicles } = useVehicles();
  const { data: schedules = [], isLoading } = useDepartureSchedules(vehicleId);
  const createSchedule = useCreateDepartureSchedule(vehicleId);
  const updateSchedule = useUpdateDepartureSchedule(vehicleId);
  const deleteSchedule = useDeleteDepartureSchedule(vehicleId);
  const isDemo = vehicles?.find((vehicle) => vehicle.id === vehicleId)?.is_demo ?? false;

  const draft = useScheduleDraft(emptyForm());
  const { open: showCreate, form: newForm, setForm: setNewForm, show: setShowCreate } = draft;
  const deletingId = draft.pending.delete ?? null, togglingId = draft.pending.toggle ?? null;
  async function handleCreate() { await createSchedule.mutateAsync(formToInput(newForm)); draft.show(false, emptyForm()); }
  function handleToggle(schedule: DepartureSchedule) {
    return draft.during('toggle', schedule.id, () => updateSchedule.mutateAsync({
      scheduleId: schedule.rivian_schedule_id,
      body: { name: schedule.name, enabled: !schedule.enabled, occurrence: schedule.occurrence, comfort_settings: schedule.comfort_settings },
    }));
  }
  function handleDelete(schedule: DepartureSchedule) { return draft.during('delete', schedule.id, () => deleteSchedule.mutateAsync(schedule.rivian_schedule_id)); }

  if (isDemo) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center">
        <AlarmClock className="h-8 w-8 text-fg-tertiary" />
        <p className="text-sm font-medium text-fg">Schedules are read-only for demos</p>
        <p className="text-xs text-fg-tertiary">Demo vehicles include historical examples but do not simulate Rivian schedule changes.</p>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col gap-3 overflow-y-auto p-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <AlarmClock className="h-4 w-4 text-fg-tertiary" />
          <span className="text-sm font-medium text-fg">Departure Schedules</span>
          {!isLoading && (
            <span className="rounded-full bg-bg-elevated px-2 py-0.5 text-xs text-fg-tertiary">
              {schedules.length}
            </span>
          )}
        </div>
        {VEHICLE_CONTROLS_ENABLED && !showCreate && (
          <Button
            size="sm"
            variant="secondary"
            iconLeft={<Plus className="h-3.5 w-3.5" />}
            onClick={() => { draft.show(true, emptyForm()); }}
          >
            New
          </Button>
        )}
      </div>

      {/* Create form */}
      {VEHICLE_CONTROLS_ENABLED && showCreate && (
        <NewScheduleForm
          form={newForm}
          setForm={setNewForm}
          onSave={handleCreate}
          onCancel={() => setShowCreate(false)}
          saving={createSchedule.isPending}
        />
      )}

      <p className="text-xs text-fg-tertiary">Read-only telemetry. Change schedules in the Rivian app.</p>
      {/* List */}
      {isLoading ? (
        <div className="flex flex-1 items-center justify-center">
          <span className="text-sm text-fg-tertiary">Loading schedules…</span>
        </div>
      ) : schedules.length === 0 && !showCreate ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 text-center">
          <AlarmClock className="h-8 w-8 text-fg-tertiary" />
          <p className="text-sm text-fg-tertiary">No departure schedules yet.</p>
          <p className="text-xs text-fg-tertiary">Schedules from Rivian appear here after telemetry collection.</p>
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {schedules.map((sched) => (
            <ScheduleCard
              key={sched.id}
              schedule={sched}
              onToggle={() => handleToggle(sched)}
              onDelete={() => handleDelete(sched)}
              toggling={togglingId === sched.id}
              deleting={deletingId === sched.id}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ── Registry ──────────────────────────────────────────────────────────────────

registerWidget({
  componentType: 'custom',
  definitionId: 'charging.departure.schedules',
  title: 'Departure Schedules',
  defaultSize: { w: 5, h: 10 },
  minSize: { w: 4, h: 6 },
  defaultOptions: {},
  component: DepartureSchedulesWidget,
});
