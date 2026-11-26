import React from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api, BASEMAP_CONFIG_QUERY_KEY, queryKeys, useMe } from '@panda/hooks';
import type { ExternalConnectionRecord, UpdateExternalConnectionBody } from '@panda/types';
import { Switch } from '@panda/ui/primitives';

const VISIBLE_IDS: ReadonlyArray<ExternalConnectionRecord['id']> = ['basemap', 'nominatim', 'open_meteo'];

function buildToggleBody(row: ExternalConnectionRecord, nextEnabled: boolean): UpdateExternalConnectionBody {
  const mode = nextEnabled
    ? (row.mode === 'disabled' ? 'remote' : row.mode)
    : 'disabled';
  return {
    enabled: nextEnabled,
    mode,
    ...(row.id === 'basemap' ? { basemap_provider: row.basemap_provider ?? 'openfreemap' } : {}),
    ...(row.id === 'open_meteo' && row.weather_precision ? { weather_precision: row.weather_precision } : {}),
    ...(row.forecast_url ? { forecast_url: row.forecast_url } : {}),
    ...(row.archive_url ? { archive_url: row.archive_url } : {}),
    ...(row.base_url ? { base_url: row.base_url } : {}),
    ...(row.light_url_template ? { light_url_template: row.light_url_template } : {}),
    ...(row.dark_url_template ? { dark_url_template: row.dark_url_template } : {}),
    ...(row.attribution ? { attribution: row.attribution } : {}),
    ...(row.attribution_url ? { attribution_url: row.attribution_url } : {}),
    ...(row.request_identifier ? { request_identifier: row.request_identifier } : {}),
  };
}

/** Owner/admin toggles for server-proxied Maps, Nominatim address lookup, and Open-Meteo weather. */
export function RExternalCard() {
  const me = useMe();
  const canManage = me.data?.role === 'admin' || me.data?.role === 'super_user';
  const queryClient = useQueryClient();

  const connections = useQuery({
    queryKey: queryKeys.externalConnections.all,
    queryFn: () => api.getExternalConnections(),
    enabled: canManage,
    retry: false,
  });

  const toggle = useMutation({
    mutationFn: ({ row, enabled }: { row: ExternalConnectionRecord; enabled: boolean }) =>
      api.updateExternalConnection(row.id, buildToggleBody(row, enabled)),
    onSuccess: (updated) => {
      queryClient.setQueryData(queryKeys.externalConnections.all, updated);
      void queryClient.invalidateQueries({ queryKey: BASEMAP_CONFIG_QUERY_KEY });
    },
  });

  if (!canManage) return null;

  const rows = (connections.data?.connections ?? []).filter((row) => VISIBLE_IDS.includes(row.id));

  return (
    <section id="maps" className="r-card r-pad r-rise" style={{ '--i': 2.5 } as React.CSSProperties} aria-labelledby="settings-maps-title">
      <div className="r-ctitle">
        <h2 id="settings-maps-title">Maps & connected services</h2>
      </div>
      {connections.isLoading ? (
        <p className="r-note" role="status">Loading external services…</p>
      ) : connections.isError ? (
        <p className="r-note">Service settings could not be loaded.</p>
      ) : (
        <div>
          {rows.map((row, idx) => {
            const checked = row.enabled && row.mode !== 'disabled';
            return (
              <div key={row.id} className={`r-trow${idx === 0 ? ' r-trow-top' : ''}`}>
                <div>
                  <div>{row.name}</div>
                  <div className="r-trow-sub">{row.purpose}</div>
                  {row.last_error && checked ? (
                    <div className="r-trow-sub">Last issue: {row.last_error}</div>
                  ) : null}
                </div>
                <Switch
                  checked={checked}
                  size="md"
                  disabled={toggle.isPending}
                  aria-label={row.name}
                  onChange={(next) => toggle.mutate({ row, enabled: next })}
                />
              </div>
            );
          })}
        </div>
      )}
      {toggle.isError ? <p className="r-form-error" role="alert">Could not update service setting.</p> : null}
    </section>
  );
}
