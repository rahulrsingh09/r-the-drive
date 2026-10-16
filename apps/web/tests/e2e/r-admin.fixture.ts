// Synthetic API responses; no live vehicle or account is used.
import type { ApiKeyRecord, BackupOverview, ExternalConnectionsResponse } from '@panda/types';

type Reply = (body: unknown, status?: number, headers?: Record<string,string>) => Promise<unknown>;
export function createAdminFixture(vehicleId: string) {
  const now = new Date(Date.now() - 60_000).toISOString();
  let backfill = 'done';
  let capture = false;
  let captured = false;
  const captureState = () => ({state:capture?'capturing':captured?'stopped':'idle',started_at:captured?now:null,ends_at:captured?new Date(Date.now()+3600000).toISOString():null,stopped_at:captured&&!capture?now:null,stop_reason:captured&&!capture?'user':null,event_count:captured?3:0,last_event_at:captured?now:null,truncated:false});
  const keys: ApiKeyRecord[] = [{id:'key-1',vehicle_id:vehicleId,name:'Home dashboard',access_level:'read',access_level_state:'supported',created_at:now,last_used_at:now,expires_at:null,revoked_at:null}];
  const backups: BackupOverview = {
    settings:{enabled:true,frequency:'daily',run_at:'03:00',timezone:'UTC',day_of_week:null,day_of_month:null,retention_count:7,local_enabled:true,s3_enabled:false,target_type:'local',endpoint:'',region:null,bucket:'',prefix:'',access_key:null,has_secret_key:false,updated_at:now},
    recent_runs:[{id:'run-1',trigger:'scheduled',status:'completed',phase:'completed',progress_percent:100,artifact_key:'backup-1',started_at:now,completed_at:now,error_message:null,created_at:now,updated_at:now}],
    recent_runs_total:1,recent_runs_page:1,recent_runs_per_page:10,
    artifacts:[{id:'backup-1',run_id:'run-1',storage_type:'local',file_name:'panda-2026-10-09.tar.gz',storage_path:'/preview/backup-1',size_bytes:9150000,checksum_sha256:'0'.repeat(64),manifest:{format:'panda-recovery-package',restore_availability:'available',package:{format:'panda-recovery-package',format_version:1,source:{app_version:'local-preview'},scope:{included:['Telemetry','Settings','Dashboards'],redacted:['Provider credentials']}}},created_at:now}],
    restore_requests:[],latest_successful_run:null,next_run_at:'2026-10-10T10:00:00Z',runtime_readiness:{pg_dump_available:true,run_now_allowed:true,restore_automation_available:false,reason:null,restore_automation_reason:'Local review: no database is connected.'},s3_catalog_error:null,
  };
  const connections: ExternalConnectionsResponse = {can_manage:true,connections:[{
    id:'basemap',name:'Street maps',purpose:'Street context for your recorded drives.',data_shared:['Map tile area'],disabled_effect:'Maps display recorded routes without a street basemap.',execution:'Cloudflare gateway',privacy_url:null,terms_url:null,editable:false,enabled:true,mode:'remote',endpoint:null,endpoint_is_private:false,weather_precision:'approximate',forecast_url:null,archive_url:null,base_url:null,light_url_template:null,dark_url_template:null,attribution:'OpenFreeMap',attribution_url:null,request_identifier:null,custom_autocomplete:false,allow_private_network:false,has_api_key:false,has_bearer_token:false,updated_at:now,last_attempt_at:now,last_success_at:now,last_error:null,last_test_at:null,last_test_ok:null,last_test_error:null,cache:null,request_count_today:12,
  }]};
  connections.connections.push({...connections.connections[0]!,id:'open_meteo',name:'Weather',purpose:'Outside temperature estimates for recorded drives.',data_shared:['Rounded drive coordinates','Drive date'],disabled_effect:'New temperature estimates stop. Recorded readings remain.',execution:'Server',enabled:false,mode:'disabled',request_count_today:0,attribution:null});
  const auth: Record<string,unknown> = {oidc_enabled:false,password_login_enabled:true,oidc_auto_login:false,issuer_url:null,public_base_url:null,client_id:null,button_label:'Sign in',scopes:'openid profile email',token_auth_method:'client_secret_basic',auto_signup:false,auto_link_verified_email:false,allowed_email_domains:[],required_claim_name:null,required_claim_value:null};
  const samples = [0,1,2].map(i=>({ts:new Date(Date.now() - (i+1)*300_000).toISOString(),battery_level:68-i,speed_mph:i*15,odometer_miles:12486-i,range_miles:218,latitude:null,longitude:null,power_kw:i*8,tire_fl_psi:45,tire_fr_psi:45,tire_rl_psi:45,tire_rr_psi:44,doors_locked:true,ota_current_version:'2026.35',twelve_volt_health:'good',is_online:true}));
  return async (url: URL, method: string, body: Record<string,unknown>, reply: Reply): Promise<boolean> => {
    const path=url.pathname;
    const done=async(value:unknown,status=200)=>{await reply(value,status);return true;};
    if(path==='/v1/api-keys') {
      if(method==='POST'){const record={...keys[0]!,id:`key-${keys.length+1}`,name:String(body.name),revoked_at:null};keys.push(record);return done({key:'preview-key-not-valid-outside-this-preview',record});}
      return done(keys);
    }
    if(path.startsWith('/v1/api-keys/')&&method==='DELETE'){const key=keys.find(k=>k.id===path.split('/').pop());if(key)key.revoked_at=now;return done({});}
    if(path==='/v1/api/catalog')return done({version:'v1',authentication:'Bearer token',endpoints:[{method:'GET',path:'/v1/vehicles',summary:'Your vehicles'},{method:'GET',path:'/v1/trips',summary:'Drive history'},{method:'GET',path:'/v1/charging',summary:'Charging history'}]});
    if(path.endsWith('/backfill-status'))return done({vehicle_id:vehicleId,status:backfill,history_backfilled_at:now,rivian_session_count:6,local_session_count:6,missing_source_count:0});
    if(path.endsWith('/backfill')&&method==='POST'){backfill='pending';return done({});}
    if(path==='/v1/admin/backups')return done(backups);
    if(path==='/v1/admin/backups/settings'&&method==='PUT'){Object.assign(backups.settings,body);return done(backups.settings);}
    if(path==='/v1/admin/backups/run'&&method==='POST'){const run={...backups.recent_runs[0]!,id:'run-2',trigger:'manual' as const};backups.recent_runs.unshift(run);backups.recent_runs_total++;return done({run,artifacts:backups.artifacts});}
    if(path==='/v1/admin/backups/s3/test')return done({ok:true,message:'Local sample validation completed. No connection was made.'});
    if(path.endsWith('/backup-1/download')) {await reply('Sample recovery package. This file contains no database.',200,{'content-disposition':'attachment; filename="preview-backup.txt"'});return true;}
    if(path==='/v1/admin/backups/restores/preflight')return done({error:{message:'Restore is unavailable in the sample preview. No database is connected.'}},409);
    if(path==='/v1/settings/external-connections')return done(connections);
    if(path==='/v1/settings/authentication'){
      if(method==='PUT')Object.assign(auth,body);
      return done({...Object.fromEntries(Object.entries(auth).map(([key,value])=>[key,{value,source:'database'}])),client_secret:{configured:false,source:'default'},last_validation_at:null,callback_url:'https://preview.invalid/v1/auth/oidc/callback'});
    }
    if(path==='/v1/settings/authentication/test')return done({valid:false,discovery:'unavailable',message:'Sample preview: no SSO provider is connected.'});
    if(path.endsWith('/ingestion-diagnostics/start')){capture=true;captured=true;return done(captureState());}
    if(path.endsWith('/ingestion-diagnostics/stop')){capture=false;return done(captureState());}
    if(path.endsWith('/ingestion-diagnostics'))return done(captureState());
    if(path.endsWith('/raw-data')){
      const search=url.searchParams.get('search')?.toLowerCase()??'';
      const filtered=search ? samples.filter(sample=>JSON.stringify(sample).toLowerCase().includes(search)) : samples;
      return done({vehicle_id:vehicleId,samples:filtered,total:filtered.length,page:1,per_page:25,field_coverage:Object.keys(samples[0]!).filter(k=>k!=='ts').map(field=>({field,sample_count:3})),coverage:{first_event_at:samples[2]!.ts,last_event_at:samples[0]!.ts,sample_count:filtered.length,odometer_samples:3,battery_samples:3,range_samples:3,outside_temp_samples:0,power_samples:3,regen_samples:0,tire_pressure_samples:3,lock_samples:3,software_samples:3}});
    }
    if(path.endsWith('/raw-events'))return done({items:[{id:'event-1',event_type:'vehicle_state',received_at:now,message_type:'telemetry',has_json:true}],total:1,page:1,per_page:25,retention_days:7});
    if(path.endsWith('/raw-events/event-1'))return done({id:'event-1',event_type:'vehicle_state',received_at:now,payload_format:'json',payload:{battery_level:68,sample:true}});
    if(path==='/v1/admin/rivian/stewardship')return done({active_collectors:1,raw_events_retained:3,retention_days:7,totals_24h:{ws_payload_messages_received:3,telemetry_writes_persisted:3,telemetry_writes_suppressed:0}});
    if(path.endsWith('/health'))return done({vehicle_id:vehicleId,extended_telemetry:{collector:null},vehicle:{name:'Your Rivian',model:'R2',trim:'Performance'},latest:{ts:now,twelve_volt_health:'good',hv_thermal_event:'normal'},runtime:{last_event_at:now,worker_health:'healthy'},generated_at:now,tires:{ts:now,tire_fl_psi:45,tire_fr_psi:45,tire_rl_psi:45,tire_rr_psi:44},closures:{door_front_left_closed:true,door_front_right_closed:true,door_rear_left_closed:true,door_rear_right_closed:true,closure_frunk_closed:true,closure_liftgate_closed:true},current_software_version:'2026.35',software_history:[{version:'2026.35',installed_at:now,observed_until:null},{version:'2026.31',installed_at:'2026-09-18T10:00:00Z',observed_until:now}],thermal_events_30d:2});
    return false;
  };
}
