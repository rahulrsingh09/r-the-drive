use std::collections::BTreeSet;

use serde::Deserialize;
use uuid::Uuid;

use crate::errors::AppError;

/// The tag filters shared by trip history and the trip-derived dashboards.
/// Keeping parsing here prevents equivalent endpoints from slowly diverging.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TripTagMatch {
    All,
    Any,
}

#[derive(Debug, Clone)]
pub struct TripTagFilter {
    pub tag_ids: Option<Vec<Uuid>>,
    pub match_all: bool,
    pub untagged: bool,
}

impl Default for TripTagFilter {
    fn default() -> Self {
        Self {
            tag_ids: None,
            match_all: true,
            untagged: false,
        }
    }
}

impl TripTagFilter {
    pub fn is_active(&self) -> bool {
        self.tag_ids.is_some() || self.untagged
    }
}

#[derive(Default)]
struct ParsedTagIds {
    unique: BTreeSet<Uuid>,
    supplied_count: usize,
}

impl ParsedTagIds {
    fn parse(raw: Option<&str>) -> Result<Self, AppError> {
        raw.unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .try_fold(Self::default(), |mut parsed, token| {
                let id = Uuid::parse_str(token).map_err(|_| {
                    AppError::Validation("tag_ids must be a comma-separated list of UUIDs".into())
                })?;
                parsed.unique.insert(id);
                parsed.supplied_count += 1;
                Ok(parsed)
            })
    }

    fn into_ids(self) -> Result<Vec<Uuid>, AppError> {
        // Complete syntax validation before reporting set-level errors.
        if self.supplied_count != self.unique.len() {
            return Err(AppError::Validation(
                "tag_ids must not contain duplicates".into(),
            ));
        }
        if self.supplied_count > 50 {
            return Err(AppError::Validation(
                "tag_ids may contain at most 50 IDs".into(),
            ));
        }
        Ok(self.unique.into_iter().collect())
    }
}

enum TripCohort {
    AllTrips,
    Tagged(Vec<Uuid>),
    Untagged,
}

impl TripCohort {
    fn from_query(raw_tag_ids: Option<&str>, untagged: bool) -> Result<Self, AppError> {
        let ids = ParsedTagIds::parse(raw_tag_ids)?.into_ids()?;
        match (ids.is_empty(), untagged) {
            (true, false) => Ok(Self::AllTrips),
            (true, true) => Ok(Self::Untagged),
            (false, false) => Ok(Self::Tagged(ids)),
            (false, true) => Err(AppError::Validation(
                "untagged cannot be combined with tag_ids".into(),
            )),
        }
    }

    fn into_filter(self, tag_match: Option<TripTagMatch>) -> TripTagFilter {
        let (tag_ids, untagged) = match self {
            Self::AllTrips => (None, false),
            Self::Untagged => (None, true),
            Self::Tagged(ids) => (Some(ids), false),
        };
        TripTagFilter {
            tag_ids,
            match_all: !matches!(tag_match, Some(TripTagMatch::Any)),
            untagged,
        }
    }
}

pub fn parse_tag_filter(
    raw_tag_ids: Option<&str>,
    tag_match: Option<TripTagMatch>,
    raw_untagged: Option<bool>,
) -> Result<TripTagFilter, AppError> {
    TripCohort::from_query(raw_tag_ids, raw_untagged.unwrap_or(false))
        .map(|cohort| cohort.into_filter(tag_match))
}

fn require_complete_tag_membership(tag_ids: &[Uuid], known_count: i64) -> Result<(), AppError> {
    if known_count == tag_ids.len() as i64 {
        Ok(())
    } else {
        Err(AppError::Validation(
            "tag_ids must belong to the selected vehicle".into(),
        ))
    }
}

pub async fn require_known_vehicle_tags(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    filter: &TripTagFilter,
) -> Result<(), AppError> {
    let Some(tag_ids) = filter.tag_ids.as_deref() else {
        return Ok(());
    };
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM panda.trip_tags WHERE vehicle_id=$1 AND id=ANY($2)",
    )
    .bind(vehicle_id)
    .bind(tag_ids)
    .fetch_one(pool)
    .await?;
    require_complete_tag_membership(tag_ids, count)
}

struct AssignmentScope<'a> {
    trip_alias: &'a str,
    tag_ids_param: usize,
}

impl AssignmentScope<'_> {
    fn all_tags(&self) -> String {
        format!(
            "FROM panda.trip_tag_assignments tta WHERE tta.trip_id={}.id",
            self.trip_alias
        )
    }

    fn selected_tags(&self) -> String {
        format!(
            "{} AND tta.tag_id=ANY(${})",
            self.all_tags(),
            self.tag_ids_param
        )
    }
}

/// A static-query-compatible predicate. `alias` is supplied only by route code,
/// while parameter indices are explicit to make SQL binding reviewable.
pub fn sql_predicate(
    alias: &str,
    tag_ids_param: usize,
    match_all_param: usize,
    untagged_param: usize,
) -> String {
    let scope = AssignmentScope {
        trip_alias: alias,
        tag_ids_param,
    };
    let selected = scope.selected_tags();
    let all = scope.all_tags();
    format!(
        " AND (${tag_ids_param}::uuid[] IS NULL OR CASE WHEN ${match_all_param} THEN \
         (SELECT COUNT(DISTINCT tta.tag_id) {selected}) = cardinality(${tag_ids_param}) \
         ELSE EXISTS (SELECT 1 {selected}) END) \
         AND (NOT ${untagged_param} OR NOT EXISTS (SELECT 1 {all}))"
    )
}

#[cfg(test)]
mod analytics_math_contract_tests {
    use super::*;

    fn ids(count: u128) -> String {
        (1..=count)
            .map(|n| Uuid::from_u128(n).to_string())
            .collect::<Vec<_>>()
            .join(",")
    }

    fn validation(raw: &str, untagged: bool) -> String {
        match parse_tag_filter(Some(raw), None, Some(untagged)).unwrap_err() {
            AppError::Validation(message) => message,
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn empty_tokens_are_absent_and_match_mode_is_independent() {
        for raw in [None, Some(""), Some(" , \t,\n, ")] {
            for (mode, match_all) in [
                (None, true),
                (Some(TripTagMatch::All), true),
                (Some(TripTagMatch::Any), false),
            ] {
                for untagged in [false, true] {
                    let filter = parse_tag_filter(raw, mode, Some(untagged)).unwrap();
                    assert!(filter.tag_ids.is_none());
                    assert_eq!(filter.match_all, match_all);
                    assert_eq!(filter.untagged, untagged);
                    assert_eq!(filter.is_active(), untagged);
                }
            }
        }
        assert!(!TripTagFilter::default().is_active());
        assert!(TripTagFilter {
            tag_ids: Some(vec![]),
            ..Default::default()
        }
        .is_active());
    }

    #[test]
    fn uuid_spellings_are_normalized_before_duplicate_detection() {
        let id = Uuid::from_u128(0xabcdef);
        let raw = format!(" {}, {} ", id, id.simple().to_string().to_uppercase());
        assert_eq!(
            validation(&raw, false),
            "tag_ids must not contain duplicates"
        );
        for spelling in [
            id.to_string(),
            id.simple().to_string().to_uppercase(),
            id.urn().to_string(),
            id.braced().to_string(),
        ] {
            assert_eq!(
                parse_tag_filter(Some(&spelling), None, None)
                    .unwrap()
                    .tag_ids,
                Some(vec![id])
            );
        }
    }

    #[test]
    fn fifty_ids_are_allowed_and_fifty_one_are_rejected() {
        assert_eq!(
            parse_tag_filter(Some(&ids(50)), None, None)
                .unwrap()
                .tag_ids
                .unwrap()
                .len(),
            50
        );
        assert_eq!(
            validation(&ids(51), false),
            "tag_ids may contain at most 50 IDs"
        );
    }

    #[test]
    fn validation_precedence_is_syntax_duplicates_limit_then_conflict() {
        let duplicate_oversize = format!("{},{}", ids(51), Uuid::from_u128(1));
        assert_eq!(
            validation(&format!("{duplicate_oversize},invalid"), true),
            "tag_ids must be a comma-separated list of UUIDs"
        );
        assert_eq!(
            validation(&duplicate_oversize, true),
            "tag_ids must not contain duplicates"
        );
        assert_eq!(
            validation(&ids(51), true),
            "tag_ids may contain at most 50 IDs"
        );
        assert_eq!(
            validation(&ids(1), true),
            "untagged cannot be combined with tag_ids"
        );
    }

    #[test]
    fn canonical_order_is_independent_of_permutation_and_blank_tokens() {
        for count in 1..=50 {
            let expected: Vec<_> = (1..=count).map(Uuid::from_u128).collect();
            for shift in [0, count / 2, count - 1] {
                let mut shuffled = expected.clone();
                shuffled.rotate_left(shift as usize);
                shuffled.reverse();
                let raw = shuffled
                    .iter()
                    .map(|id| format!(" , {id} , "))
                    .collect::<Vec<_>>()
                    .join(",");
                let filter = parse_tag_filter(Some(&raw), Some(TripTagMatch::Any), None).unwrap();
                assert_eq!(filter.tag_ids, Some(expected.clone()));
                assert!(!filter.match_all);
                assert!(filter.is_active());
            }
        }
    }

    #[test]
    fn tag_values_cannot_supply_sql_syntax() {
        for raw in [
            "' OR TRUE --",
            "NULL",
            "[]",
            "$1",
            "1); DROP TABLE panda.trips; --",
        ] {
            assert_eq!(
                validation(raw, false),
                "tag_ids must be a comma-separated list of UUIDs"
            );
        }
    }

    #[test]
    fn membership_requires_an_exact_vehicle_scoped_count() {
        for count in 0..=50 {
            let tags: Vec<_> = (1..=count).map(Uuid::from_u128).collect();
            assert!(require_complete_tag_membership(&tags, count as i64).is_ok());
            for wrong_count in [-1, count as i64 - 1, count as i64 + 1] {
                match require_complete_tag_membership(&tags, wrong_count).unwrap_err() {
                    AppError::Validation(message) => {
                        assert_eq!(message, "tag_ids must belong to the selected vehicle")
                    }
                    other => panic!("unexpected error: {other:?}"),
                }
            }
        }
    }

    #[tokio::test]
    async fn absent_tag_ids_do_not_consult_the_database() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://synthetic:synthetic@127.0.0.1:1/analytics_contract")
            .unwrap();
        pool.close().await;
        for untagged in [false, true] {
            let filter = parse_tag_filter(None, None, Some(untagged)).unwrap();
            require_known_vehicle_tags(&pool, Uuid::nil(), &filter)
                .await
                .unwrap();
        }
    }

    #[test]
    fn serialized_match_modes_keep_the_wire_contract() {
        assert!(matches!(
            serde_json::from_str::<TripTagMatch>("\"all\"").unwrap(),
            TripTagMatch::All
        ));
        assert!(matches!(
            serde_json::from_str::<TripTagMatch>("\"any\"").unwrap(),
            TripTagMatch::Any
        ));
        for raw in ["\"ALL\"", "\"Any\"", "\"none\"", "null", "true"] {
            assert!(serde_json::from_str::<TripTagMatch>(raw).is_err());
        }
    }

    #[test]
    fn predicate_keeps_null_all_any_and_untagged_contract_at_each_callsite() {
        for (alias, tag, all, untagged) in [
            ("t", 4, 5, 6),
            ("t", 8, 9, 10),
            ("t", 6, 7, 8),
            ("t", 5, 6, 7),
            ("cohort_trip", 14, 25, 36),
        ] {
            let expected = format!(
                "AND (${tag}::uuid[] IS NULL OR CASE WHEN ${all} THEN \
                 (SELECT COUNT(DISTINCT tta.tag_id) FROM panda.trip_tag_assignments tta \
                 WHERE tta.trip_id={alias}.id AND tta.tag_id=ANY(${tag})) = cardinality(${tag}) \
                 ELSE EXISTS (SELECT 1 FROM panda.trip_tag_assignments tta \
                 WHERE tta.trip_id={alias}.id AND tta.tag_id=ANY(${tag})) END) \
                 AND (NOT ${untagged} OR NOT EXISTS (SELECT 1 FROM panda.trip_tag_assignments tta \
                 WHERE tta.trip_id={alias}.id))"
            );
            assert_eq!(
                sql_predicate(alias, tag, all, untagged)
                    .split_whitespace()
                    .collect::<Vec<_>>(),
                expected.split_whitespace().collect::<Vec<_>>()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_defaults_to_all_and_rejects_untagged_combination() {
        let tag = Uuid::new_v4();
        let filter = parse_tag_filter(Some(&tag.to_string()), None, None).unwrap();
        assert!(filter.match_all);
        assert_eq!(filter.tag_ids, Some(vec![tag]));
        assert!(parse_tag_filter(Some(&tag.to_string()), None, Some(true)).is_err());
    }

    #[test]
    fn predicate_uses_the_requested_bind_positions_and_trip_alias() {
        let predicate = sql_predicate("t", 4, 5, 6);
        assert!(predicate.contains("$4::uuid[]"));
        assert!(predicate.contains("CASE WHEN $5"));
        assert!(predicate.contains("NOT $6"));
        assert!(predicate.contains("tta.trip_id=t.id"));
    }
}
