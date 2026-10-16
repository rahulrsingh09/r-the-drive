import type {
  Vehicle,
  VehicleStatus,
  VehicleImages,
  TrackPoint,
  TripMapResponse,
  TirePressureTimelineResponse,
  ChargeSession,
  StatsSummary,
  EfficiencyByTag,
  AuthTokens,
  AuthMeResponse,
  AuthConfigResponse,
  AuthenticationSettings,
  AuthenticationSettingsUpdate,
  OidcIdentityStatus,
  ConnectResult,
  RefreshVehicleCredentialsResult,
  AddVehicleBody,
  AddVehicleResult,
  CreateDemoVehicleBody,
  CreateDemoVehicleResult,
  VehicleIngestionCapture,
  ApiKeyRecord,
  CreateApiKeyBody,
  CreateApiKeyResult,
  ApiCatalog,
  RawTelemetryResponse,
  RawTelemetryQuery,
  TelemetryLaneFrame,
  TelemetryLaneQuery,
  RawEventDetail,
  RawEventListResponse,
  RawEventQuery,
  Place,
  PlaceSearchSuggestion,
  UpsertPlaceBody,
  VehicleHealth,
  BatteryHealthSummary,
  BatteryMileagePoint,
  RivianStewardshipResponse,
  MetricCatalogEntry,
  MetricSeriesPoint,
  MetricValueResponse,
  MetricBatchRequest,
  MetricBatchResponse,
  BackupOverview,
  UpdateBackupSettingsBody,
  RunBackupResponse,
  UnitPreferences,
  ThemePreferences,
  UserPreferencesResponse,
  DashboardChartFavorites,
  AppTimezone,
  AppVersionResponse,
  UpdateCheckSettings,
  CreateBackupRestoreRequestBody,
  BackupRestoreRequest,
  RestoreJob,
  RestorePreflightResponse,
  StartBackupRestoreBody,
  StartRestoreResponse,
  UploadBackupResponse,
  IdleDrainResponse,
  VehicleMember,
  AddVehicleMemberBody,
  UpdateVehicleMemberBody,
  CreateVehicleInviteBody,
  VehicleInvite,
  UpdateVehicleSettingsBody,
  AdminUserRecord,
  AdminVehicleOption,
  CreateAccountInvitationBody,
  UpdateAdminUserBody,
  AdminUserDetail,
  AdminUserInvite,
  AccountInvitation,
  AccountInvitationPreview,
  AuthSetupResponse,
  ChangePasswordBody,
  ExternalConnectionsResponse,
  UpdateExternalConnectionBody,
  TestExternalConnectionResponse,
  PurgeExternalConnectionCacheResponse,
  TripTag,
  TripTagAssignmentRequest,
  TripTagColorToken,
  ChargeSessionUpdate,
  ChargingNetworkPreference,
} from '@panda/types';
import { RequestTransport } from './apiClientTransport';
import type {
  ChargingSchedule,
  ChargingScheduleInput,
  DepartureSchedule,
  DepartureScheduleInput,
  LiveSession,
  BackfillStatus,
} from './apiClientTransport';
import {
  decodeListVehicles,
  decodeDownloadVehicleIngestionCapture,
  decodeListVehicleMembers,
  decodeListVehicleInvites,
  decodeListUsers,
  decodeListAdminVehicleOptions,
  decodeListAccountInvitations,
  decodeListUserVehicleMemberships,
  decodeListUserInvites,
  decodeListPlaces,
  decodeDownloadBackupArtifact,
  decodeGetBatteryMileage,
  decodeListTrips,
  decodeGetTrip,
  decodeGetTripDetailData,
  decodeGetSpeedProfile,
  decodeGetTripPowerProfile,
  decodeGetTripDetailSeries,
  decodeListChargeSessions,
  decodeGetChargeSession,
  decodeUpdateChargeSession,
  decodeGetChargeCurve,
  decodeGetChargeCurveAnalysis,
  decodeGetChargingSummary,
  decodeGetChargingChartSeries,
  decodeGetLiveSession,
  decodeGetEfficiencySummary,
  decodeGetEfficiencyByMode,
  decodeGetMetricCatalog,
  buildTimeframeParams,
  buildTripTagFilterParams,
} from './normalization';

export class AuthenticatedTransport extends RequestTransport {
  async login(email: string, password: string): Promise<AuthTokens> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/login',
      body: { email, password },
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async register(email: string, password: string, setupToken?: string): Promise<AuthTokens> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/register',
      body: { email, password, ...(setupToken ? { setup_token: setupToken } : {}) },
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async logout(): Promise<void> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/logout',
    });
  }

  async changePassword(body: ChangePasswordBody): Promise<void> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/password',
      body,
    });
  }

  async refresh(): Promise<AuthTokens> {
    return this.refreshAccessToken();
  }

  async setup(): Promise<AuthSetupResponse> {
    return this.execute({
      method: 'GET',
      path: '/v1/auth/setup',
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async previewAccountInvitation(token: string): Promise<AccountInvitationPreview> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/account-invitations/preview',
      body: { token },
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async acceptAccountInvitation(token: string, password: string): Promise<AuthTokens> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/account-invitations/accept',
      body: { token, password },
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async startAccountInvitationOidc(token: string): Promise<{ authorization_url: string }> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/account-invitations/oidc/start',
      body: { token },
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async resumeSession(): Promise<AuthTokens | null> {
    return this.renewSession(true);
  }

  async me(): Promise<AuthMeResponse> {
    return this.execute({
      method: 'GET',
      path: '/v1/auth/me',
    });
  }

  async getAuthConfig(): Promise<AuthConfigResponse> {
    return this.execute({
      method: 'GET',
      path: '/v1/auth/config',
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async getAuthenticationSettings(): Promise<AuthenticationSettings> {
    return this.execute({
      method: 'GET',
      path: '/v1/settings/authentication',
    });
  }

  async updateAuthenticationSettings(
    body: AuthenticationSettingsUpdate
  ): Promise<AuthenticationSettings> {
    return this.execute({
      method: 'PUT',
      path: '/v1/settings/authentication',
      body,
    });
  }

  async testAuthenticationSettings(): Promise<{
    valid: boolean;
    discovery: string;
    message: string;
  }> {
    return this.execute({
      method: 'POST',
      path: '/v1/settings/authentication/test',
    });
  }

  async getOidcIdentities(): Promise<
    OidcIdentityStatus & { oidc_link_available?: boolean; button_label?: string }
  > {
    return this.execute({
      method: 'GET',
      path: '/v1/auth/identities',
    });
  }

  async startOidc(returnTo?: string): Promise<{ authorization_url: string }> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/oidc/start',
      body: returnTo ? { return_to: returnTo } : {},
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async startOidcLink(returnTo?: string): Promise<{ authorization_url: string }> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/oidc/link/start',
      body: returnTo ? { return_to: returnTo } : {},
    });
  }

  async startOidcPasswordSetup(newPassword: string): Promise<{ authorization_url: string }> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/password/oidc/start',
      body: { new_password: newPassword },
    });
  }

  async unlinkOidc(currentPassword: string): Promise<void> {
    return this.execute({
      method: 'POST',
      path: '/v1/auth/oidc/unlink',
      body: { current_password: currentPassword },
    });
  }

  async getUnitPreferences(): Promise<UserPreferencesResponse> {
    return this.execute({
      method: 'GET',
      path: '/v1/auth/preferences',
    });
  }

  async updateUnitPreferences(
    units: UnitPreferences,
    theme?: ThemePreferences
  ): Promise<UserPreferencesResponse> {
    return this.execute({
      method: 'PUT',
      path: '/v1/auth/preferences',
      body: { units, ...(theme ? { theme } : {}) },
    });
  }

  async updateThemePreferences(theme: ThemePreferences): Promise<UserPreferencesResponse> {
    return this.execute({
      method: 'PUT',
      path: '/v1/auth/preferences',
      body: { theme },
    });
  }

  async getDashboardChartFavorites(): Promise<{ chart_favorites: DashboardChartFavorites }> {
    return this.execute({
      method: 'GET',
      path: '/v1/auth/preferences/chart-favorites',
    });
  }

  async updateDashboardChartFavorite(
    key: string,
    chartId: string
  ): Promise<{ chart_favorites: DashboardChartFavorites }> {
    return this.execute({
      method: 'PUT',
      path: '/v1/auth/preferences/chart-favorites',
      body: {
        key,
        chart_id: chartId,
      },
    });
  }

  async getAppTimezone(): Promise<AppTimezone> {
    return this.execute({
      method: 'GET',
      path: '/v1/settings/timezone',
    });
  }

  async getAppVersion(): Promise<AppVersionResponse> {
    return this.execute({
      method: 'GET',
      path: '/v1/app/version',
    });
  }

  async getUpdateCheckSettings(): Promise<UpdateCheckSettings> {
    return this.execute({
      method: 'GET',
      path: '/v1/settings/update-check',
    });
  }

  async updateUpdateCheckSettings(body: UpdateCheckSettings): Promise<UpdateCheckSettings> {
    return this.execute({
      method: 'PUT',
      path: '/v1/settings/update-check',
      body,
    });
  }

  async updateAppTimezone(timezone: string): Promise<AppTimezone> {
    return this.execute({
      method: 'PUT',
      path: '/v1/settings/timezone',
      body: { timezone },
    });
  }

  async listVehicles(): Promise<Vehicle[]> {
    return this.execute({
      method: 'GET',
      path: '/v1/vehicles',
      decode: decodeListVehicles,
    });
  }

  async vehicleStatus(vehicleId: string): Promise<VehicleStatus> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/status`,
    });
  }

  async vehicleImages(vehicleId: string): Promise<VehicleImages> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/images`,
    });
  }

  async refreshVehicleArtwork(vehicleId: string): Promise<{ ok: boolean; vehicle_id: string }> {
    return this.execute({
      method: 'POST',
      path: `/v1/admin/vehicles/${vehicleId}/images/remirror`,
    });
  }

  async purgeVehicleArtworkCache(vehicleId: string): Promise<{ ok: boolean; vehicle_id: string }> {
    return this.execute({
      method: 'POST',
      path: `/v1/admin/vehicles/${vehicleId}/images/cache/purge`,
    });
  }

  async addVehicle(body: AddVehicleBody): Promise<AddVehicleResult> {
    return this.execute({
      method: 'POST',
      path: '/v1/vehicles',
      body,
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async createDemoVehicle(body: CreateDemoVehicleBody): Promise<CreateDemoVehicleResult> {
    return this.execute({
      method: 'POST',
      path: '/v1/vehicles/demo',
      body,
    });
  }

  async refreshDemoVehicle(vehicleId: string): Promise<CreateDemoVehicleResult> {
    return this.execute({
      method: 'POST',
      path: `/v1/vehicles/${vehicleId}/demo/refresh`,
    });
  }

  async getVehicleIngestionCapture(vehicleId: string): Promise<VehicleIngestionCapture> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/ingestion-diagnostics`,
    });
  }

  async startVehicleIngestionCapture(vehicleId: string): Promise<VehicleIngestionCapture> {
    return this.execute({
      method: 'POST',
      path: `/v1/vehicles/${vehicleId}/ingestion-diagnostics/start`,
    });
  }

  async stopVehicleIngestionCapture(vehicleId: string): Promise<VehicleIngestionCapture> {
    return this.execute({
      method: 'POST',
      path: `/v1/vehicles/${vehicleId}/ingestion-diagnostics/stop`,
    });
  }

  async downloadVehicleIngestionCapture(
    vehicleId: string
  ): Promise<{ blob: Blob; fileName: string }> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/ingestion-diagnostics/export`,
      retryOnUnauthorized: true,
      reportErrors: true,
      response: 'raw',
      decode: decodeDownloadVehicleIngestionCapture,
    });
  }

  async deleteVehicle(
    vehicleId: string
  ): Promise<{ ok: boolean; default_vehicle_id: string | null }> {
    return this.execute({
      method: 'DELETE',
      path: `/v1/vehicles/${vehicleId}`,
    });
  }

  async refreshVehicleCredentials(
    vehicleId: string,
    rivianVehicleId?: string
  ): Promise<RefreshVehicleCredentialsResult> {
    return this.execute({
      method: 'PUT',
      path: `/v1/vehicles/${vehicleId}/credentials`,
      body: {
        rivian_vehicle_id: rivianVehicleId,
      },
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async connectRivian(email: string, password: string): Promise<ConnectResult> {
    return this.execute({
      method: 'POST',
      path: '/v1/vehicles/connect',
      body: { email, password },
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async connectRivianOtp(challengeId: string, otp: string): Promise<ConnectResult> {
    return this.execute({
      method: 'POST',
      path: '/v1/vehicles/connect/otp',
      body: { challenge_id: challengeId, otp_code: otp },
      retryOnUnauthorized: true,
      reportErrors: false,
    });
  }

  async updateVehicleBatteryConfig(
    vehicleId: string,
    body: { battery_capacity_kwh?: number; battery_config?: string }
  ): Promise<void> {
    return this.execute({
      method: 'PUT',
      path: `/v1/vehicles/${vehicleId}/battery-config`,
      body,
    });
  }

  async updateVehicleSettings(vehicleId: string, body: UpdateVehicleSettingsBody): Promise<void> {
    return this.execute({
      method: 'PUT',
      path: `/v1/vehicles/${vehicleId}/settings`,
      body,
    });
  }

  async updateVehicleName(vehicleId: string, name: string): Promise<void> {
    return this.execute({
      method: 'PUT',
      path: `/v1/vehicles/${vehicleId}/name`,
      body: { name },
    });
  }

  async setDefaultVehicle(vehicleId: string): Promise<{ ok: boolean; default_vehicle_id: string }> {
    return this.execute({
      method: 'POST',
      path: `/v1/vehicles/${vehicleId}/default`,
    });
  }

  async listVehicleMembers(vehicleId: string): Promise<VehicleMember[]> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/members`,
      decode: decodeListVehicleMembers,
    });
  }

  async addVehicleMember(
    vehicleId: string,
    body: AddVehicleMemberBody
  ): Promise<{ ok: boolean; invite_created: boolean; invite_token?: string }> {
    return this.execute({
      method: 'POST',
      path: `/v1/vehicles/${vehicleId}/members`,
      body,
    });
  }

  async updateVehicleMember(
    vehicleId: string,
    userId: string,
    body: UpdateVehicleMemberBody
  ): Promise<void> {
    return this.execute({
      method: 'PUT',
      path: `/v1/vehicles/${vehicleId}/members/${userId}`,
      body,
    });
  }

  async removeVehicleMember(vehicleId: string, userId: string): Promise<void> {
    return this.execute({
      method: 'DELETE',
      path: `/v1/vehicles/${vehicleId}/members/${userId}`,
    });
  }

  async listVehicleInvites(vehicleId: string): Promise<VehicleInvite[]> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/invites`,
      decode: decodeListVehicleInvites,
    });
  }

  async createVehicleInvite(
    vehicleId: string,
    body: CreateVehicleInviteBody
  ): Promise<{ ok: boolean; invite_token: string; expires_at: string }> {
    return this.execute({
      method: 'POST',
      path: `/v1/vehicles/${vehicleId}/invites`,
      body,
    });
  }

  async revokeVehicleInvite(vehicleId: string, inviteId: string): Promise<void> {
    return this.execute({
      method: 'DELETE',
      path: `/v1/vehicles/${vehicleId}/invites/${inviteId}`,
    });
  }

  async acceptVehicleInvite(token: string): Promise<{ ok: boolean; vehicle_id: string }> {
    return this.execute({
      method: 'POST',
      path: `/v1/invites/${token}/accept`,
    });
  }

  async previewVehicleInvite(token: string): Promise<VehicleInvite & { vehicle_name: string }> {
    return this.execute({
      method: 'GET',
      path: `/v1/invites/${token}`,
    });
  }

  async listUsers(search = ''): Promise<AdminUserRecord[]> {
    return this.execute({
      method: 'GET',
      path: '/v1/admin/users',
      params: search ? { search } : undefined,
      decode: decodeListUsers,
    });
  }

  async listAdminVehicleOptions(): Promise<AdminVehicleOption[]> {
    return this.execute({
      method: 'GET',
      path: '/v1/admin/vehicles',
      decode: decodeListAdminVehicleOptions,
    });
  }

  async listAccountInvitations(): Promise<AccountInvitation[]> {
    return this.execute({
      method: 'GET',
      path: '/v1/admin/account-invitations',
      decode: decodeListAccountInvitations,
    });
  }

  async createAccountInvitation(body: CreateAccountInvitationBody): Promise<{
    id: string;
    invitee_email: string;
    vehicle_id: string | null;
    vehicle_ids: string[];
    auth_methods: AccountInvitation['auth_methods'];
    expires_at: string;
    activation_token: string;
  }> {
    return this.execute({
      method: 'POST',
      path: '/v1/admin/account-invitations',
      body,
    });
  }

  async revokeAccountInvitation(id: string): Promise<void> {
    return this.execute({
      method: 'DELETE',
      path: `/v1/admin/account-invitations/${id}`,
    });
  }

  async updateUser(id: string, body: UpdateAdminUserBody): Promise<void> {
    return this.execute({
      method: 'PATCH',
      path: `/v1/admin/users/${id}`,
      body,
    });
  }

  async deleteUser(id: string): Promise<void> {
    return this.execute({
      method: 'DELETE',
      path: `/v1/admin/users/${id}`,
    });
  }

  async listUserVehicleMemberships(id: string): Promise<
    Array<{
      vehicle_id: string;
      role: string;
      is_default: boolean;
      created_at: string;
      model: string;
      display_name: string | null;
    }>
  > {
    return this.execute({
      method: 'GET',
      path: `/v1/admin/users/${id}/vehicles`,
      decode: decodeListUserVehicleMemberships,
    });
  }

  async getUserDetail(id: string): Promise<AdminUserDetail> {
    return this.execute({
      method: 'GET',
      path: `/v1/admin/users/${id}/detail`,
    });
  }

  async listUserInvites(id: string): Promise<AdminUserInvite[]> {
    return this.execute({
      method: 'GET',
      path: `/v1/admin/users/${id}/invites`,
      decode: decodeListUserInvites,
    });
  }

  async revokeUserInvite(userId: string, inviteId: string): Promise<void> {
    return this.execute({
      method: 'POST',
      path: `/v1/admin/users/${userId}/invites/${inviteId}/revoke`,
    });
  }

  async grantUserVehicleMembership(
    userId: string,
    vehicleId: string,
    role: 'owner' | 'manager' | 'viewer'
  ): Promise<void> {
    return this.execute({
      method: 'POST',
      path: `/v1/admin/users/${userId}/vehicles/${vehicleId}`,
      body: { role },
    });
  }

  async updateUserVehicleMembership(
    userId: string,
    vehicleId: string,
    role: 'owner' | 'manager' | 'viewer'
  ): Promise<void> {
    return this.execute({
      method: 'PATCH',
      path: `/v1/admin/users/${userId}/vehicles/${vehicleId}`,
      body: { role },
    });
  }

  async removeUserVehicleMembership(userId: string, vehicleId: string): Promise<void> {
    return this.execute({
      method: 'DELETE',
      path: `/v1/admin/users/${userId}/vehicles/${vehicleId}`,
    });
  }

  async listApiKeys(): Promise<ApiKeyRecord[]> {
    return this.execute({
      method: 'GET',
      path: '/v1/api-keys',
    });
  }

  async createApiKey(body: CreateApiKeyBody): Promise<CreateApiKeyResult> {
    return this.execute({
      method: 'POST',
      path: '/v1/api-keys',
      body,
    });
  }

  async revokeApiKey(id: string): Promise<void> {
    return this.execute({
      method: 'DELETE',
      path: `/v1/api-keys/${id}`,
    });
  }

  async listPlaces(): Promise<Place[]> {
    return this.execute({
      method: 'GET',
      path: '/v1/places',
      decode: decodeListPlaces,
    });
  }

  async searchPlaceAddresses(query: string, limit = 5): Promise<PlaceSearchSuggestion[]> {
    return this.execute({
      method: 'GET',
      path: '/v1/places/search',
      params: { q: query, limit },
    });
  }

  async createPlace(body: UpsertPlaceBody): Promise<Place> {
    return this.execute({
      method: 'POST',
      path: '/v1/places',
      body,
    });
  }

  async updatePlace(id: string, body: UpsertPlaceBody): Promise<Place> {
    return this.execute({
      method: 'PUT',
      path: `/v1/places/${id}`,
      body,
    });
  }

  async deletePlace(id: string): Promise<void> {
    return this.execute({
      method: 'DELETE',
      path: `/v1/places/${id}`,
    });
  }

  async getApiCatalog(): Promise<ApiCatalog> {
    return this.execute({
      method: 'GET',
      path: '/v1/api/catalog',
    });
  }

  async getBackupOverview(options?: { page?: number; perPage?: number }): Promise<BackupOverview> {
    return this.execute({
      method: 'GET',
      path: '/v1/admin/backups',
      params: options
        ? {
            ...(options.page !== undefined ? { page: options.page } : {}),
            ...(options.perPage !== undefined ? { per_page: options.perPage } : {}),
          }
        : undefined,
    });
  }

  async getExternalConnections(): Promise<ExternalConnectionsResponse> {
    return this.execute({
      method: 'GET',
      path: '/v1/settings/external-connections',
    });
  }

  async updateExternalConnection(
    id: string,
    body: UpdateExternalConnectionBody
  ): Promise<ExternalConnectionsResponse> {
    return this.execute({
      method: 'PUT',
      path: `/v1/settings/external-connections/${encodeURIComponent(id)}`,
      body,
    });
  }

  async testExternalConnection(
    id: string,
    body: UpdateExternalConnectionBody
  ): Promise<TestExternalConnectionResponse> {
    return this.execute({
      method: 'POST',
      path: `/v1/settings/external-connections/${encodeURIComponent(id)}/test`,
      body,
    });
  }

  async purgeExternalConnectionCache(id: string): Promise<PurgeExternalConnectionCacheResponse> {
    return this.execute<PurgeExternalConnectionCacheResponse>({
      method: 'POST',
      path: `/v1/settings/external-connections/${encodeURIComponent(id)}/cache/purge`,
    });
  }

  async disableOptionalExternalConnections(): Promise<ExternalConnectionsResponse> {
    return this.execute({
      method: 'POST',
      path: '/v1/settings/external-connections/disable-optional',
    });
  }

  async updateBackupSettings(body: UpdateBackupSettingsBody) {
    return this.execute({
      method: 'PUT',
      path: '/v1/admin/backups/settings',
      body,
    });
  }

  async runBackupNow(): Promise<RunBackupResponse> {
    return this.execute({
      method: 'POST',
      path: '/v1/admin/backups/run',
    });
  }

  async testBackupS3(body: UpdateBackupSettingsBody): Promise<{ ok: boolean; message: string }> {
    return this.execute({
      method: 'POST',
      path: '/v1/admin/backups/s3/test',
      body,
    });
  }

  async requestBackupRestore(body: CreateBackupRestoreRequestBody): Promise<BackupRestoreRequest> {
    return this.execute({
      method: 'POST',
      path: '/v1/admin/backups/restore-requests',
      body,
    });
  }

  async uploadBackupArtifact(
    file: File,
    onProgress?: (loaded: number, total: number) => void
  ): Promise<UploadBackupResponse> {
    return this.uploadFile(file, onProgress);
  }

  async deleteUploadedBackup(artifactId: string): Promise<void> {
    return this.execute({
      method: 'DELETE',
      path: `/v1/admin/backups/imports/${artifactId}`,
    });
  }

  async preflightBackupRestore(artifactId: string): Promise<RestorePreflightResponse> {
    return this.execute({
      method: 'POST',
      path: '/v1/admin/backups/restores/preflight',
      body: {
        artifact_id: artifactId,
      },
    });
  }

  async startBackupRestore(body: StartBackupRestoreBody): Promise<StartRestoreResponse> {
    return this.execute({
      method: 'POST',
      path: '/v1/admin/backups/restores',
      body,
    });
  }

  async getRestoreJob(jobId: string, capabilityToken: string): Promise<RestoreJob> {
    return this.restoreStatus(jobId, capabilityToken);
  }

  async downloadBackupArtifact(artifactId: string): Promise<{ blob: Blob; fileName: string }> {
    return this.execute({
      method: 'GET',
      path: `/v1/admin/backups/artifacts/${artifactId}/download`,
      retryOnUnauthorized: true,
      reportErrors: true,
      response: 'raw',
      decode: (res) => decodeDownloadBackupArtifact(res, artifactId),
    });
  }

  async getSoc(vehicleId: string, from: string | null, to: string | null, lifetime = false) {
    return this.execute<{ ts: string; value: number | null }[]>({
      method: 'GET',
      path: '/v1/battery/soc',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
      },
    });
  }

  async getRange(vehicleId: string, from: string | null, to: string | null, lifetime = false) {
    return this.execute<{ ts: string; value: number | null }[]>({
      method: 'GET',
      path: '/v1/battery/range',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
      },
    });
  }

  async getPhantomDrain(
    vehicleId: string,
    from: string | null,
    to: string | null,
    lifetime = false
  ) {
    return this.execute<
      {
        day: string;
        total_soc_lost: number | null;
        avg_drain_rate: number | null;
        hours_parked: number | null;
      }[]
    >({
      method: 'GET',
      path: '/v1/battery/phantom-drain',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
      },
    });
  }

  async getIdleDrainPeriods(
    vehicleId: string,
    from: string | null,
    to: string | null,
    limit = 250,
    minDurationHours = 6,
    lifetime = false
  ): Promise<IdleDrainResponse> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/idle-drain`,
      params: {
        ...buildTimeframeParams(from, to, lifetime),
        limit,
        min_duration_hours: minDurationHours,
      },
    });
  }

  async getParkedEnergy(
    vehicleId: string,
    from: string | null,
    to: string | null
  ): Promise<import('@panda/types').ParkedEnergyResponse> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/parked-energy`,
      params: {
        ...(from ? { from } : {}),
        ...(to ? { to } : {}),
      },
    });
  }

  async getDegradation(
    vehicleId: string,
    from?: string | null,
    to?: string | null,
    lifetime = false
  ) {
    return this.execute<
      {
        ts: string;
        usable_kwh: number;
        rated_kwh: number | null;
        capacity_pct: number;
        odometer_mi?: number | null;
      }[]
    >({
      method: 'GET',
      path: '/v1/battery/degradation',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from ?? null, to ?? null, lifetime),
      },
    });
  }

  async getBatteryHealth(vehicleId: string): Promise<BatteryHealthSummary> {
    return this.execute({
      method: 'GET',
      path: '/v1/battery/health',
      params: { vehicle_id: vehicleId },
    });
  }

  async getBatteryMileage(
    vehicleId: string,
    from?: string | null,
    to?: string | null,
    lifetime = false
  ): Promise<BatteryMileagePoint[]> {
    return this.execute({
      method: 'GET',
      path: '/v1/battery/mileage',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from ?? null, to ?? null, lifetime),
      },
      decode: decodeGetBatteryMileage,
    });
  }

  async listTrips(
    vehicleId: string,
    from: string | null,
    to: string | null,
    page = 1,
    perPage = 25,
    search = '',
    lifetime = false,
    filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }
  ) {
    const offset = (page - 1) * perPage;
    const trimmedSearch = search.trim();
    const tagIds = [...new Set(filters?.tagIds ?? [])].sort();
    return this.execute({
      method: 'GET',
      path: '/v1/trips',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
        page,
        per_page: perPage,
        limit: perPage,
        offset,
        ...(trimmedSearch ? { search: trimmedSearch } : {}),
        ...(tagIds.length
          ? { tag_ids: tagIds.join(','), tag_match: filters?.tagMatch ?? 'all' }
          : {}),
        ...(filters?.untagged ? { untagged: 1 } : {}),
      },
      decode: (response: Parameters<typeof decodeListTrips>[0]) =>
        decodeListTrips(response, page, perPage),
    });
  }

  async getTripMap(
    vehicleId: string,
    from: string | null,
    to: string | null,
    search = '',
    lifetime = false,
    filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }
  ) {
    const tagIds = [...new Set(filters?.tagIds ?? [])].sort();
    return this.execute<TripMapResponse>({
      method: 'GET',
      path: '/v1/trips/map',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
        ...(search.trim() ? { search: search.trim() } : {}),
        ...(tagIds.length
          ? { tag_ids: tagIds.join(','), tag_match: filters?.tagMatch ?? 'all' }
          : {}),
        ...(filters?.untagged ? { untagged: 1 } : {}),
      },
    });
  }

  async getTirePressureTimeline(
    vehicleId: string,
    from: string | null,
    to: string | null,
    lifetime = false,
    filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }
  ) {
    const tagIds = [...new Set(filters?.tagIds ?? [])].sort();
    return this.execute<TirePressureTimelineResponse>({
      method: 'GET',
      path: '/v1/trips/tire-pressure-timeline',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
        ...(tagIds.length
          ? { tag_ids: tagIds.join(','), tag_match: filters?.tagMatch ?? 'all' }
          : {}),
        ...(filters?.untagged ? { untagged: 1 } : {}),
      },
    });
  }

  async listTripTags(vehicleId: string) {
    return this.execute<TripTag[]>({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/trip-tags`,
    });
  }

  async createTripTag(vehicleId: string, body: { name: string; color_token?: TripTagColorToken }) {
    return this.execute<TripTag>({
      method: 'POST',
      path: `/v1/vehicles/${vehicleId}/trip-tags`,
      body,
    });
  }

  async updateTripTag(
    vehicleId: string,
    tagId: string,
    body: { name?: string; color_token?: TripTagColorToken }
  ) {
    return this.execute<TripTag>({
      method: 'PATCH',
      path: `/v1/vehicles/${vehicleId}/trip-tags/${tagId}`,
      body,
    });
  }

  async deleteTripTag(vehicleId: string, tagId: string) {
    return this.execute<{ ok: true }>({
      method: 'DELETE',
      path: `/v1/vehicles/${vehicleId}/trip-tags/${tagId}`,
    });
  }

  async updateTripTagAssignments(vehicleId: string, body: TripTagAssignmentRequest) {
    return this.execute<{ updated_trip_count: number }>({
      method: 'POST',
      path: `/v1/vehicles/${vehicleId}/trip-tags/assignments`,
      body,
    });
  }

  async getTrip(tripId: string, vehicleId: string) {
    return this.execute({
      method: 'GET',
      path: `/v1/trips/${tripId}`,
      params: {
        vehicle_id: vehicleId,
      },
      decode: decodeGetTrip,
    });
  }

  async getTripDetailData(tripId: string, vehicleId: string) {
    return this.execute({
      method: 'GET',
      path: `/v1/trips/${tripId}/detail`,
      params: {
        vehicle_id: vehicleId,
      },
      decode: decodeGetTripDetailData,
    });
  }

  async getTripTrack(tripId: string, vehicleId: string) {
    return this.execute<TrackPoint[]>({
      method: 'GET',
      path: `/v1/trips/${tripId}/track`,
      params: {
        vehicle_id: vehicleId,
      },
    });
  }

  async getSpeedProfile(tripId: string, vehicleId: string) {
    return this.execute({
      method: 'GET',
      path: `/v1/trips/${tripId}/speed`,
      params: { vehicle_id: vehicleId },
      decode: decodeGetSpeedProfile,
    });
  }

  async getElevationProfile(tripId: string, vehicleId: string) {
    return this.execute<{ ts: string; value: number | null }[]>({
      method: 'GET',
      path: `/v1/trips/${tripId}/elevation`,
      params: { vehicle_id: vehicleId },
    });
  }

  async getTripPowerProfile(tripId: string, vehicleId: string) {
    return this.execute({
      method: 'GET',
      path: `/v1/trips/${tripId}/power`,
      params: { vehicle_id: vehicleId },
      decode: decodeGetTripPowerProfile,
    });
  }

  async getTripDetailSeries(tripId: string, vehicleId: string) {
    return this.execute({
      method: 'GET',
      path: `/v1/trips/${tripId}/series`,
      params: { vehicle_id: vehicleId },
      decode: decodeGetTripDetailSeries,
    });
  }

  async listChargeSessions(
    vehicleId: string,
    from: string | null,
    to: string | null,
    page = 1,
    perPage = 25,
    search = '',
    lifetime = false,
    sessionDayLocal: string | null = null
  ) {
    const offset = (page - 1) * perPage;
    const trimmedSearch = search.trim();
    return this.execute({
      method: 'GET',
      path: '/v1/charging',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
        page,
        per_page: perPage,
        limit: perPage,
        offset,
        ...(trimmedSearch ? { search: trimmedSearch } : {}),
        ...(sessionDayLocal ? { session_day_local: sessionDayLocal } : {}),
      },
      decode: (response: Parameters<typeof decodeListChargeSessions>[0]) =>
        decodeListChargeSessions(response, page, perPage),
    });
  }

  async getChargeSession(sessionId: string, vehicleId: string) {
    return this.execute({
      method: 'GET',
      path: `/v1/charging/${sessionId}`,
      params: {
        vehicle_id: vehicleId,
      },
      decode: decodeGetChargeSession,
    });
  }

  async updateChargeSession(
    vehicleId: string,
    sessionId: string,
    body: ChargeSessionUpdate
  ): Promise<ChargeSession> {
    return this.execute({
      method: 'PATCH',
      path: `/v1/vehicles/${vehicleId}/charging-sessions/${sessionId}`,
      body,
      decode: decodeUpdateChargeSession,
    });
  }

  async listChargingNetworkPreferences(vehicleId: string): Promise<ChargingNetworkPreference[]> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/charging-networks`,
    });
  }

  async updateChargingNetworkPreference(
    vehicleId: string,
    networkVendor: string,
    costMode: ChargingNetworkPreference['cost_mode']
  ): Promise<Pick<ChargingNetworkPreference, 'network_vendor' | 'cost_mode'>> {
    return this.execute({
      method: 'PATCH',
      path: `/v1/vehicles/${vehicleId}/charging-networks/${encodeURIComponent(networkVendor)}`,
      body: { cost_mode: costMode },
    });
  }

  async getChargeCurve(sessionId: string, vehicleId: string) {
    return this.execute({
      method: 'GET',
      path: `/v1/charging/sessions/${sessionId}/curve`,
      params: { vehicle_id: vehicleId },
      decode: decodeGetChargeCurve,
    });
  }

  async getChargeCurveAnalysis(
    vehicleId: string,
    from: string | null,
    to: string | null,
    lifetime = false
  ) {
    return this.execute({
      method: 'GET',
      path: '/v1/charging/curve-analysis',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
      },
      decode: decodeGetChargeCurveAnalysis,
    });
  }

  async getChargingSummary(
    vehicleId: string,
    from: string | null,
    to: string | null,
    lifetime = false
  ) {
    return this.execute({
      method: 'GET',
      path: '/v1/charging/summary',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
      },
      decode: decodeGetChargingSummary,
    });
  }

  async getChargingChartSeries(
    vehicleId: string,
    from: string | null,
    to: string | null,
    lifetime = false
  ) {
    return this.execute({
      method: 'GET',
      path: '/v1/charging/chart-series',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
      },
      decode: decodeGetChargingChartSeries,
    });
  }

  async getVehicleHealth(vehicleId: string): Promise<VehicleHealth> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/health`,
    });
  }

  async getChargingSchedule(vehicleId: string): Promise<ChargingSchedule | null> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/charging-schedule`,
    });
  }

  async putChargingSchedule(vehicleId: string, body: ChargingScheduleInput): Promise<void> {
    return this.execute({
      method: 'PUT',
      path: `/v1/vehicles/${vehicleId}/charging-schedule`,
      body,
    });
  }

  async listDepartureSchedules(vehicleId: string): Promise<DepartureSchedule[]> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/departure-schedules`,
    });
  }

  async createDepartureSchedule(
    vehicleId: string,
    body: DepartureScheduleInput
  ): Promise<{ rivian_schedule_id: string }> {
    return this.execute({
      method: 'POST',
      path: `/v1/vehicles/${vehicleId}/departure-schedules`,
      body,
    });
  }

  async updateDepartureSchedule(
    vehicleId: string,
    scheduleId: string,
    body: DepartureScheduleInput
  ): Promise<void> {
    return this.execute({
      method: 'PATCH',
      path: `/v1/vehicles/${vehicleId}/departure-schedules/${scheduleId}`,
      body,
    });
  }

  async deleteDepartureSchedule(vehicleId: string, scheduleId: string): Promise<void> {
    return this.execute({
      method: 'DELETE',
      path: `/v1/vehicles/${vehicleId}/departure-schedules/${scheduleId}`,
    });
  }

  async getLiveSession(vehicleId: string): Promise<LiveSession | null> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/live-session`,
      decode: decodeGetLiveSession,
    });
  }

  async getBackfillStatus(vehicleId: string): Promise<BackfillStatus> {
    return this.execute({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/backfill-status`,
    });
  }

  async triggerBackfill(vehicleId: string): Promise<void> {
    return this.execute({
      method: 'POST',
      path: `/v1/vehicles/${vehicleId}/backfill`,
    });
  }

  async getStats(vehicleId: string): Promise<StatsSummary> {
    return this.execute<StatsSummary>({
      method: 'GET',
      path: '/v1/stats',
      params: { vehicle_id: vehicleId },
    });
  }

  async getEfficiencySummary(
    vehicleId: string,
    from: string | null,
    to: string | null,
    lifetime = false,
    filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }
  ) {
    return this.execute({
      method: 'GET',
      path: '/v1/efficiency/summary',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
        ...buildTripTagFilterParams(filters),
      },
      decode: decodeGetEfficiencySummary,
    });
  }

  async getEfficiencyByMode(
    vehicleId: string,
    from: string | null,
    to: string | null,
    lifetime = false,
    filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }
  ) {
    return this.execute({
      method: 'GET',
      path: '/v1/efficiency/by-mode',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
        ...buildTripTagFilterParams(filters),
      },
      decode: decodeGetEfficiencyByMode,
    });
  }

  async getEfficiencyTrend(
    vehicleId: string,
    from: string | null,
    to: string | null,
    lifetime = false,
    filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }
  ) {
    return this.execute<
      { ts: string; trip_efficiency_wh_mi: number | null; rolling_24h_wh_mi: number | null }[]
    >({
      method: 'GET',
      path: '/v1/efficiency/trend',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
        ...buildTripTagFilterParams(filters),
      },
    });
  }

  async getEfficiencyVsTemp(
    vehicleId: string,
    from: string | null,
    to: string | null,
    lifetime = false,
    filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }
  ) {
    return this.execute<
      {
        temp_c_low: number;
        temp_c_high: number;
        avg_efficiency_wh_mi: number | null;
        trip_count: number;
        total_miles: number | null;
        avg_speed_mph: number | null;
      }[]
    >({
      method: 'GET',
      path: '/v1/efficiency/vs-temp',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
        ...buildTripTagFilterParams(filters),
      },
    });
  }

  async getEfficiencyByTag(
    vehicleId: string,
    from: string | null,
    to: string | null,
    lifetime = false,
    filters?: { tagIds?: string[]; tagMatch?: 'all' | 'any'; untagged?: boolean }
  ) {
    return this.execute<EfficiencyByTag[]>({
      method: 'GET',
      path: '/v1/efficiency/by-tag',
      params: {
        vehicle_id: vehicleId,
        ...buildTimeframeParams(from, to, lifetime),
        ...buildTripTagFilterParams(filters),
      },
    });
  }

  async getMetricCatalog(): Promise<MetricCatalogEntry[]> {
    return this.execute({
      method: 'GET',
      path: '/v1/metrics/catalog',
      decode: decodeGetMetricCatalog,
    });
  }

  async getMetricValue(
    vehicleId: string,
    metric: string,
    from: string | null = null,
    to: string | null = null,
    lifetime = false
  ): Promise<MetricValueResponse> {
    return this.execute({
      method: 'GET',
      path: '/v1/metrics/value',
      params: {
        vehicle_id: vehicleId,
        metric,
        ...buildTimeframeParams(from, to, lifetime),
      },
    });
  }

  async getMetricSeries(
    vehicleId: string,
    metric: string,
    from: string | null,
    to: string | null,
    bucket = 'day',
    lifetime = false
  ): Promise<MetricSeriesPoint[]> {
    return this.execute({
      method: 'GET',
      path: '/v1/metrics/series',
      params: {
        vehicle_id: vehicleId,
        metric,
        ...buildTimeframeParams(from, to, lifetime),
        bucket,
      },
    });
  }

  async getMetricBatch(request: MetricBatchRequest): Promise<MetricBatchResponse> {
    return this.execute({
      method: 'POST',
      path: '/v1/metrics/batch',
      body: request,
    });
  }

  async getRawTelemetry(vehicleId: string, options: RawTelemetryQuery | number = {}) {
    const legacyLimit = typeof options === 'number' ? options : undefined;
    const query = typeof options === 'number' ? {} : options;
    const fields = query.fields?.filter(Boolean).join(',');
    return this.execute<RawTelemetryResponse>({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/raw-data`,
      params: {
        ...(query.from ? { from: query.from } : {}),
        ...(query.to ? { to: query.to } : {}),
        ...(query.page ? { page: query.page } : {}),
        ...(legacyLimit
          ? { limit: legacyLimit }
          : query.per_page
            ? { per_page: query.per_page }
            : {}),
        ...(query.search?.trim() ? { search: query.search.trim() } : {}),
        ...(fields ? { fields } : {}),
        ...(query.populated_only ? { populated_only: 'true' } : {}),
      },
    });
  }

  async getTelemetryLanes(vehicleId: string, query: TelemetryLaneQuery = {}) {
    return this.execute<TelemetryLaneFrame>({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/telemetry/lanes`,
      params: {
        ...(query.from ? { from: query.from } : {}),
        ...(query.to ? { to: query.to } : {}),
        ...(query.lanes?.length ? { lanes: query.lanes.join(',') } : {}),
        ...(query.resolution ? { resolution: query.resolution } : {}),
        ...(query.max_points ? { max_points: query.max_points } : {}),
      },
    });
  }

  async getRawEvents(vehicleId: string, query: RawEventQuery = {}) {
    return this.execute<RawEventListResponse>({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/raw-events`,
      params: {
        ...(query.from ? { from: query.from } : {}),
        ...(query.to ? { to: query.to } : {}),
        ...(query.page ? { page: query.page } : {}),
        ...(query.per_page ? { per_page: query.per_page } : {}),
        ...(query.event_type ? { event_type: query.event_type } : {}),
        ...(query.message_type ? { message_type: query.message_type } : {}),
      },
    });
  }

  async getRawEvent(vehicleId: string, eventId: string) {
    return this.execute<RawEventDetail>({
      method: 'GET',
      path: `/v1/vehicles/${vehicleId}/raw-events/${eventId}`,
    });
  }

  async getRivianStewardship(): Promise<RivianStewardshipResponse> {
    return this.execute({
      method: 'GET',
      path: '/v1/admin/rivian/stewardship',
    });
  }
}

export const transport = new AuthenticatedTransport();

type MethodName = {
  [Key in keyof typeof transport]: (typeof transport)[Key] extends (...args: never[]) => unknown
    ? Key
    : never;
}[keyof typeof transport];

function client<const Keys extends readonly MethodName[]>(...keys: Keys) {
  return Object.fromEntries(keys.map((key) => [key, transport[key].bind(transport)])) as Pick<
    typeof transport,
    Keys[number]
  >;
}

/** Domain-oriented client views. They share the authenticated transport and never own request logic. */
export const authClient = client(
  'login',
  'register',
  'logout',
  'changePassword',
  'refresh',
  'setup',
  'previewAccountInvitation',
  'acceptAccountInvitation',
  'startAccountInvitationOidc',
  'resumeSession',
  'me',
  'getAuthConfig',
  'getOidcIdentities',
  'startOidc',
  'startOidcLink',
  'startOidcPasswordSetup',
  'unlinkOidc',
  'getUnitPreferences',
  'updateUnitPreferences',
  'updateThemePreferences',
  'getDashboardChartFavorites',
  'updateDashboardChartFavorite',
  'getAppTimezone',
  'updateAppTimezone',
  'getAppVersion'
);

export const vehicleClient = client(
  'listVehicles',
  'vehicleStatus',
  'vehicleImages',
  'refreshVehicleArtwork',
  'purgeVehicleArtworkCache',
  'addVehicle',
  'createDemoVehicle',
  'refreshDemoVehicle',
  'getVehicleIngestionCapture',
  'startVehicleIngestionCapture',
  'stopVehicleIngestionCapture',
  'downloadVehicleIngestionCapture',
  'deleteVehicle',
  'refreshVehicleCredentials',
  'connectRivian',
  'connectRivianOtp',
  'updateVehicleBatteryConfig',
  'updateVehicleSettings',
  'updateVehicleName',
  'setDefaultVehicle',
  'listVehicleMembers',
  'addVehicleMember',
  'updateVehicleMember',
  'removeVehicleMember',
  'listVehicleInvites',
  'createVehicleInvite',
  'revokeVehicleInvite',
  'acceptVehicleInvite',
  'previewVehicleInvite',
  'getVehicleHealth'
);

export const tripClient = client(
  'listTrips',
  'getTripMap',
  'getTrip',
  'getTripDetailData',
  'getTripTrack',
  'getSpeedProfile',
  'getElevationProfile',
  'getTripPowerProfile',
  'getTripDetailSeries',
  'listTripTags',
  'createTripTag',
  'updateTripTag',
  'deleteTripTag',
  'updateTripTagAssignments'
);

export const chargingClient = client(
  'listChargeSessions',
  'getChargeSession',
  'updateChargeSession',
  'listChargingNetworkPreferences',
  'updateChargingNetworkPreference',
  'getChargeCurve',
  'getChargeCurveAnalysis',
  'getChargingSummary',
  'getChargingChartSeries',
  'getChargingSchedule',
  'putChargingSchedule',
  'listDepartureSchedules',
  'createDepartureSchedule',
  'updateDepartureSchedule',
  'deleteDepartureSchedule',
  'getLiveSession',
  'getBackfillStatus',
  'triggerBackfill'
);

export const telemetryClient = client(
  'getSoc',
  'getRange',
  'getPhantomDrain',
  'getIdleDrainPeriods',
  'getParkedEnergy',
  'getDegradation',
  'getBatteryHealth',
  'getBatteryMileage',
  'getMetricCatalog',
  'getMetricValue',
  'getMetricSeries',
  'getMetricBatch',
  'getRawTelemetry',
  'getTelemetryLanes',
  'getRawEvents',
  'getRawEvent',
  'getRivianStewardship'
);

export const analyticsClient = client(
  'getStats',
  'getEfficiencySummary',
  'getEfficiencyByMode',
  'getEfficiencyTrend',
  'getEfficiencyVsTemp',
  'getEfficiencyByTag'
);

export const systemClient = client(
  'getAuthenticationSettings',
  'updateAuthenticationSettings',
  'testAuthenticationSettings',
  'listUsers',
  'listAdminVehicleOptions',
  'listAccountInvitations',
  'createAccountInvitation',
  'revokeAccountInvitation',
  'updateUser',
  'deleteUser',
  'listUserVehicleMemberships',
  'getUserDetail',
  'listUserInvites',
  'revokeUserInvite',
  'grantUserVehicleMembership',
  'updateUserVehicleMembership',
  'removeUserVehicleMembership',
  'listApiKeys',
  'createApiKey',
  'revokeApiKey',
  'listPlaces',
  'searchPlaceAddresses',
  'createPlace',
  'updatePlace',
  'deletePlace',
  'getApiCatalog',
  'getBackupOverview',
  'getExternalConnections',
  'updateExternalConnection',
  'testExternalConnection',
  'getUpdateCheckSettings',
  'updateUpdateCheckSettings',
  'purgeExternalConnectionCache',
  'disableOptionalExternalConnections',
  'updateBackupSettings',
  'runBackupNow',
  'testBackupS3',
  'requestBackupRestore',
  'uploadBackupArtifact',
  'deleteUploadedBackup',
  'preflightBackupRestore',
  'startBackupRestore',
  'getRestoreJob',
  'downloadBackupArtifact'
);
