use super::{sanitize_error, PgPool};

pub(super) enum Event<'a> {
    Attempt,
    Success,
    Failure(&'a str),
    Test { ok: bool, message: Option<&'a str> },
}

struct Assignment {
    column: &'static str,
    value: &'static str,
}

impl Assignment {
    fn new(column: &'static str, value: &'static str) -> Self {
        Self { column, value }
    }
}

impl Event<'_> {
    fn insertions(&self) -> Vec<Assignment> {
        let mut fields = vec![
            Assignment::new("connection_id", "$1"),
            Assignment::new("last_attempt_at", "now()"),
            Assignment::new("usage_date", "CURRENT_DATE"),
            Assignment::new(
                "request_count",
                if matches!(self, Self::Attempt) {
                    "1"
                } else {
                    "0"
                },
            ),
        ];
        fields.extend(match self {
            Self::Attempt => vec![],
            Self::Success => vec![
                Assignment::new("last_success_at", "now()"),
                Assignment::new("last_error", "NULL"),
            ],
            Self::Failure(_) => vec![Assignment::new("last_error", "$2")],
            Self::Test { .. } => vec![
                Assignment::new("last_test_at", "now()"),
                Assignment::new("last_test_ok", "$2"),
                Assignment::new("last_test_error", "$3"),
            ],
        });
        fields
    }

    fn updates(&self) -> Vec<Assignment> {
        match self {
            Self::Attempt => vec![
                Assignment::new("last_attempt_at", "now()"),
                Assignment::new("usage_date", "CURRENT_DATE"),
                Assignment::new(
                    "request_count",
                    "CASE WHEN panda.external_connection_activity.usage_date = CURRENT_DATE \
                     THEN panda.external_connection_activity.request_count + 1 ELSE 1 END",
                ),
            ],
            Self::Success => vec![
                Assignment::new("last_success_at", "now()"),
                Assignment::new("last_error", "NULL"),
            ],
            Self::Failure(_) => vec![Assignment::new("last_error", "EXCLUDED.last_error")],
            Self::Test { .. } => vec![
                Assignment::new("last_test_at", "now()"),
                Assignment::new("last_test_ok", "EXCLUDED.last_test_ok"),
                Assignment::new("last_test_error", "EXCLUDED.last_test_error"),
            ],
        }
    }

    fn statement(&self) -> String {
        let insertions = self.insertions();
        let columns = insertions
            .iter()
            .map(|field| field.column)
            .collect::<Vec<_>>()
            .join(", ");
        let values = insertions
            .iter()
            .map(|field| field.value)
            .collect::<Vec<_>>()
            .join(", ");
        let updates = self
            .updates()
            .into_iter()
            .map(|field| format!("{} = {}", field.column, field.value))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "INSERT INTO panda.external_connection_activity ({columns}) VALUES ({values}) \
             ON CONFLICT (connection_id) DO UPDATE SET {updates}"
        )
    }

    pub(super) async fn persist(self, pool: &PgPool, id: &str) {
        // Identifiers and expressions are internal constants; caller data is always bound.
        let query = sqlx::query(sqlx::AssertSqlSafe(self.statement())).bind(id);
        let _ = match self {
            Self::Attempt | Self::Success => query.execute(pool).await,
            Self::Failure(message) => query.bind(sanitize_error(message)).execute(pool).await,
            Self::Test { ok, message } => {
                query
                    .bind(ok)
                    .bind(message.map(sanitize_error))
                    .execute(pool)
                    .await
            }
        };
    }
}
