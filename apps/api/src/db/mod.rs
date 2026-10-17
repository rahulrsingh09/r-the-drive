pub mod migrations;
pub mod pool;
pub mod users;
pub mod vehicles;

/// Argument and timeline primitives shared by offline database maintenance commands.
pub mod maintenance {
    use anyhow::{bail, Context, Result};
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum OptionKind {
        Vehicle,
        VehicleId,
        Apply,
        BatchSize,
        From,
        To,
        DryRun,
        Yes,
        Backup,
    }

    impl OptionKind {
        fn spelling(self) -> &'static str {
            match self {
                Self::Vehicle => "--vehicle",
                Self::VehicleId => "--vehicle-id",
                Self::Apply => "--apply",
                Self::BatchSize => "--batch-size",
                Self::From => "--from",
                Self::To => "--to",
                Self::DryRun => "--dry-run",
                Self::Yes => "--yes",
                Self::Backup => "--backup",
            }
        }

        fn missing(self) -> &'static str {
            match self {
                Self::Vehicle => "--vehicle requires a UUID argument",
                Self::VehicleId => "--vehicle-id requires a UUID value",
                Self::BatchSize => "--batch-size requires an integer argument",
                Self::From => "--from requires an RFC3339 timestamp",
                Self::To => "--to requires an RFC3339 timestamp",
                Self::Backup => "--backup requires a path argument",
                Self::Apply | Self::DryRun | Self::Yes => unreachable!("switch has no operand"),
            }
        }
    }

    #[derive(Debug)]
    pub struct Options {
        pub vehicle_id: Option<Uuid>,
        pub apply: bool,
        pub batch_size: i64,
        pub from: Option<DateTime<Utc>>,
        pub to: Option<DateTime<Utc>>,
        pub dry_run: bool,
        pub yes: bool,
        pub backup: Option<String>,
        pub help: bool,
    }

    pub fn decode_arguments(
        arguments: impl IntoIterator<Item = String>,
        allowed: &[OptionKind],
        unknown: &str,
    ) -> Result<Options> {
        let mut options = Options {
            vehicle_id: None,
            apply: false,
            batch_size: 5_000,
            from: None,
            to: None,
            dry_run: false,
            yes: false,
            backup: None,
            help: false,
        };
        let mut operand = None;
        for argument in arguments {
            if let Some(kind) = operand.take() {
                match kind {
                    OptionKind::Vehicle => {
                        options.vehicle_id =
                            Some(argument.parse().context("invalid --vehicle UUID")?)
                    }
                    OptionKind::VehicleId => options.vehicle_id = Some(argument.parse()?),
                    OptionKind::BatchSize => {
                        let size = argument.parse().context("invalid --batch-size")?;
                        if !(1..=50_000).contains(&size) {
                            bail!("--batch-size must be between 1 and 50000");
                        }
                        options.batch_size = size;
                    }
                    OptionKind::From | OptionKind::To => {
                        let timestamp =
                            DateTime::parse_from_rfc3339(&argument)?.with_timezone(&Utc);
                        if kind == OptionKind::From {
                            options.from = Some(timestamp);
                        } else {
                            options.to = Some(timestamp);
                        }
                    }
                    OptionKind::Backup => options.backup = Some(argument),
                    _ => unreachable!("switch has no operand"),
                }
                continue;
            }
            if matches!(argument.as_str(), "--help" | "-h") {
                options.help = true;
                return Ok(options);
            }
            let Some(kind) = allowed
                .iter()
                .copied()
                .find(|kind| kind.spelling() == argument)
            else {
                bail!("{unknown}: {argument}");
            };
            match kind {
                OptionKind::Apply => options.apply = true,
                OptionKind::DryRun => options.dry_run = true,
                OptionKind::Yes => options.yes = true,
                value => operand = Some(value),
            }
        }
        if let Some(kind) = operand {
            bail!(kind.missing());
        }
        Ok(options)
    }

    pub fn command_options(allowed: &[OptionKind], usage: &str, unknown: &str) -> Result<Options> {
        let options = decode_arguments(std::env::args().skip(1), allowed, unknown)?;
        if options.help {
            println!("{usage}");
            std::process::exit(0);
        }
        Ok(options)
    }

    pub fn normalized_state(value: Option<&str>) -> String {
        let value = value.unwrap_or_default().trim();
        [
            "drive", "go", "charging", "ready", "sleep", "offline", "updating",
        ]
        .into_iter()
        .find(|known| value.eq_ignore_ascii_case(known))
        .map(|known| if known == "go" { "drive" } else { known })
        .unwrap_or("unknown")
        .to_owned()
    }

    #[derive(Debug, PartialEq)]
    pub struct StatePeriod {
        pub state: String,
        pub start: DateTime<Utc>,
        pub end: DateTime<Utc>,
    }

    /// Returns a closed interval only when a new sample breaks the current run.
    pub fn extend_period(
        current: &mut Option<StatePeriod>,
        timestamp: DateTime<Utc>,
        state: String,
    ) -> Option<StatePeriod> {
        if let Some(period) = current.as_mut() {
            if period.state == state && (timestamp - period.end).num_seconds() <= 600 {
                period.end = timestamp;
                return None;
            }
        }
        current.replace(StatePeriod {
            state,
            start: timestamp,
            end: timestamp,
        })
    }

    pub struct TripEnergyInputs {
        pub soc_start: Option<f64>,
        pub soc_end: Option<f64>,
        pub capacity: Option<f64>,
        pub range_start: Option<f64>,
        pub range_end: Option<f64>,
        pub distance: f64,
        pub efficiency: Option<f64>,
    }

    impl TripEnergyInputs {
        pub fn columns(self) -> (Option<f64>, Option<String>, Option<f64>) {
            let energy = crate::ingestion::trip_detector::compute_trip_energy(
                self.soc_start,
                self.soc_end,
                self.capacity,
                self.range_start,
                self.range_end,
                self.distance,
                self.efficiency,
            );
            let efficiency = if self.distance > 0.0 {
                energy
                    .as_ref()
                    .map(|(wh, _)| *wh)
                    .or_else(|| {
                        let ((start, end), capacity) =
                            self.soc_start.zip(self.soc_end).zip(self.capacity)?;
                        (start > end).then(|| ((start - end) / 100.0) * capacity)
                    })
                    .map(|wh| wh / self.distance)
            } else {
                None
            };
            let (wh, strategy) = energy.map_or((None, None), |(wh, strategy)| {
                (Some(wh), Some(strategy.to_string()))
            });
            (wh, strategy, efficiency)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn args(value: &[&str]) -> Vec<String> {
            value.iter().map(|s| (*s).into()).collect()
        }
        #[test]
        fn options_preserve_error_precedence_and_last_value() {
            let allowed = [
                OptionKind::Vehicle,
                OptionKind::BatchSize,
                OptionKind::Apply,
            ];
            let first = "00000000-0000-0000-0000-000000000001";
            let last = "00000000-0000-0000-0000-000000000002";
            let parsed = decode_arguments(
                args(&["--vehicle", first, "--apply", "--vehicle", last]),
                &allowed,
                "unknown argument",
            )
            .unwrap();
            assert_eq!(parsed.vehicle_id, Some(last.parse().unwrap()));
            assert!(parsed.apply);
            assert_eq!(parsed.batch_size, 5000);
            assert_eq!(
                decode_arguments(args(&["--vehicle", "--help"]), &allowed, "unknown argument")
                    .unwrap_err()
                    .to_string(),
                "invalid --vehicle UUID"
            );
            assert!(
                decode_arguments(args(&["--help", "--bad"]), &allowed, "unknown argument")
                    .unwrap()
                    .help
            );
            assert_eq!(
                decode_arguments(args(&["--batch-size"]), &allowed, "unknown argument")
                    .unwrap_err()
                    .to_string(),
                "--batch-size requires an integer argument"
            );
            assert_eq!(
                decode_arguments(args(&["--bad", "--help"]), &allowed, "unknown argument")
                    .unwrap_err()
                    .to_string(),
                "unknown argument: --bad"
            );
            for bad in ["0", "50001", "-1"] {
                assert_eq!(
                    decode_arguments(args(&["--batch-size", bad]), &allowed, "unknown argument")
                        .unwrap_err()
                        .to_string(),
                    "--batch-size must be between 1 and 50000"
                );
            }
        }
        #[test]
        fn timeline_keeps_null_unknown_aliases_and_exact_gap_boundary() {
            let zero = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc);
            let mut current = None;
            assert_eq!(normalized_state(Some(" Go ")), "drive");
            assert_eq!(normalized_state(None), "unknown");
            assert_eq!(normalized_state(Some("DRİVE")), "unknown");
            assert!(extend_period(&mut current, zero, normalized_state(Some("go"))).is_none());
            assert!(extend_period(
                &mut current,
                zero + chrono::Duration::seconds(600),
                "drive".into()
            )
            .is_none());
            let closed = extend_period(
                &mut current,
                zero + chrono::Duration::seconds(1201),
                "drive".into(),
            )
            .unwrap();
            assert_eq!(closed.start, zero);
            assert_eq!(closed.end, zero + chrono::Duration::seconds(600));
            assert_eq!(
                extend_period(
                    &mut current,
                    zero + chrono::Duration::seconds(1202),
                    "unknown".into()
                )
                .unwrap()
                .state,
                "drive"
            );
        }
    }
}
