use super::*;
fn etag(response: &Value) -> String {
    response["headers"]["etag"].as_str().unwrap().to_owned()
}

pub(super) async fn exercise(h: &mut Harness) {
    h.step(
        "theme initial preferences",
        Some(2),
        "GET",
        "/v2/auth/preferences/theme",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "theme initial catalog",
        Some(2),
        "GET",
        "/v2/themes/catalog",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "theme unknown builtin first",
        Some(2),
        "POST",
        "/v2/themes",
        json!({"name":"","baseThemeId":"unknown"}),
        &[],
        422,
    )
    .await;
    h.step(
        "theme empty name",
        Some(2),
        "POST",
        "/v2/themes",
        json!({"name":"  ","baseThemeId":"classic"}),
        &[],
        422,
    )
    .await;
    h.step(
        "theme long name",
        Some(2),
        "POST",
        "/v2/themes",
        json!({"name":"x".repeat(81),"baseThemeId":"classic"}),
        &[],
        422,
    )
    .await;
    h.step(
        "theme unknown field",
        Some(2),
        "POST",
        "/v2/themes",
        json!({"name":"Name","baseThemeId":"classic","extra":true}),
        &[],
        422,
    )
    .await;
    h.raw_step(
        "theme malformed",
        Some(2),
        "POST",
        "/v2/themes",
        "{",
        &[],
        400,
    )
    .await;
    h.raw_step(
        "theme oversized",
        Some(2),
        "POST",
        "/v2/themes",
        &"x".repeat(98305),
        &[],
        413,
    )
    .await;
    let created = h
        .step(
            "theme create",
            Some(2),
            "POST",
            "/v2/themes",
            json!({"name":"  Synthetic Theme  ","baseThemeId":"classic"}),
            &[],
            200,
        )
        .await;
    let id = created["body"]["themeId"].as_str().unwrap().to_owned();
    h.label(&id, "theme-primary");
    let path = format!("/v2/themes/{id}");
    let mut version = etag(&created);
    h.step(
        "theme empty revisions",
        Some(2),
        "GET",
        &path,
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "theme foreign",
        Some(3),
        "GET",
        &path,
        Value::Null,
        &[],
        404,
    )
    .await;
    h.step(
        "theme missing etag",
        Some(2),
        "POST",
        &format!("{path}/revisions"),
        json!({"definition":{}}),
        &[],
        409,
    )
    .await;
    h.step(
        "theme foreign mutation",
        Some(3),
        "POST",
        &format!("{path}/revisions"),
        json!({"definition":{}}),
        &[("if-match", version.clone())],
        404,
    )
    .await;
    for (name, definition) in [
        ("null", Value::Null),
        ("empty", json!({})),
        (
            "bad color",
            json!({"theme":"classic","tokens":{"accent":{"dark":"red"}}}),
        ),
        (
            "too big",
            json!({"theme":"classic","padding":"x".repeat(65536)}),
        ),
    ] {
        h.step(
            &format!("theme definition {name}"),
            Some(2),
            "POST",
            &format!("{path}/revisions"),
            json!({"definition":definition}),
            &[("if-match", version.clone())],
            422,
        )
        .await;
    }
    let saved = h
        .step(
            "theme first revision",
            Some(2),
            "POST",
            &format!("{path}/revisions"),
            json!({"definition":{"theme":"classic"}}),
            &[("if-match", version.clone())],
            200,
        )
        .await;
    version = etag(&saved);
    h.step(
        "theme publish missing",
        Some(2),
        "POST",
        &format!("{path}/revisions/99/publish"),
        json!({}),
        &[("if-match", version.clone())],
        404,
    )
    .await;
    h.step(
        "theme rollback unpublished",
        Some(2),
        "POST",
        &format!("{path}/rollback"),
        json!({"revision":1}),
        &[("if-match", version.clone())],
        422,
    )
    .await;
    h.step(
        "theme publish apply rollback",
        Some(2),
        "POST",
        &format!("{path}/revisions/1/publish"),
        json!({"apply":true}),
        &[("if-match", version.clone())],
        409,
    )
    .await;
    let published = h
        .step(
            "theme publish",
            Some(2),
            "POST",
            &format!("{path}/revisions/1/publish"),
            json!({}),
            &[("if-match", version.clone())],
            200,
        )
        .await;
    version = etag(&published);
    let published = h
        .step(
            "theme republish",
            Some(2),
            "POST",
            &format!("{path}/revisions/1/publish"),
            json!({}),
            &[("if-match", version.clone())],
            200,
        )
        .await;
    version = etag(&published);
    let applied = h
        .step(
            "theme publish and apply",
            Some(2),
            "POST",
            &format!("{path}/revisions/1/publish"),
            json!({"apply":true}),
            &[
                ("if-match", version.clone()),
                (
                    "x-theme-preferences-if-match",
                    "\"theme-preferences-1\"".into(),
                ),
            ],
            200,
        )
        .await;
    version = etag(&applied);
    h.step(
        "theme published catalog",
        Some(2),
        "GET",
        "/v2/themes/catalog",
        Value::Null,
        &[],
        200,
    )
    .await;
    let prefs = h
        .step(
            "theme custom preferences",
            Some(2),
            "GET",
            "/v2/auth/preferences/theme",
            Value::Null,
            &[],
            200,
        )
        .await;
    let prefs=h.step("theme set custom mode",Some(2),"PUT","/v2/auth/preferences/theme",json!({"schemaVersion":2,"mode":"system","selection":{"kind":"custom","themeId":id,"revision":1}}),&[("if-match",etag(&prefs))],200).await;
    h.step(
        "theme preference schema",
        Some(2),
        "PUT",
        "/v2/auth/preferences/theme",
        json!({"schemaVersion":1,"mode":"bad","selection":{"kind":"builtin","themeId":"no"}}),
        &[],
        422,
    )
    .await;
    h.step(
        "theme preference mode",
        Some(2),
        "PUT",
        "/v2/auth/preferences/theme",
        json!({"schemaVersion":2,"mode":"bad","selection":{"kind":"builtin","themeId":"no"}}),
        &[],
        422,
    )
    .await;
    h.step(
        "theme preference stale before builtin",
        Some(2),
        "PUT",
        "/v2/auth/preferences/theme",
        json!({"schemaVersion":2,"mode":"dark","selection":{"kind":"builtin","themeId":"no"}}),
        &[],
        409,
    )
    .await;
    h.step(
        "theme preference invalid builtin",
        Some(2),
        "PUT",
        "/v2/auth/preferences/theme",
        json!({"schemaVersion":2,"mode":"dark","selection":{"kind":"builtin","themeId":"no"}}),
        &[("if-match", etag(&prefs))],
        422,
    )
    .await;
    h.step("theme preference foreign custom",Some(3),"PUT","/v2/auth/preferences/theme",json!({"schemaVersion":2,"mode":"dark","selection":{"kind":"custom","themeId":id,"revision":1}}),&[("if-match","\"theme-preferences-1\"".into())],422).await;
    let saved = h
        .step(
            "theme second revision",
            Some(2),
            "POST",
            &format!("{path}/revisions"),
            json!({"definition":{"theme":"classic","series":{"series-16":{"dark":"#112233"}}}}),
            &[("if-match", version.clone())],
            200,
        )
        .await;
    version = etag(&saved);
    let published = h
        .step(
            "theme second publish apply",
            Some(2),
            "POST",
            &format!("{path}/revisions/2/publish"),
            json!({"apply":true}),
            &[
                ("if-match", version.clone()),
                ("x-theme-preferences-if-match", etag(&prefs)),
            ],
            200,
        )
        .await;
    version = etag(&published);
    let prefs = h
        .step(
            "theme latest preferences",
            Some(2),
            "GET",
            "/v2/auth/preferences/theme",
            Value::Null,
            &[],
            200,
        )
        .await;
    h.step(
        "theme rollback",
        Some(2),
        "POST",
        &format!("{path}/rollback"),
        json!({"revision":1}),
        &[
            ("if-match", version.clone()),
            ("x-theme-preferences-if-match", etag(&prefs)),
        ],
        200,
    )
    .await;
    h.step(
        "theme all revisions",
        Some(2),
        "GET",
        &path,
        Value::Null,
        &[],
        200,
    )
    .await;
    let retired = h
        .step(
            "theme retire",
            Some(2),
            "DELETE",
            &path,
            Value::Null,
            &[("if-match", version.clone())],
            200,
        )
        .await;
    version = etag(&retired);
    h.step(
        "theme retirement fallback",
        Some(2),
        "GET",
        "/v2/auth/preferences/theme",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "theme retired catalog",
        Some(2),
        "GET",
        "/v2/themes/catalog",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "theme retired detail",
        Some(2),
        "GET",
        &path,
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "theme retire repeat",
        Some(2),
        "DELETE",
        &path,
        Value::Null,
        &[("if-match", version.clone())],
        409,
    )
    .await;
    for (suffix, body) in [
        ("revisions", json!({"definition":{}})),
        ("revisions/1/publish", json!({})),
        ("rollback", json!({"revision":1})),
    ] {
        h.step(
            &format!("theme retired {suffix}"),
            Some(2),
            "POST",
            &format!("{path}/{suffix}"),
            body,
            &[("if-match", version.clone())],
            409,
        )
        .await;
    }
    sqlx::query("UPDATE panda.user_preferences SET theme_selection_kind='custom', theme_palette='classic', theme_builtin_id='classic', theme_custom_id=$1, theme_custom_revision=1 WHERE user_id=$2")
        .bind(id.parse::<Uuid>().unwrap()).bind(Uuid::from_u128(103)).execute(&h.fixture.pool).await.unwrap();
    h.step(
        "theme unavailable custom fallback",
        Some(2),
        "GET",
        "/v2/auth/preferences/theme",
        Value::Null,
        &[],
        200,
    )
    .await;
    let prefs = h
        .step(
            "theme builtin before",
            Some(2),
            "GET",
            "/v2/auth/preferences/theme",
            Value::Null,
            &[],
            200,
        )
        .await;
    h.step(
        "theme builtin selection",
        Some(2),
        "PUT",
        "/v2/auth/preferences/theme",
        json!({"schemaVersion":2,"mode":"light","selection":{"kind":"builtin","themeId":"rad"}}),
        &[("if-match", etag(&prefs))],
        200,
    )
    .await;
    // Twenty active themes serialize under the user row lock; retired themes do not count.
    for index in 0..20 {
        let row = h
            .step(
                &format!("theme capacity {index}"),
                Some(3),
                "POST",
                "/v2/themes",
                json!({"name":format!("Capacity {index:02}"),"baseThemeId":"rad"}),
                &[],
                200,
            )
            .await;
        h.label(
            row["body"]["themeId"].as_str().unwrap(),
            &format!("theme-capacity-{index}"),
        );
    }
    h.step(
        "theme over capacity",
        Some(3),
        "POST",
        "/v2/themes",
        json!({"name":"Overflow","baseThemeId":"rad"}),
        &[],
        422,
    )
    .await;
    h.step(
        "timezone initial",
        Some(2),
        "GET",
        "/v1/settings/timezone",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "timezone user denied before validation",
        Some(2),
        "PUT",
        "/v1/settings/timezone",
        json!({"timezone":"invalid"}),
        &[],
        403,
    )
    .await;
    h.step(
        "timezone invalid",
        Some(1),
        "PUT",
        "/v1/settings/timezone",
        json!({"timezone":"invalid"}),
        &[],
        422,
    )
    .await;
    for timezone in [" UTC ", "America/Los_Angeles", "Etc/GMT+12"] {
        h.step(
            &format!("timezone {timezone}"),
            Some(1),
            "PUT",
            "/v1/settings/timezone",
            json!({"timezone":timezone}),
            &[],
            200,
        )
        .await;
    }
    h.step(
        "update check initial",
        Some(2),
        "GET",
        "/v1/settings/update-check",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "update check user denied",
        Some(2),
        "PUT",
        "/v1/settings/update-check",
        json!({"enabled":true,"frequency":"daily"}),
        &[],
        403,
    )
    .await;
    for frequency in ["hourly", "daily", "weekly", "monthly"] {
        h.step(
            &format!("update check {frequency}"),
            Some(0),
            "PUT",
            "/v1/settings/update-check",
            json!({"enabled":frequency!="daily","frequency":frequency}),
            &[],
            200,
        )
        .await;
    }
    h.step(
        "update check unknown frequency",
        Some(0),
        "PUT",
        "/v1/settings/update-check",
        json!({"enabled":true,"frequency":"annually"}),
        &[],
        422,
    )
    .await;
    h.step(
        "update check unknown field",
        Some(0),
        "PUT",
        "/v1/settings/update-check",
        json!({"enabled":true,"frequency":"daily","extra":true}),
        &[],
        422,
    )
    .await;
    h.step(
        "security status denied",
        Some(2),
        "GET",
        "/v1/admin/security/status",
        Value::Null,
        &[],
        403,
    )
    .await;
    h.step(
        "security status admin",
        Some(1),
        "GET",
        "/v1/admin/security/status",
        Value::Null,
        &[],
        200,
    )
    .await;
    h.step(
        "authentication settings denied",
        Some(2),
        "GET",
        "/v1/settings/authentication",
        Value::Null,
        &[],
        403,
    )
    .await;
    // Atomic settings audit: an app-role constraint failure must roll back the setting too.
    sqlx::query("ALTER TABLE panda.security_events ADD CONSTRAINT synthetic_reject_settings CHECK(event_type <> 'application_timezone_updated') NOT VALID").execute(&h.fixture.pool).await.unwrap();
    h.step(
        "timezone audit failure rollback",
        Some(0),
        "PUT",
        "/v1/settings/timezone",
        json!({"timezone":"UTC"}),
        &[],
        500,
    )
    .await;
    sqlx::query("ALTER TABLE panda.security_events DROP CONSTRAINT synthetic_reject_settings")
        .execute(&h.fixture.pool)
        .await
        .unwrap();
}
