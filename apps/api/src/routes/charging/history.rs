use super::*;

struct HistoryPlan {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
    offset: i64,
    local_day: Option<String>,
}
impl HistoryPlan {
    fn resolve(input: SessionListParams) -> Self {
        let from = input
            .from
            .unwrap_or_else(|| Utc::now() - chrono::Duration::days(90));
        let to = input.to.unwrap_or_else(Utc::now);
        let limit = input.per_page.or(input.limit).unwrap_or(50).clamp(1, 200);
        let page = input.page.unwrap_or(1).max(1);
        Self {
            from,
            to,
            limit,
            offset: input.offset.unwrap_or((page - 1) * limit).max(0),
            local_day: input.session_day_local,
        }
    }
    fn project(&self, rows: Vec<SessionRow>, total: i64) -> serde_json::Value {
        serde_json::json!({"data":rows,"items":rows,"total":total,"limit":self.limit,
            "offset":self.offset,"page":self.offset/self.limit+1,"per_page":self.limit})
    }
}

pub(super) async fn history(
    state: &AppState,
    user_id: Uuid,
    vehicle_id: Uuid,
    p: SessionListParams,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_membership(&state.pool, user_id, vehicle_id).await?;
    let plan = HistoryPlan::resolve(p);
    let HistoryPlan {
        from,
        to,
        limit,
        offset,
        ref local_day,
    } = plan;

    let mut tx = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query_as::<_, SessionRow>(sqlx::AssertSqlSafe(c::session_query(
        c::SessionProjection::History,
    )))
    .bind(vehicle_id)
    .bind(from)
    .bind(to)
    .bind(limit)
    .bind(offset)
    .bind(local_day)
    .fetch_all(&mut *tx)
    .await?;

    let total: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM panda.charge_sessions cs \
         WHERE cs.vehicle_id=$1 AND cs.started_at>=$2 AND cs.started_at<=$3 \
           AND ($4 IS NULL OR TO_CHAR((cs.started_at AT TIME ZONE COALESCE((SELECT value FROM panda.system_config WHERE key = 'app_timezone'), 'UTC'))::date, 'YYYY-MM-DD') = $4)"
    )
    .bind(vehicle_id)
    .bind(from)
    .bind(to)
    .bind(local_day)
    .fetch_one(&mut *tx)
    .await?;

    tx.rollback().await?;

    Ok(Json(plan.project(rows, total)))
}

pub(super) async fn detail(
    state: &AppState,
    user_id: Uuid,
    vehicle_id: Uuid,
    id: Uuid,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_membership(&state.pool, user_id, vehicle_id).await?;

    let session = sqlx::query_as::<_, SessionRow>(sqlx::AssertSqlSafe(c::session_query(
        c::SessionProjection::Detail,
    )))
    .bind(id)
    .bind(vehicle_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;

    let curve = load_curve(
        &state.pool,
        vehicle_id,
        session.id,
        session.started_at,
        session.ended_at,
        session.charger_type.as_deref(),
    )
    .await?;

    Ok(Json(serde_json::json!({
        "session": session,
        "curve": curve,
    })))
}
