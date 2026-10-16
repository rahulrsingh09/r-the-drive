use super::AppState;
use redis::aio::MultiplexedConnection;

pub(super) enum Operation {
    Measure,
    Purge,
}

#[derive(Default)]
pub(super) struct Totals {
    pub entries: u64,
    pub bytes: u64,
}

impl Operation {
    async fn apply(
        &self,
        redis: &mut MultiplexedConnection,
        keys: Vec<String>,
        totals: &mut Totals,
    ) {
        match self {
            Self::Measure => {
                for key in keys
                    .into_iter()
                    .filter(|key| !key.ends_with(":content-type") && !key.ends_with(":max-age"))
                {
                    let length: u64 = redis::cmd("STRLEN")
                        .arg(key)
                        .query_async(redis)
                        .await
                        .unwrap_or(0);
                    totals.entries = totals.entries.saturating_add(1);
                    totals.bytes = totals.bytes.saturating_add(length);
                }
            }
            Self::Purge if !keys.is_empty() => {
                let removed: u64 = redis::cmd("UNLINK")
                    .arg(keys)
                    .query_async(redis)
                    .await
                    .unwrap_or(0);
                totals.entries = totals.entries.saturating_add(removed);
            }
            Self::Purge => {}
        }
    }
}

pub(super) async fn traverse(state: &AppState, pattern: &str, operation: Operation) -> Totals {
    let mut totals = Totals::default();
    let Ok(mut redis) = state.redis.get_multiplexed_async_connection().await else {
        return totals;
    };
    let mut pending = Some(0_u64);
    while let Some(cursor) = pending.take() {
        let page: redis::RedisResult<(u64, Vec<String>)> = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(500)
            .query_async(&mut redis)
            .await;
        match page {
            Ok((next, keys)) => {
                operation.apply(&mut redis, keys, &mut totals).await;
                pending = (next != 0).then_some(next);
            }
            Err(_) => break,
        }
    }
    totals
}
