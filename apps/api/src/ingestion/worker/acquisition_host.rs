use super::*;
use tokio::time::{Duration as Wait, Instant, Interval};

pub(super) enum Signal {
    Stop,
    RetryCompletions,
    ReconcileState,
    SilentSocket,
    SocketExited(Result<(), tokio::task::JoinError>),
    Inbound(Box<WsInboundEvent>),
    EmptyParallax,
}

pub(super) struct AcquisitionHost {
    socket: WorkerTask<()>,
    socket_finished: bool,
    restart: RestartSchedule,
    pub(super) last_inbound: Instant,
    retry: Interval,
    reconcile: Interval,
}
struct RestartSchedule {
    started: Instant,
    seconds: u64,
}
impl RestartSchedule {
    fn exit_delay(&mut self, now: Instant) -> Wait {
        if now.duration_since(self.started) >= Wait::from_secs(300) {
            self.seconds = 1;
        }
        Wait::from_secs(self.seconds)
    }
    fn restarted(&mut self, now: Instant, after_exit: bool) {
        if after_exit {
            self.seconds = (self.seconds * 2).min(60);
        }
        self.started = now;
    }
}
impl AcquisitionHost {
    pub(super) async fn new(socket: WorkerTask<()>, started: Instant) -> Self {
        let mut reconcile = tokio::time::interval(STATE_PERIOD_RECONCILE_INTERVAL);
        reconcile.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        reconcile.tick().await;
        let mut retry = tokio::time::interval(Wait::from_secs(15));
        retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        Self {
            socket,
            socket_finished: false,
            restart: RestartSchedule {
                started,
                seconds: 1,
            },
            last_inbound: Instant::now(),
            retry,
            reconcile,
        }
    }
    pub(super) async fn next(
        &mut self,
        shutdown: &mut broadcast::Receiver<()>,
        socket: &mut mpsc::Receiver<WsInboundEvent>,
        parallax: &mut mpsc::Receiver<(String, TelemetryEvent)>,
        parallax_enabled: bool,
    ) -> Signal {
        tokio::select! {
            _=shutdown.recv()=>Signal::Stop,
            _=self.retry.tick()=>Signal::RetryCompletions,
            _=self.reconcile.tick()=>Signal::ReconcileState,
            _=tokio::time::sleep_until(self.last_inbound+WS_WATCHDOG_TIMEOUT)=>Signal::SilentSocket,
            inbound=socket.recv()=>match inbound { Some(inbound)=>Signal::Inbound(Box::new(inbound)),None=>Signal::Stop },
            event=parallax.recv(),if parallax_enabled && !parallax.is_closed()=>match event {
                Some((topic,event))=>Signal::Inbound(Box::new(WsInboundEvent {
                    kind:WsInboundKind::Telemetry,received_at:Utc::now(),raw:String::new(),
                    message_type:Some(format!("parallax:{topic}")),telemetry:Some(event),charging_session:None,battery_cell_type:None,
                })),
                None=>Signal::EmptyParallax,
            },
            result=&mut self.socket,if !self.socket_finished=>{self.socket_finished=true;Signal::SocketExited(result)},
        }
    }
    pub(super) async fn wait_for_restart(
        &mut self,
        shutdown: &mut broadcast::Receiver<()>,
    ) -> bool {
        let delay = self.restart.exit_delay(Instant::now());
        tokio::select! { biased; _=shutdown.recv()=>false,_=tokio::time::sleep(delay)=>true }
    }
    pub(super) async fn replace(
        &mut self,
        spawn: impl FnOnce() -> WorkerTask<()>,
        after_exit: bool,
    ) {
        if !after_exit {
            self.socket.abort();
            let _ = (&mut self.socket).await;
        }
        self.socket = spawn();
        self.socket_finished = false;
        self.restart.restarted(Instant::now(), after_exit);
        self.last_inbound = Instant::now();
    }
    pub(super) async fn stop(
        &mut self,
        parallax: &mut Option<WorkerTask<()>>,
        companions: &mut tokio::task::JoinSet<()>,
    ) {
        if !self.socket_finished {
            self.socket.abort();
            let _ = (&mut self.socket).await;
            self.socket_finished = true;
        }
        if let Some(mut task) = parallax.take() {
            tokio::select! {
                _=&mut task=>{},
                _=tokio::time::sleep(Wait::from_secs(5))=>{task.abort();let _=task.await;}
            }
        }
        companions.abort_all();
        while companions.join_next().await.is_some() {}
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pending() -> WorkerTask<()> {
        WorkerTask(tokio::spawn(std::future::pending()))
    }
    #[tokio::test(start_paused = true)]
    async fn exit_backoff_cap_stable_reset_and_watchdog_restart() {
        let now = Instant::now();
        let mut schedule = RestartSchedule {
            started: now,
            seconds: 1,
        };
        for expected in [1, 2, 4, 8, 16, 32, 60, 60] {
            assert_eq!(schedule.exit_delay(Instant::now()).as_secs(), expected);
            schedule.restarted(Instant::now(), true);
        }
        tokio::time::advance(Wait::from_secs(300)).await;
        assert_eq!(schedule.exit_delay(Instant::now()).as_secs(), 1);
        schedule.restarted(Instant::now(), false);
        assert_eq!(schedule.seconds, 1);
    }
    #[tokio::test(start_paused = true)]
    async fn actual_selector_timers_sources_closure_and_cancellation() {
        let mut host = AcquisitionHost::new(pending(), Instant::now()).await;
        let (stop, mut shutdown) = broadcast::channel(1);
        let (socket, mut socket_rx) = mpsc::channel(256);
        let (parallax, mut parallax_rx) = mpsc::channel(256);
        assert!(matches!(
            host.next(&mut shutdown, &mut socket_rx, &mut parallax_rx, true)
                .await,
            Signal::RetryCompletions
        ));
        parallax
            .send((
                "charge".into(),
                TelemetryEvent::empty(Uuid::nil(), Utc::now()),
            ))
            .await
            .unwrap();
        let Signal::Inbound(inbound) = host
            .next(&mut shutdown, &mut socket_rx, &mut parallax_rx, true)
            .await
        else {
            panic!("expected frame")
        };
        assert_eq!(inbound.message_type.as_deref(), Some("parallax:charge"));
        host.last_inbound = Instant::now() - WS_WATCHDOG_TIMEOUT;
        assert!(matches!(
            host.next(&mut shutdown, &mut socket_rx, &mut parallax_rx, true)
                .await,
            Signal::SilentSocket
        ));
        host.replace(pending, false).await;
        drop(socket);
        assert!(matches!(
            host.next(&mut shutdown, &mut socket_rx, &mut parallax_rx, true)
                .await,
            Signal::Stop
        ));
        stop.send(()).unwrap();
        assert!(!host.wait_for_restart(&mut shutdown).await);
        let mut companions = tokio::task::JoinSet::new();
        companions.spawn(std::future::pending::<()>());
        let mut parallax = Some(pending());
        let start = Instant::now();
        host.stop(&mut parallax, &mut companions).await;
        assert_eq!(start.elapsed(), Wait::from_secs(5));
        assert!(parallax.is_none() && companions.is_empty());
    }
    #[tokio::test(start_paused = true)]
    async fn exited_socket_can_cancel_backoff_and_join_cleanup_without_repolling() {
        let task = WorkerTask(tokio::spawn(async {
            panic!("synthetic acquisition panic");
        }));
        let mut host = AcquisitionHost::new(task, Instant::now()).await;
        let (stop, mut shutdown) = broadcast::channel(1);
        let (_socket, mut socket_rx) = mpsc::channel(256);
        let (_parallax, mut parallax_rx) = mpsc::channel(256);
        loop {
            match host
                .next(&mut shutdown, &mut socket_rx, &mut parallax_rx, false)
                .await
            {
                Signal::SocketExited(result) => {
                    assert!(result.unwrap_err().is_panic());
                    break;
                }
                Signal::RetryCompletions => {}
                _ => panic!("unexpected selector outcome"),
            }
        }
        stop.send(()).unwrap();
        assert!(!host.wait_for_restart(&mut shutdown).await);
        let mut companions = tokio::task::JoinSet::new();
        companions.spawn(std::future::pending::<()>());
        host.stop(&mut None, &mut companions).await;
        assert!(companions.is_empty());
    }
}
