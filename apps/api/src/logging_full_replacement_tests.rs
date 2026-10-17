use super::*;
use std::{io::Write, sync::{Arc, Mutex}};

#[derive(Clone)]
struct Output(Arc<Mutex<Vec<u8>>>);

impl Write for Output {
    fn write(&mut self, value: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(value);
        Ok(value.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

#[test]
fn levels_fields_and_spans_keep_exact_log_frames() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let output = Output(bytes.clone());
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer().with_ansi(false)
            .event_format(PandaLogFormat)
            .with_writer(move || output.clone())
    );
    tracing::subscriber::with_default(subscriber, || {
        let scope = tracing::info_span!("invisible_scope", private_detail = "not emitted");
        let _guard = scope.enter();
        tracing::trace!("trace event");
        tracing::debug!("debug event");
        tracing::info!(vehicle = "synthetic", count = 3, "started");
        tracing::warn!(ready = false, "waiting");
        tracing::error!(error = "synthetic failure", "failed");
    });
    assert_eq!(String::from_utf8(bytes.lock().unwrap().clone()).unwrap(), concat!(
        "[panda][TRACE] trace event\n",
        "[panda][DEBUG] debug event\n",
        "[panda][INFO] started vehicle=\"synthetic\" count=3\n",
        "[panda][WARN] waiting ready=false\n",
        "[panda][ERROR] failed error=\"synthetic failure\"\n",
    ));
}
