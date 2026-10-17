pub mod charge_detector;
pub mod closure_motion;
pub mod location;
pub mod parser;
pub mod poller;
pub mod rivian_auth;
pub mod rivian_poll;
pub mod session_store;
pub mod supervisor;
pub mod trip_detector;
pub mod trip_signals;
pub mod worker;
pub mod ws_client;

use crate::config::Config;
use sqlx::PgPool;

pub async fn start_workers(
    pool: PgPool,
    redis: redis::Client,
    age_key: String,
    config: Config,
) -> anyhow::Result<supervisor::SupervisorHandle> {
    use futures::{StreamExt, TryStreamExt};
    let handle =
        supervisor::WorkerSupervisor::start(pool.clone(), redis, age_key.clone(), config.clone());
    let enrolled: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT v.id FROM panda.vehicles v WHERE v.rivian_vehicle_id NOT LIKE 'demo-%'",
    )
    .fetch_all(&pool)
    .await?;
    futures::stream::iter(enrolled)
        .then(|vehicle_id| {
            let handle = &handle;
            async move {
                match handle
                    .send(supervisor::SupervisorCommand::StartWorker { vehicle_id })
                    .await
                {
                    true => Ok(()),
                    false => Err(anyhow::anyhow!(
                        "worker supervisor stopped while starting vehicle {vehicle_id}"
                    )),
                }
            }
        })
        .try_collect::<()>()
        .await?;
    crate::routes::vehicles::start_vehicle_artwork_repair_worker(pool, config, age_key);
    Ok(handle)
}

/// Values stay typed through binding; column names come only from source code.
#[derive(Clone, Copy)]
pub(crate) enum AcquisitionValue<'a> {
    Id(uuid::Uuid),
    Text(Option<&'a str>),
    Float(Option<f64>),
    Int(Option<i32>),
    Count(i64),
    Date(chrono::NaiveDate),
    Flag(Option<bool>),
    Json(Option<&'a serde_json::Value>),
    Time(Option<chrono::DateTime<chrono::Utc>>),
    Texts(Option<&'a [String]>),
    Bytes(&'a [u8]),
    Sql(&'static str),
}

pub(crate) enum AcquisitionMerge {
    Replace,
    Present,
    Keep,
    Sum,
    Greatest,
    Expression(&'static str),
}

pub(crate) struct AcquisitionWrite<'a> {
    table: &'static str,
    fields: Vec<(&'static str, AcquisitionValue<'a>, AcquisitionMerge)>,
}

impl<'a> AcquisitionWrite<'a> {
    pub(crate) fn new(table: &'static str) -> Self {
        Self {
            table,
            fields: Vec::new(),
        }
    }

    pub(crate) fn field(
        mut self,
        name: &'static str,
        value: AcquisitionValue<'a>,
        merge: AcquisitionMerge,
    ) -> Self {
        self.fields.push((name, value, merge));
        self
    }

    fn bind(query: &mut sqlx::QueryBuilder<sqlx::Postgres>, value: AcquisitionValue<'a>) {
        match value {
            AcquisitionValue::Id(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Text(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Float(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Int(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Count(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Date(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Flag(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Json(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Time(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Texts(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Bytes(value) => {
                query.push_bind(value);
            }
            AcquisitionValue::Sql(value) => {
                query.push(value);
            }
        }
    }

    fn insertion(&self) -> sqlx::QueryBuilder<sqlx::Postgres> {
        let columns = self
            .fields
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<Vec<_>>()
            .join(", ");
        let mut query =
            sqlx::QueryBuilder::new(format!("INSERT INTO {} ({columns}) VALUES (", self.table));
        for (index, (_, value, _)) in self.fields.iter().enumerate() {
            if index != 0 {
                query.push(", ");
            }
            Self::bind(&mut query, *value);
        }
        query.push(")");
        query
    }

    pub(crate) async fn insert<'e>(
        self,
        pool: impl sqlx::Executor<'e, Database = sqlx::Postgres>,
    ) -> Result<u64, sqlx::Error> {
        Ok(self
            .insertion()
            .build()
            .execute(pool)
            .await?
            .rows_affected())
    }

    pub(crate) async fn upsert<'e>(
        self,
        pool: impl sqlx::Executor<'e, Database = sqlx::Postgres>,
        conflict: &'static str,
        condition: &'static str,
    ) -> Result<u64, sqlx::Error> {
        let updates = self
            .fields
            .iter()
            .filter_map(|(name, _, merge)| {
                let value = match merge {
                    AcquisitionMerge::Keep => return None,
                    AcquisitionMerge::Replace => format!("EXCLUDED.{name}"),
                    AcquisitionMerge::Present => {
                        format!("COALESCE(EXCLUDED.{name}, {}.{name})", self.table)
                    }
                    AcquisitionMerge::Sum => format!("{}.{name} + EXCLUDED.{name}", self.table),
                    AcquisitionMerge::Greatest => {
                        format!("GREATEST({}.{name}, EXCLUDED.{name})", self.table)
                    }
                    AcquisitionMerge::Expression(expression) => (*expression).to_owned(),
                };
                Some(format!("{name} = {value}"))
            })
            .collect::<Vec<_>>();
        let mut query = self.insertion();
        query.push(" ON CONFLICT ").push(conflict);
        if updates.is_empty() {
            query.push(" DO NOTHING");
        } else {
            query.push(" DO UPDATE SET ").push(updates.join(", "));
            if !condition.is_empty() {
                query.push(" WHERE ").push(condition);
            }
        }
        Ok(query.build().execute(pool).await?.rows_affected())
    }

    pub(crate) async fn update<'e>(
        self,
        pool: impl sqlx::Executor<'e, Database = sqlx::Postgres>,
        keys: &[(&'static str, uuid::Uuid)],
        condition: &'static str,
    ) -> Result<u64, sqlx::Error> {
        let mut query = sqlx::QueryBuilder::new(format!("UPDATE {} SET ", self.table));
        for (index, (name, value, merge)) in self.fields.into_iter().enumerate() {
            if index != 0 {
                query.push(", ");
            }
            query.push(name).push(" = ");
            match merge {
                AcquisitionMerge::Expression(expression) => {
                    query.push(expression);
                }
                AcquisitionMerge::Greatest => {
                    query.push("GREATEST(COALESCE(").push(name).push(", 0), ");
                    Self::bind(&mut query, value);
                    query.push(")");
                }
                AcquisitionMerge::Present => {
                    query.push("COALESCE(");
                    Self::bind(&mut query, value);
                    query.push(", ").push(name).push(")");
                }
                _ => Self::bind(&mut query, value),
            }
        }
        query.push(" WHERE ");
        for (index, (name, value)) in keys.iter().enumerate() {
            if index != 0 {
                query.push(" AND ");
            }
            query.push(*name).push(" = ").push_bind(*value);
        }
        if !condition.is_empty() {
            query.push(" AND (").push(condition).push(")");
        }
        Ok(query.build().execute(pool).await?.rows_affected())
    }
}
