//! Manages one Tokio task per vehicle.

use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::{config::Config, ingestion::worker::run_vehicle_worker};

type WorkerEntry = (JoinHandle<()>, broadcast::Sender<()>);

enum WorkerPhase {
    Waiting,
    Running(WorkerEntry),
}

struct WorkerSlot {
    restart: RestartState,
    phase: WorkerPhase,
}

impl WorkerSlot {
    fn waiting() -> Self {
        Self {
            restart: RestartState::new(),
            phase: WorkerPhase::Waiting,
        }
    }

    async fn reconcile<F>(&mut self, vehicle_id: Uuid, spawn: &F)
    where
        F: Fn(Uuid, broadcast::Receiver<()>) -> JoinHandle<()>,
    {
        match &self.phase {
            WorkerPhase::Running((task, _)) if !task.is_finished() => return,
            WorkerPhase::Running(_) => {
                let WorkerPhase::Running((task, shutdown)) =
                    std::mem::replace(&mut self.phase, WorkerPhase::Waiting)
                else {
                    unreachable!()
                };
                let _ = shutdown.send(());
                match task.await {
                    Ok(()) => tracing::warn!(%vehicle_id, "worker exited; scheduling recovery"),
                    Err(error) => {
                        tracing::error!(%vehicle_id, %error, "worker crashed; scheduling recovery")
                    }
                }
                self.restart.failed();
            }
            WorkerPhase::Waiting => {}
        }
        if tokio::time::Instant::now() >= self.restart.restart_at {
            let (shutdown, receiver) = broadcast::channel(1);
            self.phase = WorkerPhase::Running((spawn(vehicle_id, receiver), shutdown));
            self.restart.started_at = tokio::time::Instant::now();
            tracing::info!(%vehicle_id, failures=self.restart.failures, "worker started");
        }
    }

    async fn stop(self, vehicle_id: Uuid) {
        if let WorkerPhase::Running(worker) = self.phase {
            stop_task(vehicle_id, worker).await;
        }
    }
}

enum SupervisorState {
    Serving,
    Draining,
}

struct RestartState {
    failures: u32,
    started_at: tokio::time::Instant,
    restart_at: tokio::time::Instant,
}

impl RestartState {
    fn new() -> Self {
        Self {
            failures: 0,
            started_at: tokio::time::Instant::now(),
            restart_at: tokio::time::Instant::now(),
        }
    }

    fn failed(&mut self) {
        if self.started_at.elapsed() >= Duration::from_secs(300) {
            self.failures = 0;
        }
        let delay = Duration::from_secs((5_u64 * (1_u64 << self.failures.min(6))).min(300));
        self.failures = self.failures.saturating_add(1);
        self.restart_at = tokio::time::Instant::now() + delay;
    }
}

async fn stop_task(vehicle_id: Uuid, (mut handle, shutdown): WorkerEntry) {
    let _ = shutdown.send(());
    if tokio::time::timeout(Duration::from_secs(5), &mut handle)
        .await
        .is_err()
    {
        handle.abort();
        let _ = handle.await;
        tracing::warn!(%vehicle_id, "worker stop timed out; aborted");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tokio::time::timeout;

    /// Build a minimal `Config` suitable for unit tests (no real DB connections made).
    fn test_config() -> Config {
        Config {
            database_url: "postgres://invalid/invalid".into(),
            redis_url: "redis://127.0.0.1/".into(),
            jwt_secret: None,
            jwt_public_key: None,
            age_encryption_key: None,
            port: 3001,
            allowed_origins: vec![],
            s3_endpoint: None,
            s3_access_key: None,
            s3_secret_key: None,
            backup_artifact_dir: std::env::temp_dir()
                .join("panda-backups-test")
                .to_string_lossy()
                .into_owned(),
            vehicle_image_cache_dir: std::env::temp_dir()
                .join("panda-vehicle-images-test")
                .to_string_lossy()
                .into_owned(),
            backup_driver: "pg_dump".into(),
            backup_poll_interval_seconds: 60,
            restore_agent_url: "http://127.0.0.1:3002".into(),
            restore_agent_key_file: "/backups/.restore-agent-key".into(),
            recovery: crate::config::RecoveryConfig::default(),
            origin_bind: crate::config::OriginBindConfig::default(),
            security: Default::default(),
            rivian_ws_reconnect_initial_seconds: 10,
            rivian_ws_reconnect_max_seconds: 900,
            rivian_raw_event_retention_days: 7,
            rivian_persist_raw_events: false,
            rivian_suppress_duplicate_telemetry: true,
            panda_env: None,
            cookie_insecure: Some(true),
            allow_insecure_lan_http_auth: false,
            rate_limit: crate::config::RateLimitConfig::default(),
        }
    }

    /// Build a `WorkerSupervisor` whose `run()` loop can be driven by the caller
    /// via the returned `SupervisorHandle`. Workers in `extra_workers` are
    /// pre-injected so tests can verify `StopWorker` / `Shutdown` behaviour
    /// without touching the real worker factory.
    fn make_supervisor_with_workers(
        extra_workers: HashMap<Uuid, (JoinHandle<()>, broadcast::Sender<()>)>,
    ) -> (SupervisorHandle, WorkerSupervisor) {
        use sqlx::postgres::PgPoolOptions;
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://invalid/invalid")
            .expect("lazy pool");
        let redis = redis::Client::open("redis://127.0.0.1/").expect("redis client");
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let sup = WorkerSupervisor {
            pool,
            redis,
            age_key: "test-key".into(),
            config: test_config(),
            workers: extra_workers
                .into_iter()
                .map(|(id, worker)| {
                    (
                        id,
                        WorkerSlot {
                            restart: RestartState::new(),
                            phase: WorkerPhase::Running(worker),
                        },
                    )
                })
                .collect(),
            cmd_rx,
        };
        (SupervisorHandle { tx: cmd_tx }, sup)
    }

    // ── noop handle ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn noop_handle_accepts_all_commands_without_panicking() {
        let handle = SupervisorHandle::noop();
        let id = Uuid::new_v4();
        handle
            .send(SupervisorCommand::StartWorker { vehicle_id: id })
            .await;
        handle
            .send(SupervisorCommand::StopWorker { vehicle_id: id })
            .await;
        handle.send(SupervisorCommand::Shutdown).await;
    }

    #[tokio::test]
    async fn send_reports_when_supervisor_channel_is_closed() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let handle = SupervisorHandle { tx };

        assert!(
            !handle
                .send(SupervisorCommand::StartWorker {
                    vehicle_id: Uuid::new_v4(),
                })
                .await
        );
    }

    // ── StopWorker sends shutdown signal ─────────────────────────────────────

    #[tokio::test]
    async fn stop_worker_signals_shutdown_broadcast() {
        let vehicle_id = Uuid::new_v4();

        // A worker that parks until it receives the broadcast shutdown signal.
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);
        let received_shutdown = Arc::new(AtomicBool::new(false));
        let received_clone = Arc::clone(&received_shutdown);
        let handle = tokio::spawn(async move {
            let _ = shutdown_rx.recv().await;
            received_clone.store(true, Ordering::SeqCst);
        });

        let mut workers = HashMap::new();
        workers.insert(vehicle_id, (handle, shutdown_tx));

        let (handle, mut sup) = make_supervisor_with_workers(workers);

        // Drive the supervisor loop in a background task.
        tokio::spawn(async move { sup.run().await });

        handle
            .send(SupervisorCommand::StopWorker { vehicle_id })
            .await;

        // Give the worker time to process the broadcast and set the flag.
        timeout(Duration::from_secs(2), async {
            while !received_shutdown.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("worker should have received the shutdown broadcast within 2s");
    }

    // ── StartWorker is idempotent ─────────────────────────────────────────────

    #[tokio::test]
    async fn start_worker_twice_does_not_spawn_duplicate() {
        // We can't easily inspect the workers map from outside, so we verify
        // idempotency indirectly: a second StartWorker for the same vehicle_id
        // must NOT send a second shutdown broadcast when we subsequently stop it.
        let vehicle_id = Uuid::new_v4();

        let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);
        let signal_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let signal_clone = Arc::clone(&signal_count);
        let handle = tokio::spawn(async move {
            // Count how many shutdown signals arrive.
            while shutdown_rx.recv().await.is_ok() {
                signal_clone.fetch_add(1, Ordering::SeqCst);
            }
        });

        let mut workers = HashMap::new();
        workers.insert(vehicle_id, (handle, shutdown_tx));

        let (sup_handle, mut sup) = make_supervisor_with_workers(workers);
        tokio::spawn(async move { sup.run().await });

        // This second StartWorker should be ignored (vehicle already registered).
        sup_handle
            .send(SupervisorCommand::StartWorker { vehicle_id })
            .await;
        sup_handle
            .send(SupervisorCommand::StopWorker { vehicle_id })
            .await;

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Only one shutdown signal should have arrived (from the one real worker).
        assert_eq!(
            signal_count.load(Ordering::SeqCst),
            1,
            "duplicate StartWorker must not result in extra shutdown signals"
        );
    }

    // ── Shutdown drains all workers ───────────────────────────────────────────

    #[tokio::test]
    async fn shutdown_signals_all_workers() {
        let ids: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
        let mut workers = HashMap::new();

        let flags: Vec<Arc<AtomicBool>> = ids
            .iter()
            .map(|_| Arc::new(AtomicBool::new(false)))
            .collect();

        for (id, flag) in ids.iter().zip(flags.iter()) {
            let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);
            let flag_clone = Arc::clone(flag);
            let handle = tokio::spawn(async move {
                let _ = shutdown_rx.recv().await;
                flag_clone.store(true, Ordering::SeqCst);
            });
            workers.insert(*id, (handle, shutdown_tx));
        }

        let (sup_handle, mut sup) = make_supervisor_with_workers(workers);
        tokio::spawn(async move { sup.run().await });

        sup_handle.send(SupervisorCommand::Shutdown).await;

        timeout(Duration::from_secs(2), async {
            loop {
                if flags.iter().all(|f| f.load(Ordering::SeqCst)) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all workers should receive shutdown within 2s");
    }

    #[tokio::test(start_paused = true)]
    async fn crashed_worker_restarts_with_bounded_backoff() {
        use std::sync::atomic::AtomicUsize;
        let vehicle_id = Uuid::new_v4();
        let (_handle, mut sup) = make_supervisor_with_workers(HashMap::new());
        sup.workers.insert(vehicle_id, WorkerSlot::waiting());
        let starts = Arc::new(AtomicUsize::new(0));
        let count = starts.clone();
        let factory = move |_, _| {
            count.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async { panic!("synthetic worker crash") })
        };
        sup.reconcile(&factory).await;
        tokio::task::yield_now().await;
        sup.reconcile(&factory).await;
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_secs(4)).await;
        sup.reconcile(&factory).await;
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_secs(1)).await;
        sup.reconcile(&factory).await;
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        tokio::task::yield_now().await;
        sup.reconcile(&factory).await;
        assert_eq!(
            sup.workers[&vehicle_id].restart.restart_at - tokio::time::Instant::now(),
            Duration::from_secs(10)
        );
        let state = &mut sup.workers.get_mut(&vehicle_id).unwrap().restart;
        for _ in 0..100 {
            state.failed();
        }
        assert_eq!(
            state.restart_at - tokio::time::Instant::now(),
            Duration::from_secs(300)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn deliberate_stop_cancels_pending_restart() {
        let vehicle_id = Uuid::new_v4();
        let (handle, mut sup) = make_supervisor_with_workers(HashMap::new());
        let mut state = RestartState::new();
        state.failed();
        sup.workers.insert(
            vehicle_id,
            WorkerSlot {
                restart: state,
                phase: WorkerPhase::Waiting,
            },
        );
        handle
            .send(SupervisorCommand::StopWorker { vehicle_id })
            .await;
        handle.send(SupervisorCommand::Shutdown).await;
        sup.run_with(|_, _| panic!("stopped worker must not spawn"))
            .await;
        assert!(sup.workers.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn closed_command_channel_stops_workers_and_waits_for_abort() {
        let vehicle_id = Uuid::new_v4();
        let (shutdown, _rx) = broadcast::channel(1);
        let task = tokio::spawn(std::future::pending::<()>());
        let abort = task.abort_handle();
        let (handle, mut sup) =
            make_supervisor_with_workers(HashMap::from([(vehicle_id, (task, shutdown))]));
        drop(handle);
        sup.run_with(|_, _| panic!("shutdown must not spawn")).await;
        assert!(abort.is_finished());
    }
}

#[derive(Debug)]
pub enum SupervisorCommand {
    StartWorker { vehicle_id: Uuid },
    StopWorker { vehicle_id: Uuid },
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct SupervisorHandle {
    tx: mpsc::Sender<SupervisorCommand>,
}

impl SupervisorHandle {
    pub async fn send(&self, cmd: SupervisorCommand) -> bool {
        self.tx.send(cmd).await.is_ok()
    }

    /// Creates a handle whose commands are silently dropped — for use in tests.
    pub fn noop() -> Self {
        let (tx, _rx) = mpsc::channel(1);
        Self { tx }
    }
}

pub struct WorkerSupervisor {
    pool: sqlx::PgPool,
    redis: redis::Client,
    age_key: String,
    config: Config,
    workers: HashMap<Uuid, WorkerSlot>,
    cmd_rx: mpsc::Receiver<SupervisorCommand>,
}

impl WorkerSupervisor {
    pub fn start(
        pool: sqlx::PgPool,
        redis: redis::Client,
        age_key: String,
        config: Config,
    ) -> SupervisorHandle {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);

        let mut sup = WorkerSupervisor {
            pool,
            redis,
            age_key,
            config,
            workers: HashMap::new(),
            cmd_rx,
        };

        tokio::spawn(async move { sup.run().await });

        SupervisorHandle { tx: cmd_tx }
    }

    async fn run(&mut self) {
        let (pool, redis, age_key, config) = (
            self.pool.clone(),
            self.redis.clone(),
            self.age_key.clone(),
            self.config.clone(),
        );
        self.run_with(move |vehicle_id, shutdown| {
            tokio::spawn(run_vehicle_worker(
                vehicle_id,
                pool.clone(),
                redis.clone(),
                age_key.clone(),
                config.clone(),
                shutdown,
            ))
        })
        .await;
    }

    async fn reconcile<F>(&mut self, spawn: &F)
    where
        F: Fn(Uuid, broadcast::Receiver<()>) -> JoinHandle<()>,
    {
        for (&vehicle_id, slot) in &mut self.workers {
            slot.reconcile(vehicle_id, spawn).await;
        }
    }

    async fn run_with<F>(&mut self, spawn: F)
    where
        F: Fn(Uuid, broadcast::Receiver<()>) -> JoinHandle<()>,
    {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut state = SupervisorState::Serving;
        loop {
            match state {
                SupervisorState::Serving => {
                    let command = tokio::select! {
                        biased;
                        command = self.cmd_rx.recv() => command,
                        _ = interval.tick() => {
                            self.reconcile(&spawn).await;
                            continue;
                        }
                    };
                    match command {
                        Some(SupervisorCommand::StartWorker { vehicle_id }) => {
                            self.workers
                                .entry(vehicle_id)
                                .or_insert_with(WorkerSlot::waiting);
                            self.reconcile(&spawn).await;
                        }
                        Some(SupervisorCommand::StopWorker { vehicle_id }) => {
                            if let Some(slot) = self.workers.remove(&vehicle_id) {
                                slot.stop(vehicle_id).await;
                            }
                        }
                        Some(SupervisorCommand::Shutdown) | None => {
                            state = SupervisorState::Draining
                        }
                    }
                }
                SupervisorState::Draining => {
                    let mut stopping = tokio::task::JoinSet::new();
                    for (vehicle_id, slot) in self.workers.drain() {
                        stopping.spawn(slot.stop(vehicle_id));
                    }
                    while stopping.join_next().await.is_some() {}
                    return;
                }
            }
        }
    }
}
