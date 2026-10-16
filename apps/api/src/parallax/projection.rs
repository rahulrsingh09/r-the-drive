//! Canonical wire decoding and projection have separate, pure stages.
use super::*;

pub(super) enum CanonicalReading {
    Power(PowerStateMessage),
    Gnss(GnssState),
    Odometer(OdometerState),
    Gear(GearState),
    Closures(ClosureStates),
    Locks(LockStates),
    Tires(TireStates),
    Temperatures(CabinTemperatures),
    Preconditioning(PreconditioningState),
    Defrost(DefrostState),
}

impl CanonicalReading {
    pub fn decode(topic: &str, payload: &[u8]) -> Result<Option<Self>> {
        Ok(Some(match topic {
            "vehicle.power.state" => Self::Power(PowerStateMessage::decode(payload)?),
            "dynamics.vehicle.gnss" => Self::Gnss(GnssState::decode(payload)?),
            "dynamics.vehicle.odometer" => Self::Odometer(OdometerState::decode(payload)?),
            "dynamics.vehicle.gear" => Self::Gear(GearState::decode(payload)?),
            "body.closures.states" => Self::Closures(ClosureStates::decode(payload)?),
            "body.locks.states" => Self::Locks(LockStates::decode(payload)?),
            "dynamics.tires.state" => Self::Tires(TireStates::decode(payload)?),
            "comfort.cabin.cabin_temperatures" => {
                Self::Temperatures(CabinTemperatures::decode(payload)?)
            }
            "comfort.cabin.cabin_preconditioning_status" => {
                Self::Preconditioning(PreconditioningState::decode(payload)?)
            }
            "comfort.cabin.defrost_defog_status" => Self::Defrost(DefrostState::decode(payload)?),
            _ => return Ok(None),
        }))
    }

    pub fn project(
        self,
        empty_payload: bool,
        source_at: DateTime<Utc>,
        vehicle_id: Uuid,
        notes: &mut Vec<String>,
    ) -> Result<Option<TelemetryEvent>> {
        let mut event = TelemetryEvent::empty(vehicle_id, source_at);
        let mut meaningful = false;
        match self {
            Self::Power(value) => {
                let state = match value.state.context("missing power state")? {
                    1 => PowerState::Sleep,
                    2 => PowerState::Unknown, // Rivian's standby has no canonical field.
                    3 => PowerState::Ready,
                    4 => PowerState::Go,
                    state => anyhow::bail!("unknown Parallax power state {state}"),
                };
                event.power_state = Some(state);
                event.power_state_ts = Some(source_at);
                meaningful = true;
            }
            Self::Gnss(value) => {
                let latitude = value.latitude.context("missing GNSS latitude")?;
                let longitude = value.longitude.context("missing GNSS longitude")?;
                if !latitude.is_finite()
                    || !(-90.0..=90.0).contains(&latitude)
                    || !longitude.is_finite()
                    || !(-180.0..=180.0).contains(&longitude)
                {
                    anyhow::bail!("invalid GNSS coordinates");
                }
                event.latitude = Some(latitude);
                event.longitude = Some(longitude);
                event.location_ts = Some(source_at);
                if let Some(altitude) = value.altitude_m {
                    if !altitude.is_finite() || !(-1_000.0..=20_000.0).contains(&altitude) {
                        anyhow::bail!("invalid GNSS altitude");
                    }
                    event.altitude_m = Some(altitude);
                }
                meaningful = true;
            }
            Self::Odometer(value) => {
                let kilometers = value.kilometers.context("missing odometer")?;
                if kilometers > 2_000_000 {
                    anyhow::bail!("invalid odometer");
                }
                event.odometer_miles = Some(kilometers as f64 * 0.621_371_192);
                event.odometer_miles_ts = Some(source_at);
                meaningful = true;
            }
            Self::Gear(value) => {
                // Gear enum as observed on an R1S by apohor/rivolt: 1 P, 2 R,
                // 4 D; 3 N is inferred from the ordering. Values match the legacy
                // gearStatus strings.
                let gear = match value.gear.context("missing gear")? {
                    1 => Some("park"),
                    2 => Some("reverse"),
                    3 => Some("neutral"),
                    4 => Some("drive"),
                    other => {
                        notes.push(format!("skipped unknown gear {other}"));
                        None
                    }
                };
                if let Some(gear) = gear {
                    event.gear_status = Some(gear.into());
                    meaningful = true;
                }
            }
            Self::Closures(value) => {
                for state in value.states {
                    let position = state.position.context("missing closure position")?;
                    // Every frame ends with this marker: stateless on the R2,
                    // mirroring the window state on the R1.
                    if position == CLOSURE_SENTINEL_POSITION {
                        continue;
                    }
                    // Protobuf omits a zero status, which the app enum defines as
                    // unspecified: the vehicle does not have this closure (for
                    // example the side bin or tonneau on an R1S).
                    let Some(status) = state.state.filter(|status| *status != 0) else {
                        continue;
                    };
                    if position == CLOSURE_CHARGE_PORT_POSITION {
                        // Same meaning as legacy chargePortState: open or ajar
                        // count as open; opening, closing, and closed do not.
                        if (1..=5).contains(&status) {
                            event.charge_port_open = Some(matches!(status, 1 | 3));
                            meaningful = true;
                        } else {
                            notes.push(format!(
                                "skipped charge port with unexpected state {status}"
                            ));
                        }
                        continue;
                    }
                    let field = closure_field(&mut event, position);
                    if field.is_none() && position != CLOSURE_REAR_GLASS_POSITION {
                        notes.push(format!(
                            "skipped unmapped closure position {position} with state {status}"
                        ));
                        continue;
                    }
                    // App enum: 1 OPEN, 2 CLOSE, 3 AJAR, 4 OPENING, 5 CLOSING.
                    // Every state except CLOSE is "not closed", as legacy reports.
                    let closed = match status {
                        2 => true,
                        1 | 3 | 4 | 5 => false,
                        other => {
                            notes.push(format!(
                                "skipped closure position {position} with unexpected state {other}"
                            ));
                            continue;
                        }
                    };
                    if let Some(field) = field {
                        *field = Some(closed);
                        meaningful = true;
                    }
                    let transition = match status {
                        3 => Some(ClosureTransition::Ajar),
                        4 => Some(ClosureTransition::Opening),
                        5 => Some(ClosureTransition::Closing),
                        _ => None,
                    };
                    if let (Some(transition), Some(name)) =
                        (transition, closure_field_name(position))
                    {
                        event
                            .closure_transitions
                            .get_or_insert_with(Default::default)
                            .insert(name.to_owned(), transition);
                    }
                }
            }
            Self::Locks(value) => {
                for state in value.states {
                    let position = state.position.context("missing lock position")?;
                    // R1 vehicles also report positions with no canonical field
                    // (14, 15). Skip those entries rather than losing the other
                    // locks in the same frame.
                    if lock_field_name(position).is_none() {
                        notes.push(format!("skipped unmapped lock position {position}"));
                        continue;
                    }
                    // App enum: 1 LOCKED, 2 UNLOCKED, 3 PARTIALLY_UNLOCKED.
                    let locked = match state.state {
                        Some(1) => true,
                        Some(2 | 3) => false,
                        other => {
                            let reason = other.map_or_else(
                                || "missing state".into(),
                                |s| format!("unexpected state {s}"),
                            );
                            notes.push(format!("skipped lock position {position} with {reason}"));
                            continue;
                        }
                    };
                    meaningful |= set_lock(&mut event, position, locked)?;
                }
            }
            Self::Tires(value) => {
                for state in value.states {
                    let position = state.position.context("missing tire position")?;
                    let status = state.status.context("missing tire status")?;
                    if !matches!(status, 1 | 2) {
                        anyhow::bail!("unknown tire status {status}");
                    }
                    let pressure_bar = state.pressure_bar.context("missing tire pressure")?;
                    if !pressure_bar.is_finite() || !(0.0..=10.0).contains(&pressure_bar) {
                        anyhow::bail!("invalid tire pressure");
                    }
                    meaningful |= set_tire(&mut event, position, pressure_bar, status == 1)?;
                }
            }
            Self::Temperatures(value) => {
                if let Some(cabin) = value.cabin_c {
                    event.cabin_temp_c = Some(valid_temperature(cabin as f64)?);
                    meaningful = true;
                }
                if let Some(driver) = value.driver_c {
                    event.driver_temp_c = Some(valid_temperature(driver as f64)?);
                    meaningful = true;
                }
            }
            Self::Preconditioning(value) => {
                let status = match value.status {
                    Some(1 | 2) => Some("initiate"),
                    Some(4) => Some("active"),
                    Some(0 | 3) => Some("off"),
                    None if empty_payload => Some("off"),
                    Some(other) => {
                        notes.push(format!("skipped unknown preconditioning status {other}"));
                        None
                    }
                    None => {
                        notes.push("skipped preconditioning frame with missing status".into());
                        None
                    }
                };
                if let Some(status) = status {
                    event.cabin_precon_status = Some(status.into());
                    meaningful = true;
                }
            }
            Self::Defrost(value) => {
                let status = match value.status.context("missing defrost status")? {
                    // The R1S reports 4 whenever legacy reports "Off"; as in
                    // rivian-python-client, only 2 means defrosting.
                    0 | 1 | 4 => Some(false),
                    2 => Some(true),
                    other => {
                        notes.push(format!("skipped unknown defrost status {other}"));
                        None
                    }
                };
                if let Some(status) = status {
                    event.defrost_active = Some(status);
                    meaningful = true;
                }
            }
        }
        Ok(meaningful.then_some(event))
    }
}
