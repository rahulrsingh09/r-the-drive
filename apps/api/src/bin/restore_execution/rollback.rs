use super::*;

enum Step {
    RecordStart,
    Stop,
    Database,
    Artwork,
    Start,
    Health,
    RecordSuccess,
}

async fn record(config: &Config, job_id: Uuid, finished: bool) -> anyhow::Result<()> {
    if let Ok(mut job) = restore_jobs::read(config, job_id).await {
        if finished {
            job.rollback_state = restore_jobs::RollbackState::Succeeded;
            job.retryable = true;
        } else {
            job.phase = RestorePhase::RollingBack;
            job.message = "Activation failed; restoring the previous database and artwork".into();
            job.rollback_state = restore_jobs::RollbackState::InProgress;
        }
        job.updated_at = Utc::now();
        restore_jobs::write(config, &job).await?;
    }
    Ok(())
}

pub(super) async fn runtime(
    config: &Config,
    job_id: Uuid,
    prepared: &PreparedDatabase,
    artwork: Option<&ArtworkActivation>,
    cause: anyhow::Error,
) -> anyhow::Error {
    let result = async {
        for step in [
            Step::RecordStart,
            Step::Stop,
            Step::Database,
            Step::Artwork,
            Step::Start,
            Step::Health,
            Step::RecordSuccess,
        ] {
            match step {
                Step::RecordStart => record(config, job_id, false).await?,
                Step::Stop => {
                    fs::write(restore_marker_path(), job_id.to_string()).await?;
                    let _ = stop_api_process().await;
                }
                Step::Database => {
                    rollback_databases(
                        config,
                        &prepared.previous_database,
                        &prepared.candidate_database,
                    )
                    .await?;
                }
                Step::Artwork => {
                    if let Some(artwork) = artwork {
                        rollback_artwork(artwork).await?;
                    }
                }
                Step::Start => {
                    let _ = fs::remove_file(restore_marker_path()).await;
                }
                Step::Health => wait_for_api_health(config).await?,
                Step::RecordSuccess => record(config, job_id, true).await?,
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    match result {
        Ok(()) => anyhow::anyhow!(
            "restored candidate activation failed and was rolled back: {cause}"
        ),
        Err(error) => anyhow::anyhow!(
            "restored candidate activation failed ({cause}); automatic rollback also failed: {error}"
        ),
    }
}
