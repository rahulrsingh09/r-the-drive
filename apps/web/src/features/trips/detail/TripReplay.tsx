import React from 'react';
import { Pause, Play } from 'lucide-react';
import { useAuth, useBasemapConfig } from '@panda/hooks';
import { RouteReplayMap } from '@panda/ui/charts';
import { useDocumentTheme } from '@panda/ui/hooks';
import { formatAppTime } from '@panda/ui/lib/dateTime';
import { n0, n1 } from '../activity/activityModel';
import { readingAt, REPLAY_SPEEDS, type ReplayModel } from '../activity/replayModel';
import { useReplayState, type ReplayStore } from './replayStore';
import { Segmented } from '../activity/Segmented';

export interface TripReplayProps {
  model: ReplayModel;
  store: ReplayStore;
  startedAt: Date;
}

/** Map with replay: play or pause, scrubber, 30× / 60× / 240×, and a readout of time, distance, battery and speed. */
export function TripReplay({ model, store, startedAt }: TripReplayProps) {
  const state = useReplayState(store);
  const accessToken = useAuth((auth) => auth.accessToken);
  const basemap = useBasemapConfig();
  const isDark = useDocumentTheme();
  const reading = readingAt(model, state.minute);
  const total = model.totalMinutes;
  React.useEffect(() => () => store.stop(), [store]);

  return (
    <div className="r-act-replay" data-trip-replay>
      <RouteReplayMap
        route={model.route}
        progress={reading.fraction}
        mode={isDark ? 'dark' : 'light'}
        basemapConfig={basemap.data}
        accessToken={accessToken}
        height="auto"
        className="r-act-map"
        label="Trip route map with replay"
      />
      <div className="r-act-rp-bar">
        <button
          type="button"
          className="r-act-rp-play"
          aria-label={state.playing ? 'Pause replay' : 'Play replay'}
          onClick={() => store.toggle()}
          disabled={total <= 0}
          data-replay-play
        >
          {state.playing ? <Pause aria-hidden="true" /> : <Play aria-hidden="true" />}
        </button>
        <input
          type="range"
          min={0}
          max={1000}
          value={total > 0 ? Math.round((state.minute / total) * 1000) : 1000}
          aria-label="Replay position"
          aria-valuetext={`${formatAppTime(startedAt.getTime() + state.minute * 60_000)}, ${n1(reading.miles)} mi`}
          onChange={(event) => { store.stop(); store.seek((Number(event.target.value) / 1000) * total); }}
          disabled={total <= 0}
          data-replay-scrubber
        />
        <Segmented
          small
          className="r-act-rp-speed"
          label="Replay speed"
          value={String(state.speed)}
          options={REPLAY_SPEEDS.map((speed) => ({ value: String(speed), label: `${speed}×` }))}
          onChange={(value) => store.setSpeed(Number(value) as typeof REPLAY_SPEEDS[number])}
        />
      </div>
      <dl className="r-act-rp-read" data-replay-readout>
        <div><dt>Time</dt><dd className="num" data-readout="time">{formatAppTime(startedAt.getTime() + state.minute * 60_000)}</dd></div>
        <div><dt>Distance</dt><dd className="num" data-readout="distance">{n1(reading.miles)} mi</dd></div>
        <div><dt>Battery</dt><dd className="num" data-readout="battery">{reading.battery != null ? `${n0(reading.battery)}%` : '–'}</dd></div>
        <div><dt>Speed</dt><dd className="num" data-readout="speed">{reading.mph != null ? `${n0(reading.mph)} mph` : '–'}</dd></div>
      </dl>
      <p className="r-act-note">
        Replay uses the car&apos;s own timestamped positions. Speed is distance over time. Use two fingers, or Ctrl and scroll, to move the map.
      </p>
    </div>
  );
}
