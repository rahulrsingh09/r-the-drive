use super::*;
use sqlx::Row;

struct Sampling {
    source: usize,
    emitted: usize,
    count: usize,
    maximum: Option<usize>,
}

impl Sampling {
    fn take(&mut self) -> bool {
        let selected = match self.maximum {
            None => true,
            Some(maximum) if self.count <= maximum => true,
            Some(maximum) => self.source == self.emitted * (self.count - 1) / (maximum - 1),
        };
        self.source += 1;
        if selected {
            self.emitted += 1;
        }
        selected
    }
}

struct Cursor {
    projection: planning::Projection,
    sampling: Sampling,
}

impl Cursor {
    async fn open(
        conn: &mut sqlx::PgConnection,
        request: &StreamRequest,
        query: &SeriesQuery,
    ) -> Result<Self, AppError> {
        let count = match request.max_points {
            Some(_) => {
                let sql = format!(
                    "SELECT COUNT(*)::bigint AS count FROM ({}) AS metric_count",
                    query.sql
                );
                bind_series(&sql, request, query.filtered)
                    .fetch_one(&mut *conn)
                    .await?
                    .try_get::<i64, _>("count")? as usize
            }
            None => 0,
        };
        let sql = format!("DECLARE metric_points NO SCROLL CURSOR FOR {}", query.sql);
        bind_series(&sql, request, query.filtered)
            .persistent(false)
            .execute(&mut *conn)
            .await?;
        Ok(Self {
            projection: query.projection,
            sampling: Sampling {
                source: 0,
                emitted: 0,
                count,
                maximum: request.max_points,
            },
        })
    }

    async fn drain(
        mut self,
        conn: &mut sqlx::PgConnection,
        output: &mut MetricOutput,
    ) -> Result<(), AppError> {
        loop {
            let rows = sqlx::query("FETCH FORWARD 10000 FROM metric_points")
                .fetch_all(&mut *conn)
                .await?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                if !self.sampling.take() {
                    continue;
                }
                let point = self.projection.point(&row)?;
                if self.sampling.emitted > 1 {
                    output.write(b",").await?;
                }
                output.json(&point).await?;
            }
        }
        sqlx::query("CLOSE metric_points")
            .execute(&mut *conn)
            .await?;
        Ok(())
    }
}

struct Document<'a> {
    output: &'a mut MetricOutput,
    standalone: bool,
    values: usize,
    series: usize,
}

impl<'a> Document<'a> {
    async fn begin(output: &'a mut MetricOutput, standalone: bool) -> Result<Self, AppError> {
        if !standalone {
            output.write(b"{\"values\":[").await?;
        }
        Ok(Self {
            output,
            standalone,
            values: 0,
            series: 0,
        })
    }

    async fn value(
        &mut self,
        metric: &MetricDef,
        value: Option<f64>,
        ts: Option<DateTime<Utc>>,
    ) -> Result<(), AppError> {
        if self.values > 0 {
            self.output.write(b",").await?;
        }
        self.output
            .json(&MetricValueResponse {
                metric: metric.id.into(),
                value,
                unit: metric.unit,
                label: metric.label,
                ts,
            })
            .await?;
        self.values += 1;
        Ok(())
    }

    async fn series_phase(&mut self) -> Result<(), AppError> {
        if !self.standalone {
            self.output.write(b"],\"series\":[").await?;
        }
        Ok(())
    }

    async fn open_series(&mut self, metric: &MetricDef) -> Result<(), AppError> {
        if self.series > 0 {
            self.output.write(b",").await?;
        }
        if !self.standalone {
            self.output.write(b"{\"metric\":").await?;
            self.output.json(&metric.id).await?;
            self.output.write(b",\"points\":").await?;
        }
        self.output.write(b"[").await?;
        self.series += 1;
        Ok(())
    }

    async fn close_series(&mut self) -> Result<(), AppError> {
        if !self.standalone {
            self.output.write(b"]}").await?;
        }
        Ok(())
    }

    async fn finish(self, request: &StreamRequest) -> Result<(), AppError> {
        if self.standalone {
            self.output.write(b"]").await?;
        } else {
            self.output.write(b"],\"bucket\":").await?;
            self.output.json(&request.bucket).await?;
            self.output.write(b",\"density\":").await?;
            self.output.json(&request.density).await?;
            if let Some(maximum) = request.max_points {
                self.output.write(b",\"max_points\":").await?;
                self.output.json(&maximum).await?;
            }
            self.output.write(b"}").await?;
        }
        self.output.flush().await
    }
}

pub(super) async fn produce(
    state: &AppState,
    request: &StreamRequest,
    output: &mut MetricOutput,
    permit: crate::services::resource_limits::ResourcePermit,
) -> Result<(), AppError> {
    let mut tx = crate::services::admitted_read::AdmittedRead::begin(
        &state.pool,
        permit,
        state.config.security.metrics_timeout_seconds,
    )
    .await?;
    let mut document = Document::begin(output, request.standalone).await?;
    if !request.standalone {
        for (metric, latest, _, _) in &request.metrics {
            if !latest {
                continue;
            }
            let (value, ts) = metric_value(
                &mut tx,
                request.vehicle_id,
                metric,
                request.from,
                request.to,
                &request.tag_filter,
            )
            .await?;
            document.value(metric, value, ts).await?;
        }
    }
    document.series_phase().await?;
    for (metric, _, include_series, query) in &request.metrics {
        if !include_series {
            continue;
        }
        let query = query.as_ref().expect("series validated before response");
        document.open_series(metric).await?;
        Cursor::open(&mut tx, request, query)
            .await?
            .drain(&mut tx, document.output)
            .await?;
        document.close_series().await?;
    }
    // Only a committed read may publish the closing JSON delimiter.
    tx.commit().await?;
    document.finish(request).await
}
