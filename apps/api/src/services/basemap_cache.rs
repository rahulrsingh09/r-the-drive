//! Basemap-only eviction. Operational Redis keys never participate in this index.
use crate::config::SecurityConfig;
#[cfg(test)]
use redis::AsyncCommands;

const INDEX: &str = "external:cache:basemap:lru";
const COSTS: &str = "external:cache:basemap:costs";
const TOTAL: &str = "external:cache:basemap:bytes";
const CLEAN_LEGACY: &str = r#"
local page=redis.call('SCAN',ARGV[1],'MATCH','external:basemap:*','COUNT',100)
for _,key in ipairs(page[2]) do
  if redis.pcall('TTL',key)==-1 then redis.pcall('UNLINK',key) end
end
return page[1]
"#;
const PUT: &str = r#"
local key,index,costs,total=unpack(KEYS)
local budget,ttl=tonumber(ARGV[4]),tonumber(ARGV[5])
local payload={key,key..':content-type',key..':max-age'}
local estimate=math.ceil(#ARGV[1]*1.5)+#ARGV[2]+#ARGV[3]+5*#key+2048
local function failed(result)
  return type(result)=='table' and result.err
end
local function bytes()
  return tonumber(redis.call('GET',total) or '0')
end
local function remove(k)
  local cost=tonumber(redis.call('HGET',costs,k) or '0')
  redis.call('UNLINK',k,k..':content-type',k..':max-age')
  redis.call('HDEL',costs,k); redis.call('ZREM',index,k)
  local remaining=math.max(0,tonumber(redis.call('GET',total) or '0')-cost)
  redis.call('SET',total,remaining)
end
if estimate > budget then return 0 end
if failed(redis.pcall('SET',total,'0','NX')) then return 0 end
if redis.call('HEXISTS',costs,key)==1 then remove(key) end
local oldest=redis.call('ZRANGE',index,0,63)
for _,resource in ipairs(oldest) do
  if redis.call('EXISTS',resource)==1 and bytes()+estimate<=budget then break end
  remove(resource)
end
if bytes()+estimate>budget then return 0 end
local written={}
for field,resource in ipairs(payload) do
  if failed(redis.pcall('SET',resource,ARGV[field],'EX',ttl)) then
    if #written>0 then redis.call('UNLINK',unpack(written)) end
    return 0
  end
  written[#written+1]=resource
end
local actual=2*#key+512
for _,resource in ipairs(payload) do
  actual=actual+redis.call('MEMORY','USAGE',resource)
end
if actual>estimate or bytes()+actual>budget then
  redis.call('UNLINK',unpack(payload))
  return 0
end
if failed(redis.pcall('HSET',costs,key,actual)) then
  redis.call('UNLINK',unpack(payload))
  return 0
end
if failed(redis.pcall('ZADD',index,ARGV[6],key)) then
  redis.call('HDEL',costs,key)
  redis.call('UNLINK',unpack(payload))
  return 0
end
redis.call('INCRBY',total,actual)
return 1
"#;

pub async fn store(
    client: &redis::Client,
    key: &str,
    bytes: &[u8],
    content_type: &str,
    max_age: u64,
    config: &SecurityConfig,
) {
    if !key.starts_with("external:basemap:") {
        return;
    }
    let result = async {
        let mut connection = client.get_multiplexed_async_connection().await?;
        redis::Script::new(PUT)
            .key(key)
            .key(INDEX)
            .key(COSTS)
            .key(TOTAL)
            .arg(bytes)
            .arg(content_type)
            .arg(max_age)
            .arg(config.basemap_cache_max_bytes)
            .arg(config.basemap_cache_ttl_seconds)
            .arg(chrono::Utc::now().timestamp_millis())
            .invoke_async::<i64>(&mut connection)
            .await
    }
    .await;
    if let Err(error) = result {
        tracing::debug!(?error, "basemap cache write skipped");
    }
}

pub async fn touch(connection: &mut redis::aio::MultiplexedConnection, key: &str) {
    let _: Result<i64, _> = redis::cmd("ZADD")
        .arg(INDEX)
        .arg("XX")
        .arg(chrono::Utc::now().timestamp_millis())
        .arg(key)
        .query_async(connection)
        .await;
}

/// Invalidate legacy permanent entries in bounded SCAN batches. New entries
/// always have TTLs; this makes a warm cache miss safe during the upgrade.
pub fn start_maintenance(client: redis::Client) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut cursor = 0u64;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let attempt = async {
                let mut connection = client.get_multiplexed_async_connection().await?;
                let next: u64 = redis::Script::new(CLEAN_LEGACY)
                    .arg(cursor)
                    .invoke_async(&mut connection)
                    .await?;
                Ok::<_, redis::RedisError>(next)
            }
            .await;
            if let Ok(next) = attempt {
                cursor = next;
                if cursor == 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires a dedicated PANDA_TEST_REDIS_URL"]
    async fn shared_budget_ttl_legacy_cleanup_and_operational_keys() {
        let client = redis::Client::open(std::env::var("PANDA_TEST_REDIS_URL").unwrap()).unwrap();
        let mut conn = client.get_multiplexed_async_connection().await.unwrap();
        let _: () = redis::cmd("FLUSHDB").query_async(&mut conn).await.unwrap();
        let _: () = conn.set("session:fixture", "protected").await.unwrap();
        let config = SecurityConfig {
            basemap_cache_max_bytes: 50_000,
            basemap_cache_ttl_seconds: 60,
            ..Default::default()
        };
        for index in 0..10 {
            store(
                &client,
                &format!("external:basemap:fixture:{index}"),
                &vec![0; 20_000],
                "image/png",
                60,
                &config,
            )
            .await;
        }
        let total: u64 = conn.get(TOTAL).await.unwrap();
        assert!(total > 0 && total <= config.basemap_cache_max_bytes);
        let entries: Vec<String> = conn.zrange(INDEX, 0, -1).await.unwrap();
        assert!(entries.len() <= 2 && !entries.is_empty());
        for key in entries {
            let ttl: i64 = conn.ttl(key).await.unwrap();
            assert!(ttl > 0 && ttl <= 60);
        }
        let legacy = "external:basemap:legacy:fixture";
        let _: () = conn.set(legacy, "old").await.unwrap();
        let task = start_maintenance(client.clone());
        for _ in 0..40 {
            if !conn.exists::<_, bool>(legacy).await.unwrap() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        task.abort();
        assert!(!conn.exists::<_, bool>(legacy).await.unwrap());
        assert_eq!(
            conn.get::<_, String>("session:fixture").await.unwrap(),
            "protected"
        );
        let _: () = redis::cmd("FLUSHDB").query_async(&mut conn).await.unwrap();
    }
}
