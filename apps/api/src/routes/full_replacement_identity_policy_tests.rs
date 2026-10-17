//! Synthetic HTTP/DB characterization shared by baseline and replacement.
use crate::{authorization_test_support::Fixture, services::{authentication_settings as settings, oidc}};
use axum::{body::{to_bytes, Body}, http::{Request, StatusCode}, Router};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn request(f: &Fixture, method: &str, path: &str, token: &str, body: Value) -> (StatusCode, Value) {
    let response = Router::new()
        .nest("/v1", super::router().merge(crate::routes::api_keys::router()))
        .layer(axum::Extension(f.state.jwt_keys.decoding.clone()))
        .with_state(f.state.clone())
        .oneshot(Request::builder().method(method).uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap())
        .await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
#[ignore = "requires task-owned isolated TimescaleDB and Redis"]
async fn full_identity_policy_key_lifecycle_and_non_enumeration() {
    let f = Fixture::new().await;
    let owner = f.user("user").await;
    let stranger = f.user("user").await;
    let viewer = f.user("user").await;
    let vehicle = f.vehicle(owner, "identity-policy-synthetic").await;
    f.member(viewer, vehicle, "viewer").await;
    let owner_token = f.token(owner);
    for actor in [stranger, viewer] {
        let before = f.snapshot().await;
        assert_eq!(request(&f, "POST", "/v1/api-keys", &f.token(actor), json!({"vehicle_id":vehicle,"name":"denied"})).await.0, StatusCode::FORBIDDEN);
        assert_eq!(before, f.snapshot().await);
    }
    for body in [json!({"vehicle_id":vehicle,"name":" "}), json!({"vehicle_id":vehicle,"name":"bad","expires_in_days":0}), json!({"vehicle_id":vehicle,"name":"bad","expires_in_days":366})] {
        assert_eq!(request(&f,"POST","/v1/api-keys",&owner_token,body).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (status, created) = request(&f,"POST","/v1/api-keys",&owner_token,json!({"vehicle_id":vehicle,"name":"  synthetic key  ","expires_in_days":7})).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["record"]["name"], "synthetic key");
    assert_eq!(created["record"]["access_level"], "read");
    assert_eq!(created["record"]["expiry_state"], "expiring");
    let key = created["key"].as_str().unwrap();
    assert!(key.starts_with("panda_"));
    assert_eq!(key.len(),49);
    let id = created["record"]["id"].as_str().unwrap();
    let stored: Vec<u8> = sqlx::query_scalar("SELECT key_hash FROM panda.api_keys WHERE id=$1")
        .bind(Uuid::parse_str(id).unwrap()).fetch_one(&f.state.pool).await.unwrap();
    assert_eq!(stored, crate::routes::api_keys::hash_api_key(key));
    let (status, listed) = request(&f,"GET","/v1/api-keys",&owner_token,Value::Null).await;
    assert_eq!(status,StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(),1);
    assert!(listed[0].get("key").is_none());
    assert_eq!(request(&f,"GET","/v1/api-keys",key,Value::Null).await.0,StatusCode::FORBIDDEN);
    assert_eq!(request(&f,"GET","/v1/api-keys",&f.token(stranger),Value::Null).await.1,json!([]));
    for target in [id.to_owned(),Uuid::new_v4().to_string()] {
        assert_eq!(request(&f,"DELETE",&format!("/v1/api-keys/{target}"),&f.token(stranger),Value::Null).await.0,StatusCode::NOT_FOUND);
    }
    assert_eq!(request(&f,"DELETE",&format!("/v1/api-keys/{id}"),&owner_token,Value::Null).await.0,StatusCode::NO_CONTENT);
    assert_eq!(request(&f,"DELETE",&format!("/v1/api-keys/{id}"),&owner_token,Value::Null).await.0,StatusCode::NOT_FOUND);
    let audit: i64=sqlx::query_scalar("SELECT count(*) FROM panda.security_events WHERE event_type IN ('api_key_created','api_key_revoked')").fetch_one(&f.state.pool).await.unwrap();
    assert_eq!(audit,2);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned isolated TimescaleDB and Redis"]
async fn full_identity_policy_admin_permissions_and_memberships() {
    let f=Fixture::new().await;
    let super_user=f.user("super_user").await;
    let admin=f.user("admin").await;
    let user=f.user("user").await;
    let vehicle=f.vehicle(user,"identity-policy-admin").await;
    let ordinary=f.token(user);
    for path in ["/v1/admin/users".to_owned(),"/v1/admin/vehicles".to_owned(),"/v1/admin/account-invitations".to_owned(),format!("/v1/admin/users/{admin}/detail"),format!("/v1/admin/users/{}/detail",Uuid::new_v4()),format!("/v1/admin/users/{admin}/vehicles")] {
        assert_eq!(request(&f,"GET",&path,&ordinary,Value::Null).await.0,StatusCode::FORBIDDEN);
    }
    assert_eq!(request(&f,"PATCH",&format!("/v1/admin/users/{super_user}"),&f.token(admin),json!({"email":"forbidden@example.invalid"})).await.0,StatusCode::FORBIDDEN);
    assert_eq!(request(&f,"PATCH",&format!("/v1/admin/users/{user}"),&f.token(admin),json!({"role":"super_user"})).await.0,StatusCode::FORBIDDEN);
    assert_eq!(request(&f,"PATCH",&format!("/v1/admin/users/{super_user}"),&f.token(super_user),json!({"role":"user"})).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(request(&f,"PATCH",&format!("/v1/admin/users/{super_user}"),&f.token(super_user),json!({"is_disabled":true})).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let membership=format!("/v1/admin/users/{user}/vehicles/{vehicle}");
    assert_eq!(request(&f,"DELETE",&membership,&f.token(admin),Value::Null).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let admin_membership=format!("/v1/admin/users/{admin}/vehicles/{vehicle}");
    assert_eq!(request(&f,"POST",&admin_membership,&f.token(admin),json!({"role":"owner"})).await.0,StatusCode::OK);
    assert_eq!(request(&f,"DELETE",&membership,&f.token(admin),Value::Null).await.0,StatusCode::OK);
    assert_eq!(request(&f,"DELETE",&membership,&f.token(admin),Value::Null).await.0,StatusCode::NOT_FOUND);
    assert_eq!(request(&f,"POST",&membership,&f.token(admin),json!({"role":"invented"})).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let body=json!({"email":" Invitation@Example.invalid ","vehicle_ids":[vehicle,vehicle],"expires_in_days":2});
    let (status,invite)=request(&f,"POST","/v1/admin/account-invitations",&f.token(admin),body.clone()).await;
    assert_eq!(status,StatusCode::OK);
    assert_eq!(invite["auth_methods"],"password");
    assert_eq!(invite["vehicle_ids"],json!([vehicle]));
    assert_eq!(invite["invitee_email"],"invitation@example.invalid");
    assert_eq!(invite["activation_token"].as_str().unwrap().len(),48);
    assert_eq!(request(&f,"POST","/v1/admin/account-invitations",&f.token(admin),body).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let listed=request(&f,"GET","/v1/admin/account-invitations",&f.token(admin),Value::Null).await.1;
    assert!(listed["invitations"][0].get("activation_token").is_none());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned isolated TimescaleDB and Redis"]
async fn full_identity_policy_settings_validation_and_secret_boundary() {
    let f=Fixture::new().await;
    let actor=f.user("super_user").await;
    let pool=&f.state.pool; let key=&f.state.age_key;
    let initial=settings::load(pool,key).await.unwrap();
    assert!(initial.password_login_enabled.value);
    assert!(!initial.client_secret.configured);
    assert!(settings::update(pool,key,actor,settings::AuthenticationSettingsUpdate{password_login_enabled:Some(false),..Default::default()}).await.is_err());
    let edit=settings::AuthenticationSettingsUpdate{
        oidc_enabled:Some(true),issuer_url:Some(Some("https://identity.example.invalid/".into())),
        public_base_url:Some(Some("https://app.example.invalid".into())),client_id:Some(Some("client".into())),client_secret:Some(Some(" synthetic-secret ".into())),scopes:Some("openid email openid".into()),..Default::default()};
    let response=settings::update(pool,key,actor,edit).await.unwrap();
    assert_eq!(response.scopes.value,"openid email");
    assert_eq!(response.issuer_url.value.as_deref(),Some("https://identity.example.invalid"));
    assert!(response.client_secret.configured);
    assert!(!serde_json::to_string(&response).unwrap().contains("synthetic-secret"));
    let effective=settings::load_effective(pool,key).await.unwrap();
    assert_eq!(effective.client_secret.as_deref(),Some("synthetic-secret"));
    let ciphertext: Vec<u8>=sqlx::query_scalar("SELECT client_secret_encrypted FROM panda.authentication_settings").fetch_one(pool).await.unwrap();
    assert!(!String::from_utf8_lossy(&ciphertext).contains("synthetic-secret"));
    settings::record_validation(pool,&effective,key).await.unwrap();
    assert!(settings::update(pool,key,actor,settings::AuthenticationSettingsUpdate{password_login_enabled:Some(false),..Default::default()}).await.is_err());
    sqlx::query("INSERT INTO panda.user_oidc_identities(user_id,issuer,subject) VALUES ($1,'https://identity.example.invalid/','synthetic')").bind(actor).execute(pool).await.unwrap();
    assert!(!settings::update(pool,key,actor,settings::AuthenticationSettingsUpdate{password_login_enabled:Some(false),..Default::default()}).await.unwrap().password_login_enabled.value);
    assert!(settings::update(pool,key,actor,settings::AuthenticationSettingsUpdate{client_id:Some(Some("changed".into())),..Default::default()}).await.is_err());
    assert!(settings::update(pool,key,actor,settings::AuthenticationSettingsUpdate{required_claim_name:Some(Some("groups".into())),..Default::default()}).await.is_err());
    f.cleanup().await;
}

fn provider(issuer: String) -> settings::EffectiveAuthenticationSettings {
    settings::EffectiveAuthenticationSettings{oidc_enabled:true,password_login_enabled:true,issuer_url:Some(issuer),public_base_url:Some("https://app.example.invalid".into()),client_id:Some("fixture-client".into()),client_secret:Some("fixture-secret".into()),button_label:"SSO".into(),scopes:"openid email".into(),token_auth_method:"auto".into(),auto_signup:false,auto_link_verified_email:false,oidc_auto_login:false,allowed_email_domains:vec![],required_claim_name:None,required_claim_value:None}
}

#[tokio::test]
#[ignore = "requires task-owned TLS OIDC fixture and operator CIDR env"]
async fn full_identity_policy_oidc_local_provider_errors() {
    let origin=std::env::var("IDENTITY_POLICY_OIDC_ORIGIN").unwrap();
    for (path,expected) in [("good",true),("mismatch",false),("http-token",false),("bad-json",false),("oversize",false),("unavailable",false),("no-pkce",false)] {
        let s=provider(format!("{origin}/{path}"));
        assert_eq!(oidc::discover(&s).await.is_ok(),expected,"{path}");
    }
    let s=provider(format!("{origin}/good"));
    oidc::test_provider(&s).await.unwrap();
    let tx=oidc::Transaction{state:"state".into(),browser_binding:oidc::browser_binding("cookie"),nonce:"nonce".into(),verifier:"verifier".into(),return_to:"/".into(),user_id:None,link:false,pending_password_hash:None,invitation_id:None};
    let metadata=oidc::discover(&s).await.unwrap();
    let auth=oidc::authorization_url(&metadata,&s,&tx,"https://app.example.invalid/v1/auth/oidc/callback").unwrap();
    let pairs:std::collections::HashMap<_,_>=auth.query_pairs().into_owned().collect();
    assert_eq!(pairs["code_challenge"],oidc::pkce_challenge("verifier"));
    assert_eq!(pairs["code_challenge_method"],"S256");
    assert_eq!(pairs["nonce"],"nonce");
    for code in ["reject-code","wrong-nonce","wrong-audience","wrong-issuer","expired","bad-signature"] {
        assert!(oidc::exchange_and_verify(&s,&tx,code).await.is_err(),"{code}");
    }
    for method in ["auto","client_secret_basic","client_secret_post"] {
        let mut selected=s.clone(); selected.token_auth_method=method.into();
        let identity=oidc::exchange_and_verify(&selected,&tx,"accept-code").await.unwrap();
        assert_eq!(identity.subject,"synthetic-subject");
        assert_eq!(identity.email.as_deref(),Some("artist@example.invalid"));
        assert!(identity.email_verified);
        assert!(oidc::claim_matches(&identity,Some("groups"),Some("artists")));
        assert!(oidc::claim_matches(&identity,Some("tenant"),Some("primary")));
    }
    let blocked=provider("https://127.0.0.1:1/good".into());
    assert!(oidc::discover(&blocked).await.is_err());
}

#[tokio::test]
#[ignore = "requires task-owned isolated TimescaleDB and Redis; baseline rotation defect"]
async fn full_identity_policy_rotation_overlap_and_audit_rollback() {
    let f=Fixture::new().await;
    let user=f.user("user").await;
    let vehicle=f.vehicle(user,"identity-policy-rotation").await;
    let token=f.token(user);
    let created=request(&f,"POST","/v1/api-keys",&token,json!({"vehicle_id":vehicle,"name":"rotate"})).await.1;
    let id=created["record"]["id"].as_str().unwrap();
    let (status,rotated)=request(&f,"POST",&format!("/v1/api-keys/{id}/rotate"),&token,Value::Null).await;
    assert_eq!(status,StatusCode::CREATED);
    assert_eq!(rotated["record"]["rotated_from_id"],id);
    assert_ne!(rotated["key"],created["key"]);
    let remaining:i64=sqlx::query_scalar("SELECT extract(epoch FROM expires_at-now())::bigint FROM panda.api_keys WHERE id=$1")
        .bind(Uuid::parse_str(id).unwrap()).fetch_one(&f.state.pool).await.unwrap();
    assert!((86300..=86400).contains(&remaining));
    sqlx::raw_sql("CREATE FUNCTION panda.identity_policy_audit_fail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'identity-policy synthetic audit failure'; END $$; CREATE TRIGGER identity_policy_audit_fail BEFORE INSERT ON panda.security_events FOR EACH ROW EXECUTE FUNCTION panda.identity_policy_audit_fail();")
        .execute(&f.state.pool).await.unwrap();
    let before=f.snapshot().await;
    assert_eq!(request(&f,"POST",&format!("/v1/api-keys/{id}/rotate"),&token,Value::Null).await.0,StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(before,f.snapshot().await);
    assert_eq!(request(&f,"DELETE",&format!("/v1/api-keys/{id}"),&token,Value::Null).await.0,StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(before,f.snapshot().await);
    f.cleanup().await;
}
