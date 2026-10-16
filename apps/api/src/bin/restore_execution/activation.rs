use super::*;

enum Resources {
    Candidate,
    Database,
    Artwork(ArtworkActivation),
}

impl Resources {
    fn artwork(&self) -> Option<&ArtworkActivation> {
        match self {
            Self::Artwork(artwork) => Some(artwork),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum RuntimeStage {
    Stop,
    Swap,
    Settings,
    Artwork,
    Start,
    Verify,
}

impl RuntimeStage {
    const ORDER: [Self; 6] = [
        Self::Stop,
        Self::Swap,
        Self::Settings,
        Self::Artwork,
        Self::Start,
        Self::Verify,
    ];

    fn progress(self) -> (RestorePhase, u8, &'static str) {
        match self {
            Self::Stop => (
                RestorePhase::StoppingApplication,
                68,
                "Candidate is ready; stopping Panda API and ingestion workers",
            ),
            Self::Swap => (
                RestorePhase::SwappingDatabase,
                74,
                "Atomically activating the validated database candidate",
            ),
            Self::Settings => (
                RestorePhase::RestoringSettings,
                70,
                "Restoring sanitized backup settings",
            ),
            Self::Artwork => (
                RestorePhase::RestoringArtwork,
                78,
                "Restoring vehicle artwork",
            ),
            Self::Start => (
                RestorePhase::StartingApplication,
                86,
                "Starting Panda with restored data",
            ),
            Self::Verify => (
                RestorePhase::VerifyingHealth,
                92,
                "Waiting for migrations and application health",
            ),
        }
    }
}

struct RuntimeActivation<'a> {
    config: &'a Config,
    job_id: Uuid,
    prepared: &'a PreparedDatabase,
    staging: &'a Path,
    resources: Resources,
}

impl RuntimeActivation<'_> {
    async fn advance(&mut self, stage: RuntimeStage) -> anyhow::Result<()> {
        let (phase, percent, message) = stage.progress();
        restore_jobs::update(self.config, self.job_id, phase, percent, message).await?;
        match stage {
            RuntimeStage::Stop => {
                fs::write(restore_marker_path(), self.job_id.to_string()).await?;
                stop_api_process().await?;
                terminate_database_sessions(self.config).await?;
            }
            RuntimeStage::Swap => {
                swap_databases(
                    self.config,
                    &self.prepared.candidate_database,
                    &self.prepared.previous_database,
                )
                .await?;
                self.resources = Resources::Database;
                inject_restore_fault("database_swapped")?;
            }
            RuntimeStage::Settings => {}
            RuntimeStage::Artwork => {
                self.resources = Resources::Artwork(
                    activate_artwork(
                        self.config,
                        &self.staging.join("vehicle-image-cache"),
                        self.job_id,
                    )
                    .await?,
                );
            }
            RuntimeStage::Start => fs::remove_file(restore_marker_path()).await?,
            RuntimeStage::Verify => {
                inject_restore_fault("artwork_activated")?;
                inject_restore_fault("health_verification")?;
                verify_api_state(self.config).await?;
            }
        }
        Ok(())
    }
}

pub(super) async fn runtime(
    config: &Config,
    job_id: Uuid,
    prepared: &PreparedDatabase,
    staging: &Path,
) -> anyhow::Result<ArtworkActivation> {
    let mut activation = RuntimeActivation {
        config,
        job_id,
        prepared,
        staging,
        resources: Resources::Candidate,
    };
    for stage in RuntimeStage::ORDER {
        if let Err(cause) = activation.advance(stage).await {
            return Err(match activation.resources {
                Resources::Candidate => cause,
                _ => {
                    rollback_activated_candidate(
                        config,
                        job_id,
                        prepared,
                        activation.resources.artwork(),
                        cause,
                    )
                    .await
                }
            });
        }
    }
    match activation.resources {
        Resources::Artwork(artwork) => Ok(artwork),
        _ => unreachable!("successful activation includes the artwork stage"),
    }
}

#[derive(Clone, Copy)]
enum HostStage {
    Swap,
    Artwork,
    Publish,
}

pub(super) async fn host(config: &Config) -> anyhow::Result<()> {
    let mut state = read_host_restore_state(config).await?;
    if state.activated {
        return Ok(());
    }
    if state.activation_started {
        anyhow::bail!("host restore activation was interrupted; run host-rollback before retrying");
    }
    state.activation_started = true;
    fs::write(
        host_restore_state_path(config),
        serde_json::to_vec_pretty(&state)?,
    )
    .await?;
    terminate_database_sessions(config).await?;

    let mut resources = Resources::Candidate;
    for stage in [HostStage::Swap, HostStage::Artwork, HostStage::Publish] {
        let result = async {
            match stage {
                HostStage::Swap => {
                    swap_databases(config, &state.candidate_database, &state.previous_database)
                        .await?;
                    resources = Resources::Database;
                }
                HostStage::Artwork => {
                    let artwork = activate_artwork(
                        config,
                        &state.staging.join("vehicle-image-cache"),
                        state.job_id,
                    )
                    .await?;
                    state.artwork_destination = Some(artwork.destination.clone());
                    state.artwork_previous = Some(artwork.previous.clone());
                    state.activated = true;
                    resources = Resources::Artwork(artwork);
                }
                HostStage::Publish => {
                    fs::write(
                        host_restore_state_path(config),
                        serde_json::to_vec_pretty(&state)?,
                    )
                    .await?;
                }
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(cause) = result {
            if !matches!(resources, Resources::Candidate) {
                rollback_databases(config, &state.previous_database, &state.candidate_database)
                    .await?;
                if let Some(artwork) = resources.artwork() {
                    rollback_artwork(artwork).await?;
                }
            }
            return Err(cause);
        }
    }
    println!("host restore candidate activated");
    Ok(())
}
