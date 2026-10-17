use std::fmt;

use tracing::{Event, Subscriber};
use tracing_subscriber::{
    fmt::{format::FormatEvent, FmtContext, FormatFields},
    registry::LookupSpan,
    EnvFilter,
};

struct PandaLogFormat;

impl<S, N> FormatEvent<S, N> for PandaLogFormat
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    N: for<'writer> tracing_subscriber::fmt::format::FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut record = format!("[panda][{}] ", event.metadata().level());
        ctx.format_fields(
            tracing_subscriber::fmt::format::Writer::new(&mut record),
            event,
        )?;
        record.push('\n');
        writer.write_str(&record)
    }
}

pub fn init() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("panda_api=debug,tower_http=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .event_format(PandaLogFormat)
        .with_writer(std::io::stdout)
        .init();
}

#[cfg(test)]
use tracing_subscriber::layer::SubscriberExt;

#[cfg(test)]
#[path = "logging_full_replacement_tests.rs"]
mod full_replacement_tests;
