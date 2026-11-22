enum Reduction {
    Sum(&'static str),
    ZeroSum(&'static str),
    ConditionalEnergy(&'static str),
    Maximum(&'static str),
    Count(&'static str),
    CountWhere(&'static str),
}

impl Reduction {
    fn sql(&self) -> String {
        match self {
            Self::Sum(expression) => format!("SUM({expression})"),
            Self::ZeroSum(expression) => format!("COALESCE(SUM({expression}),0)"),
            Self::ConditionalEnergy(condition) => {
                format!("COALESCE(SUM(CASE WHEN {condition} THEN energy_kwh ELSE 0 END),0)")
            }
            Self::Maximum(expression) => format!("MAX({expression})"),
            Self::Count(expression) => format!("COUNT({expression})"),
            Self::CountWhere(condition) => format!("COUNT(*) FILTER (WHERE {condition})"),
        }
    }
}

pub(super) fn totals() -> String {
    use Reduction::*;
    let measures = [
        ("total_kwh", ZeroSum("energy_kwh")),
        ("session_count", Count("*")),
        ("home_kwh", ConditionalEnergy("is_home")),
        ("away_kwh", ConditionalEnergy("is_home = false")),
        ("unknown_location_kwh", ConditionalEnergy("is_home IS NULL")),
        ("ac_kwh", ConditionalEnergy("derived_type='ac'")),
        ("ac_l2_kwh", ConditionalEnergy("derived_type='ac_l2'")),
        ("dc_kwh", ConditionalEnergy("derived_type='dc'")),
        ("typed_session_count", Count("derived_type")),
        ("max_charge_limit_pct", Maximum("charge_limit")),
        ("max_charge_rate_kw", Maximum("rate_kw")),
        (
            "total_energy_used_kwh",
            Sum("GREATEST(energy_kwh,COALESCE(energy_used_wh/1000.0,0))"),
        ),
        ("total_cost_usd", Sum("cost_usd")),
        ("known_cost_session_count", Count("cost_usd")),
        ("unknown_cost_session_count", CountWhere("cost_usd IS NULL")),
        ("free_session_count", CountWhere("is_free_session")),
        ("total_range_added_km", Sum("range_added_km")),
        ("rivian_paid_total_usd", Sum("rivian_paid_total")),
    ];
    let projection = measures
        .into_iter()
        .map(|(alias, reduction)| format!("{} AS {alias}", reduction.sql()))
        .collect::<Vec<_>>()
        .join(",");
    let charger_sql = crate::charger_type_sql!("cs");
    format!("WITH normalized AS (
        SELECT COALESCE(cs.kwh_added,cs.energy_added_wh/1000.0) AS energy_kwh,
            cs.is_home,cs.charger_type,cs.network_vendor,cs.charge_limit,cs.cost_usd,cs.energy_used_wh,cs.is_free_session,cs.range_added_km,cs.rivian_paid_total,
            COALESCE(cs.max_charge_rate_kw,cs.avg_charge_rate_kw,CASE WHEN cs.duration_minutes>0
                THEN COALESCE(cs.kwh_added,cs.energy_added_wh/1000.0)/(cs.duration_minutes::float8/60.0) END) AS rate_kw,
            {charger_sql} AS derived_type
        FROM panda.charge_sessions cs WHERE cs.vehicle_id=$1 AND cs.started_at>=$2 AND cs.started_at<=$3
    ), typed AS (
        SELECT * FROM normalized
    ) SELECT {projection} FROM typed")
}
