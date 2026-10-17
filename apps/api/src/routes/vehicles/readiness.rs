use futures::{future, stream, StreamExt};
use std::{future::Future, time::Duration};

pub(super) async fn wait(pool: &sqlx::PgPool, vehicle: uuid::Uuid) -> bool {
    poll(
        || async {
            match sqlx::query_as::<_, (Option<String>, Option<String>)>(
                "SELECT worker_health, auth_state
                 FROM panda.vehicle_runtime_state WHERE vehicle_id = $1",
            )
            .bind(vehicle)
            .fetch_optional(pool)
            .await
            {
                Ok(Some((health, auth))) => {
                    super::vehicle_data_ready(health.as_deref(), auth.as_deref())
                }
                _ => false,
            }
        },
        tokio::time::sleep,
    )
    .await
}

struct PollCursor<Read, Pause> {
    read: Read,
    pause: Pause,
    remaining: u8,
    after_failure: bool,
}

async fn poll<Read, Reading, Pause, Pausing>(read: Read, pause: Pause) -> bool
where
    Read: FnMut() -> Reading,
    Reading: Future<Output = bool>,
    Pause: FnMut(Duration) -> Pausing,
    Pausing: Future<Output = ()>,
{
    let cursor = PollCursor {
        read,
        pause,
        remaining: 10,
        after_failure: false,
    };
    stream::unfold(cursor, |mut cursor| async move {
        // The final failed observation owns a delay even when no reads remain.
        if cursor.after_failure {
            (cursor.pause)(Duration::from_millis(500)).await;
        }
        cursor.remaining = cursor.remaining.checked_sub(1)?;
        let ready = (cursor.read)().await;
        cursor.after_failure = !ready;
        Some((ready, cursor))
    })
    .any(future::ready)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    #[derive(Debug, PartialEq)]
    enum Event {
        Read(usize),
        Pause(Duration),
    }

    async fn trace(ready_at: Option<usize>) {
        let events = RefCell::new(Vec::new());
        let reads = Cell::new(0);
        let result = poll(
            || {
                let n = reads.get() + 1;
                reads.set(n);
                events.borrow_mut().push(Event::Read(n));
                future::ready(ready_at == Some(n))
            },
            |duration| {
                events.borrow_mut().push(Event::Pause(duration));
                future::ready(())
            },
        )
        .await;
        assert_eq!(result, ready_at.is_some());
        let mut expected = Vec::new();
        for n in 1..=ready_at.unwrap_or(10) {
            expected.push(Event::Read(n));
            if ready_at != Some(n) {
                expected.push(Event::Pause(Duration::from_millis(500)));
            }
        }
        assert_eq!(*events.borrow(), expected);
    }

    #[tokio::test]
    async fn readiness_first_observation_has_no_initial_or_success_delay() {
        trace(Some(1)).await;
    }

    #[tokio::test]
    async fn readiness_early_and_last_success_stop_at_the_observation() {
        trace(Some(3)).await;
        trace(Some(10)).await;
    }

    #[tokio::test]
    async fn readiness_exhaustion_has_ten_reads_and_ten_half_second_delays() {
        trace(None).await;
    }

    struct Dropped(Rc<Cell<bool>>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    #[tokio::test]
    async fn readiness_cancellation_drops_the_active_read() {
        let dropped = Rc::new(Cell::new(false));
        let reads = Cell::new(0);
        let mut waiting = Box::pin(poll(
            || {
                reads.set(reads.get() + 1);
                let guard = Dropped(dropped.clone());
                async move {
                    let _guard = guard;
                    future::pending::<bool>().await
                }
            },
            |_| future::ready(()),
        ));
        assert!(futures::poll!(&mut waiting).is_pending());
        assert_eq!(reads.get(), 1);
        assert!(!dropped.get());
        drop(waiting);
        assert!(dropped.get());
        assert_eq!(reads.get(), 1);
    }

    #[tokio::test]
    async fn readiness_cancellation_drops_the_active_delay() {
        let dropped = Rc::new(Cell::new(false));
        let reads = Cell::new(0);
        let mut waiting = Box::pin(poll(
            || {
                reads.set(reads.get() + 1);
                future::ready(false)
            },
            |_| {
                let guard = Dropped(dropped.clone());
                async move {
                    let _guard = guard;
                    future::pending::<()>().await
                }
            },
        ));
        assert!(futures::poll!(&mut waiting).is_pending());
        assert_eq!(reads.get(), 1);
        assert!(!dropped.get());
        drop(waiting);
        assert!(dropped.get());
        assert_eq!(reads.get(), 1);
    }
}
