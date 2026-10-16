use super::*;

pub(super) struct SearchPlan {
    normalized: String,
    supplied: String,
    limit: u8,
    persistent_key: String,
}

enum SearchEvidence {
    Persistent(Value),
    Memory(Value),
    Provider(Vec<Value>),
}

impl SearchEvidence {
    fn project(self) -> Vec<AddressRecord> {
        let (hit, persistent, rows) = match self {
            Self::Persistent(value) => (true, true, value.as_array().cloned().unwrap_or_default()),
            Self::Memory(value) => (true, false, value.as_array().cloned().unwrap_or_default()),
            Self::Provider(rows) => (false, false, rows),
        };
        let suggestions: Vec<_> = rows
            .into_iter()
            .filter_map(value_to_address_record)
            .collect();
        if persistent {
            debug!(
                cache_hit = hit,
                cache = "persistent",
                suggestion_count = suggestions.len(),
                "places.address_search_completed"
            );
        } else {
            debug!(
                cache_hit = hit,
                suggestion_count = suggestions.len(),
                "places.address_search_completed"
            );
        }
        suggestions
    }
}

impl SearchPlan {
    pub(super) fn new(params: SearchParams) -> Option<Self> {
        let supplied = params.q.trim().to_owned();
        let normalized = supplied.to_lowercase();
        if normalized.len() < 3 {
            return None;
        }
        let limit = params.limit.unwrap_or(5);
        let persistent_key = search_cache_key(&normalized, limit);
        Some(Self {
            normalized,
            supplied,
            limit,
            persistent_key,
        })
    }
    pub(super) async fn resolve(self, state: &AppState) -> Result<Vec<AddressRecord>, AppError> {
        let evidence = match self.cached(state).await {
            Some(evidence) => evidence,
            None => {
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(8))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|error| {
                        AppError::Internal(anyhow!("address search client build failed: {error}"))
                    })?;
                let rows = crate::services::nominatim::search(
                    &state.pool,
                    &client,
                    &self.supplied,
                    self.limit,
                )
                .await?;
                self.publish(state, &rows).await;
                SearchEvidence::Provider(rows)
            }
        };
        Ok(evidence.project())
    }
    async fn cached(&self, state: &AppState) -> Option<SearchEvidence> {
        if let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await {
            if let Ok(Some(payload)) = redis.get::<_, Option<String>>(&self.persistent_key).await {
                if let Ok(value) = serde_json::from_str(&payload) {
                    return Some(SearchEvidence::Persistent(value));
                }
            }
        }
        let cache = state.nominatim_cache.read().await;
        cache
            .get(&self.normalized)
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(3600))
            .map(|(_, value)| SearchEvidence::Memory(value.clone()))
    }
    async fn publish(&self, state: &AppState, rows: &[Value]) {
        {
            let mut cache = state.nominatim_cache.write().await;
            cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(3600));
            if cache.len() >= 500 {
                if let Some(oldest) = cache
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(key, _)| key.clone())
                {
                    cache.remove(&oldest);
                }
            }
            cache.insert(
                self.normalized.clone(),
                (std::time::Instant::now(), Value::Array(rows.to_vec())),
            );
        }
        if let Ok(payload) = serde_json::to_string(rows) {
            if let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await {
                let _: Result<(), _> = redis.set(&self.persistent_key, payload).await;
            }
        }
    }
}
