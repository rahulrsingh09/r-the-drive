use super::*;

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn workflow_contract_artwork() {
    let f = Fixture::new().await;
    f.seed().await;
    let mut e = Evidence::default();
    let v = id(101);
    e.record(
        &f,
        "empty",
        f.request(
            Some(1),
            "GET",
            &format!("/v1/vehicles/{v}/images"),
            Value::Null,
        )
        .await,
    )
    .await;
    let root = std::path::Path::new(&f.state.config.vehicle_image_cache_dir);
    std::fs::create_dir_all(root).unwrap();
    let bytes = b"synthetic image bytes";
    std::fs::write(root.join("valid.webp"), bytes).unwrap();
    let hash = hex::encode(Sha256::digest(bytes));
    for (n, placement, design, size, metadata, overlays) in [
        (401, "side", Some("dark"), "large", json!({}), json!([])),
        (
            402,
            "side",
            Some("light"),
            "small",
            json!({"mirror_key":"valid.webp","mirror_relpath":"valid.webp","sha256":hash,"mirror_status":"ready","mime_type":"image/webp"}),
            json!([]),
        ),
        (
            403,
            "top",
            None,
            "large",
            json!({"mirror_key":"pending.webp","mirror_status":"pending"}),
            json!([]),
        ),
        (
            404,
            "rear",
            Some("dark"),
            "large",
            json!({"mirror_key":"missing.webp","mirror_relpath":"absent.webp","sha256":hash,"mirror_status":"ready"}),
            json!([]),
        ),
        (
            405,
            "front",
            Some("dark"),
            "large",
            json!({"mirror_key":"bad.webp","mirror_relpath":"valid.webp","sha256":"bad","mirror_status":"ready","overlay_mirrors":[{"source_url":"https://synthetic.invalid/overlay","mirror_key":"overlay.webp","mirror_relpath":"valid.webp","sha256":hash,"mirror_status":"ready"}]}),
            json!([{ "url":"https://synthetic.invalid/overlay","placement":"roof","design":"light"}]),
        ),
    ] {
        sqlx::query("INSERT INTO panda.vehicle_images(id,vehicle_id,placement,design,size,url,metadata,overlays,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'2026-01-01Z','2026-01-01Z')").bind(id(n)).bind(v).bind(placement).bind(design).bind(size).bind(format!("https://synthetic.invalid/{n}")).bind(metadata).bind(overlays).execute(&f.state.pool).await.unwrap();
    }
    e.record(
        &f,
        "selection",
        f.request(
            Some(1),
            "GET",
            &format!("/v1/vehicles/{v}/images"),
            Value::Null,
        )
        .await,
    )
    .await;
    for n in [1, 2, 3, 4, 5] {
        for key in [
            "valid.webp",
            "pending.webp",
            "missing.webp",
            "bad.webp",
            "overlay.webp",
            "unknown.webp",
            "invalid!key",
        ] {
            e.record(
                &f,
                &format!("asset-{n}-{key}"),
                f.request(
                    Some(n),
                    "GET",
                    &format!("/v1/vehicle-image-cache/{v}/{key}"),
                    Value::Null,
                )
                .await,
            )
            .await;
        }
    }
    mark_vehicle_artwork_repair_attempt(&f.state.pool, v).await;
    refresh_vehicle_artwork_cache_state(
        &f.state.pool,
        &f.state.config,
        v,
        Some("synthetic diagnostic that must be redacted"),
    )
    .await;
    e.record(
        &f,
        "cache-failure",
        f.request(
            Some(1),
            "GET",
            &format!("/v1/vehicles/{v}/images"),
            Value::Null,
        )
        .await,
    )
    .await;
    refresh_vehicle_artwork_cache_state(&f.state.pool, &f.state.config, v, None).await;
    e.record(
        &f,
        "cache-retry",
        f.request(
            Some(1),
            "GET",
            &format!("/v1/vehicles/{v}/images"),
            Value::Null,
        )
        .await,
    )
    .await;
    for n in [1, 5] {
        for suffix in ["remirror", "cache/purge"] {
            e.record(
                &f,
                &format!("admin-{n}-{suffix}"),
                f.request(
                    Some(n),
                    "POST",
                    &format!("/v1/admin/vehicles/{v}/images/{suffix}"),
                    Value::Null,
                )
                .await,
            )
            .await;
        }
    }
    // Local cache corruption is a restoring response; policy continues to deny outbound repair.
    std::fs::write(root.join("valid.webp"), b"corrupt").unwrap();
    e.push(
        "corrupt-cache",
        f.request(
            Some(1),
            "GET",
            &format!("/v1/vehicle-image-cache/{v}/valid.webp"),
            Value::Null,
        )
        .await,
    );
    f.cleanup().await;
    e.finish("artwork", include_str!("artwork.json"));
}
