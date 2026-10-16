use super::*;

pub(super) async fn exercise(h: &mut Harness) {
    h.step(
        "dashboard empty",
        Some(2),
        "GET",
        "/v1/dashboards",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "charts empty",
        Some(2),
        "GET",
        "/v1/charts",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "no auth",
        None,
        "GET",
        "/v1/dashboards",
        Value::Null,
        &[],
        401,
    )
    .await;
    h.step(
        "dashboard missing",
        Some(2),
        "GET",
        "/v1/dashboards/00000000-0000-0000-0000-000000000099",
        Value::Null,
        &[],
        404,
    )
    .await;
    h.step(
        "dashboard bad uuid",
        Some(2),
        "GET",
        "/v1/dashboards/nope",
        Value::Null,
        &[],
        400,
    )
    .await;
    h.step(
        "dashboard invalid slug",
        Some(2),
        "POST",
        "/v1/dashboards",
        json!({"slug":"Bad","name":"","config":{}}),
        &[],
        422,
    )
    .await;
    h.raw_step(
        "dashboard malformed",
        Some(2),
        "POST",
        "/v1/dashboards",
        "{",
        &[],
        400,
    )
    .await;
    h.raw_step(
        "dashboard oversized",
        Some(2),
        "POST",
        "/v1/dashboards",
        &"x".repeat(65537),
        &[],
        413,
    )
    .await;
    h.step(
        "dashboard missing config",
        Some(2),
        "POST",
        "/v1/dashboards",
        json!({"slug":"valid","name":"test"}),
        &[],
        422,
    )
    .await;
    let response = h.step("dashboard create",Some(2),"POST","/v1/dashboards",json!({"slug":"mine","name":"Personal","description":"synthetic","config":{"widgets":[],"extra":true}}),&[],201).await;
    let id = response["body"]["id"].as_str().unwrap().to_owned();
    h.label(&id, "dashboard-mine");
    let path = format!("/v1/dashboards/{id}");
    h.step(
        "dashboard own",
        Some(2),
        "GET",
        &path,
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "dashboard foreign read",
        Some(3),
        "GET",
        &path,
        Value::Null,
        &[],
        403,
    )
    .await;
    h.step(
        "dashboard foreign write",
        Some(3),
        "PUT",
        &path,
        json!({"name":"forbidden"}),
        &[],
        403,
    )
    .await;
    h.step(
        "dashboard foreign delete",
        Some(3),
        "DELETE",
        &path,
        Value::Null,
        &[],
        403,
    )
    .await;
    h.step(
        "dashboard name-only update",
        Some(2),
        "PUT",
        &path,
        json!({"name":"Renamed","description":null}),
        &[],
        200,
    )
    .await;
    h.step(
        "dashboard config update",
        Some(2),
        "PUT",
        &path,
        json!({"config":{"widgets":[]}}),
        &[],
        200,
    )
    .await;
    let response = h
        .step(
            "dashboard upsert existing",
            Some(2),
            "POST",
            "/v1/dashboards",
            json!({"slug":"mine","name":"Upsert","config":{}}),
            &[],
            201,
        )
        .await;
    assert_eq!(response["body"]["id"], id);
    let config_id = response["body"]["config"]["id"].as_str().unwrap();
    assert_ne!(
        config_id, id,
        "baseline upsert keeps row ID but assigns new config ID"
    );
    h.label(config_id, "dashboard-upsert-config-id");
    let response = h
        .step(
            "dashboard clone",
            Some(2),
            "POST",
            &format!("{path}/clone"),
            Value::Null,
            &[],
            201,
        )
        .await;
    let clone_id = response["body"]["id"].as_str().unwrap().to_owned();
    h.label(&clone_id, "dashboard-clone");
    h.step(
        "dashboard duplicate clone",
        Some(2),
        "POST",
        &format!("{path}/clone"),
        Value::Null,
        &[],
        422,
    )
    .await;
    h.step(
        "dashboard clone foreign",
        Some(3),
        "POST",
        &format!("{path}/clone"),
        Value::Null,
        &[],
        403,
    )
    .await;
    h.step(
        "dashboard nonadmin lock",
        Some(2),
        "POST",
        &format!("/v1/admin/dashboards/{id}/lock"),
        json!({"locked":true}),
        &[],
        403,
    )
    .await;
    h.step(
        "dashboard admin lock",
        Some(1),
        "POST",
        &format!("/v1/admin/dashboards/{id}/lock"),
        json!({"locked":true}),
        &[],
        200,
    )
    .await;
    h.step(
        "dashboard locked write",
        Some(2),
        "PUT",
        &path,
        json!({"name":"no"}),
        &[],
        403,
    )
    .await;
    h.step(
        "dashboard locked delete",
        Some(2),
        "DELETE",
        &path,
        Value::Null,
        &[],
        403,
    )
    .await;
    h.step(
        "dashboard admin edit",
        Some(0),
        "PUT",
        &format!("/v1/admin/dashboards/{id}"),
        json!({"name":"Admin edit","config":null}),
        &[],
        200,
    )
    .await;
    h.step(
        "dashboard personal restore",
        Some(0),
        "POST",
        &format!("/v1/admin/dashboards/{id}/restore-default"),
        Value::Null,
        &[],
        422,
    )
    .await;
    h.step(
        "dashboard clone delete",
        Some(2),
        "DELETE",
        &format!("/v1/dashboards/{clone_id}"),
        Value::Null,
        &[],
        204,
    )
    .await;
    for (slug, config) in [
        ("scalar", json!(7)),
        ("null-config", Value::Null),
        ("array-config", json!([])),
    ] {
        let res = h
            .step(
                &format!("dashboard config {slug}"),
                Some(2),
                "POST",
                "/v1/dashboards",
                json!({"slug":slug,"name":"","config":config}),
                &[],
                201,
            )
            .await;
        h.label(
            res["body"]["id"].as_str().unwrap(),
            &format!("dashboard-{slug}"),
        );
    }
    seed_defaults(&h.fixture.pool).await.unwrap();
    crate::services::chart_registry::seed_defaults(&h.fixture.pool)
        .await
        .unwrap();
    let chart_rows: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id,slug FROM panda.charts WHERE owner_id IS NULL ORDER BY slug")
            .fetch_all(&h.fixture.pool)
            .await
            .unwrap();
    for (id, slug) in &chart_rows {
        h.label(&id.to_string(), &format!("chart-system-{slug}"));
    }
    h.checkpoint("seeded defaults").await;
    let before = h.state().await;
    seed_defaults(&h.fixture.pool).await.unwrap();
    crate::services::chart_registry::seed_defaults(&h.fixture.pool)
        .await
        .unwrap();
    assert_eq!(
        before,
        h.state().await,
        "repeat seed must be exact row equality"
    );
    h.checkpoint("repeat seed exact equality").await;
    let system = "00000000-0000-0000-0000-000000000001";
    h.step(
        "dashboard system fetch",
        Some(3),
        "GET",
        &format!("/v1/dashboards/{system}"),
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "dashboard system by slug",
        Some(2),
        "GET",
        "/v1/dashboards/by-slug/dashboard",
        Value::Null,
        &[],
        200,
    )
    .await;
    let res = h
        .step(
            "dashboard override",
            Some(2),
            "POST",
            "/v1/dashboards",
            json!({"slug":"dashboard","name":"Override","config":{"widgets":[]}}),
            &[],
            201,
        )
        .await;
    h.label(res["body"]["id"].as_str().unwrap(), "dashboard-override");
    h.step(
        "dashboard preference own",
        Some(2),
        "GET",
        "/v1/dashboards/by-slug/dashboard",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "dashboard preference foreign",
        Some(3),
        "GET",
        "/v1/dashboards/by-slug/dashboard",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "dashboard system denied",
        Some(2),
        "PUT",
        &format!("/v1/dashboards/{system}"),
        json!({"name":"no"}),
        &[],
        403,
    )
    .await;
    h.step(
        "dashboard restore",
        Some(0),
        "POST",
        &format!("/v1/admin/dashboards/{system}/restore-default"),
        Value::Null,
        &[],
        200,
    )
    .await;
    // The lower baseline is advanced, current/future baselines and personal metadata survive.
    sqlx::query("UPDATE panda.dashboards SET baseline_revision=CASE slug WHEN 'battery' THEN NULL WHEN 'efficiency' THEN 10 ELSE baseline_revision END,name=CASE WHEN slug IN ('battery','efficiency') THEN 'custom-system' ELSE name END WHERE owner_id IS NULL").execute(&h.fixture.pool).await.unwrap();
    h.checkpoint("seed revision inputs").await;
    seed_defaults(&h.fixture.pool).await.unwrap();
    h.checkpoint("seed revision outputs and managed personal composition")
        .await;
    sqlx::query("UPDATE panda.dashboards SET baseline_revision=0,name='pending-refresh' WHERE slug IN ('battery','efficiency') AND owner_id IS NULL").execute(&h.fixture.pool).await.unwrap();
    sqlx::query("ALTER TABLE panda.dashboards ADD CONSTRAINT synthetic_seed_stop CHECK(slug <> 'efficiency') NOT VALID").execute(&h.fixture.pool).await.unwrap();
    assert!(seed_defaults(&h.fixture.pool).await.is_err());
    h.checkpoint("seed failure retains earlier system progress")
        .await;
    sqlx::query("ALTER TABLE panda.dashboards DROP CONSTRAINT synthetic_seed_stop")
        .execute(&h.fixture.pool)
        .await
        .unwrap();
    seed_defaults(&h.fixture.pool).await.unwrap();
    h.step(
        "dashboard list resolved",
        Some(2),
        "GET",
        "/v1/dashboards",
        Value::Null,
        &[],
        200,
    )
    .await;
    let defaults: Vec<Value> =
        serde_json::from_str(include_str!("../../../../charts/defaults.json")).unwrap();
    let config = defaults[0]["definition"].clone();
    let input = json!({"slug":"synthetic-chart","name":"Synthetic chart","description":"fixture","is_enabled":true,"config":config});
    h.step(
        "chart slug validation",
        Some(2),
        "POST",
        "/v1/charts",
        json!({"slug":"UPPER","name":"","config":{}}),
        &[],
        422,
    )
    .await;
    h.step(
        "chart name validation",
        Some(2),
        "POST",
        "/v1/charts",
        json!({"slug":"valid","name":" ","config":{}}),
        &[],
        422,
    )
    .await;
    h.step(
        "chart invalid config",
        Some(2),
        "POST",
        "/v1/charts",
        json!({"slug":"valid","name":"ok","config":{}}),
        &[],
        422,
    )
    .await;
    h.raw_step(
        "chart oversized",
        Some(2),
        "POST",
        "/v1/charts",
        &"x".repeat(65537),
        &[],
        413,
    )
    .await;
    let res = h
        .step(
            "chart create",
            Some(2),
            "POST",
            "/v1/charts",
            input.clone(),
            &[],
            201,
        )
        .await;
    let id = res["body"]["id"].as_str().unwrap().to_owned();
    h.label(&id, "chart-personal");
    let path = format!("/v1/charts/{id}");
    h.step(
        "chart duplicate",
        Some(2),
        "POST",
        "/v1/charts",
        input.clone(),
        &[],
        409,
    )
    .await;
    for (method, body) in [
        ("GET", Value::Null),
        ("PATCH", json!({"name":"x"})),
        ("DELETE", Value::Null),
    ] {
        h.step(
            &format!("chart foreign {method}"),
            Some(3),
            method,
            &path,
            body,
            &[],
            403,
        )
        .await;
    }
    h.step(
        "chart empty name constraint",
        Some(2),
        "PATCH",
        &path,
        json!({"name":"","description":null,"is_enabled":false}),
        &[],
        500,
    )
    .await;
    h.step(
        "chart patch",
        Some(2),
        "PATCH",
        &path,
        json!({"description":null,"is_enabled":false}),
        &[],
        200,
    )
    .await;
    h.step(
        "chart put alias",
        Some(2),
        "PUT",
        &path,
        json!({"isEnabled":true}),
        &[],
        200,
    )
    .await;
    h.step(
        "chart placements",
        Some(2),
        "PATCH",
        &format!("{path}/placements"),
        json!({"placements":[{"dashboardSlug":"battery"},{"dashboardSlug":"battery"}]}),
        &[],
        200,
    )
    .await;
    h.step(
        "chart bad placements",
        Some(2),
        "PATCH",
        &format!("{path}/placements"),
        json!({"placements":vec![json!({"dashboardSlug":"battery"});21]}),
        &[],
        422,
    )
    .await;
    h.step(
        "chart list",
        Some(2),
        "GET",
        "/v1/charts",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "chart effective",
        Some(2),
        "GET",
        "/v1/charts/effective",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "chart effective filter",
        Some(2),
        "GET",
        "/v1/charts/effective?dashboard_slug=battery",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "chart effective empty",
        Some(2),
        "GET",
        "/v1/charts/effective?dashboard_slug=missing",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "chart source manifest",
        Some(2),
        "GET",
        "/v1/chart-sources",
        Value::Null,
        &[],
        200,
    )
    .await;
    let res = h
        .step(
            "chart clone",
            Some(2),
            "POST",
            &format!("{path}/clone"),
            json!({"slug":"chart-clone","name":"Clone"}),
            &[],
            201,
        )
        .await;
    let clone = res["body"]["id"].as_str().unwrap().to_owned();
    h.label(&clone, "chart-clone");
    h.step(
        "chart duplicate clone",
        Some(2),
        "POST",
        &format!("{path}/clone"),
        json!({"slug":"chart-clone","name":"Clone"}),
        &[],
        409,
    )
    .await;
    h.step(
        "chart no base reset",
        Some(2),
        "POST",
        &format!("{path}/reset"),
        Value::Null,
        &[],
        404,
    )
    .await;
    h.step(
        "chart nonadmin lock",
        Some(2),
        "PUT",
        &format!("/v1/admin/charts/{id}/lock"),
        json!({"locked":true}),
        &[],
        403,
    )
    .await;
    h.step(
        "chart admin lock",
        Some(1),
        "PATCH",
        &format!("/v1/admin/charts/{id}/lock"),
        json!({"locked":true}),
        &[],
        200,
    )
    .await;
    h.step(
        "chart locked patch",
        Some(2),
        "PATCH",
        &path,
        json!({"name":"no"}),
        &[],
        403,
    )
    .await;
    h.step(
        "chart admin patch",
        Some(0),
        "PATCH",
        &format!("/v1/admin/charts/{id}"),
        json!({"name":"Admin"}),
        &[],
        200,
    )
    .await;
    h.step(
        "chart delete",
        Some(2),
        "DELETE",
        &format!("/v1/charts/{clone}"),
        Value::Null,
        &[],
        204,
    )
    .await;
    let slug = defaults[0]["slug"].as_str().unwrap();
    let sid = chart_rows.iter().find(|(_, s)| s == slug).unwrap().0;
    let res = h
        .step(
            "chart override",
            Some(2),
            "POST",
            "/v1/charts",
            json!({"slug":slug,"name":"Override","config":defaults[0]["definition"]}),
            &[],
            201,
        )
        .await;
    let oid = res["body"]["id"].as_str().unwrap().to_owned();
    h.label(&oid, "chart-override");
    h.step(
        "chart override effective",
        Some(2),
        "GET",
        "/v1/charts/effective",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "chart override reset",
        Some(2),
        "POST",
        &format!("/v1/charts/{oid}/reset"),
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "chart system restore",
        Some(0),
        "POST",
        &format!("/v1/admin/charts/{sid}/restore"),
        Value::Null,
        &[],
        200,
    )
    .await;
    // Concurrent uniqueness: identical inputs make winner identity irrelevant.
    let mut concurrent = input;
    concurrent["slug"] = json!("concurrent-chart");
    let raw = concurrent.to_string();
    let (a, b) = tokio::join!(
        h.request(Some(2), "POST", "/v1/charts", &raw, &[]),
        h.request(Some(2), "POST", "/v1/charts", &raw, &[])
    );
    let mut responses = vec![a, b];
    responses.sort_by_key(|v| v["status"].as_u64().unwrap());
    assert_eq!(responses[0]["status"], 201);
    assert_eq!(responses[1]["status"], 409);
    h.label(
        responses[0]["body"]["id"].as_str().unwrap(),
        "chart-concurrent",
    );
    h.observations.push(
        json!({"name":"chart concurrent unique","responses":responses,"persisted":h.state().await}),
    );
}
