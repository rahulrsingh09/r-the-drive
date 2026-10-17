//! Process-local admission before opening expensive database/Redis work.
use crate::{config::SecurityConfig, errors::AppError};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Debug, Default)]
struct Counts {
    total: usize,
    users: HashMap<Uuid, usize>,
    vehicles: HashMap<Uuid, usize>,
}

#[derive(Debug, Default)]
pub struct ResourceLimits {
    live: Mutex<Counts>,
    heavy: Mutex<Counts>,
}

#[derive(Debug)]
pub struct ResourcePermit {
    limits: Arc<ResourceLimits>,
    user: Uuid,
    vehicle: Option<Uuid>,
    live: bool,
}

impl ResourceLimits {
    pub fn live(
        self: &Arc<Self>,
        user: Uuid,
        vehicle: Uuid,
        config: &SecurityConfig,
    ) -> Result<ResourcePermit, AppError> {
        self.acquire(
            user,
            Some(vehicle),
            true,
            config.ws_max_per_user,
            config.ws_max_per_vehicle,
            config.ws_max_global,
        )
    }
    pub fn heavy(
        self: &Arc<Self>,
        user: Uuid,
        config: &SecurityConfig,
    ) -> Result<ResourcePermit, AppError> {
        self.acquire(
            user,
            None,
            false,
            config.metrics_max_per_user,
            usize::MAX,
            config.metrics_max_global,
        )
    }
    fn acquire(
        self: &Arc<Self>,
        user: Uuid,
        vehicle: Option<Uuid>,
        live: bool,
        user_max: usize,
        vehicle_max: usize,
        total_max: usize,
    ) -> Result<ResourcePermit, AppError> {
        let bucket = if live { &self.live } else { &self.heavy };
        let mut counts = bucket.lock().expect("resource counts poisoned");
        let usage = [
            (counts.total, total_max),
            (
                counts.users.get(&user).copied().unwrap_or_default(),
                user_max,
            ),
            (
                vehicle
                    .and_then(|id| counts.vehicles.get(&id).copied())
                    .unwrap_or_default(),
                if vehicle.is_some() {
                    vehicle_max
                } else {
                    usize::MAX
                },
            ),
        ];
        if usage.iter().any(|(used, maximum)| used >= maximum) {
            return Err(AppError::ResourceLimited(if live {
                "live_connections"
            } else {
                "heavy_read"
            }));
        }
        *counts.users.entry(user).or_default() += 1;
        if let Some(id) = vehicle {
            *counts.vehicles.entry(id).or_default() += 1;
        }
        counts.total += 1;
        Ok(ResourcePermit {
            limits: Arc::clone(self),
            user,
            vehicle,
            live,
        })
    }
}

impl Drop for ResourcePermit {
    fn drop(&mut self) {
        let mut counts = if self.live {
            &self.limits.live
        } else {
            &self.limits.heavy
        }
        .lock()
        .expect("resource counts poisoned");
        counts.total -= 1;
        decrement(&mut counts.users, self.user);
        if let Some(v) = self.vehicle {
            decrement(&mut counts.vehicles, v);
        }
    }
}
fn decrement(counts: &mut HashMap<Uuid, usize>, key: Uuid) {
    if let std::collections::hash_map::Entry::Occupied(mut entry) = counts.entry(key) {
        let remaining = *entry.get() - 1;
        if remaining == 0 {
            entry.remove();
        } else {
            *entry.get_mut() = remaining;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_is_bounded_and_released_on_drop() {
        let limits = Arc::new(ResourceLimits::default());
        let config = SecurityConfig {
            ws_max_per_user: 1,
            ws_max_per_vehicle: 1,
            ws_max_global: 2,
            ..Default::default()
        };
        let (user, vehicle) = (Uuid::new_v4(), Uuid::new_v4());
        let first = limits.live(user, vehicle, &config).unwrap();
        assert!(limits.live(user, Uuid::new_v4(), &config).is_err());
        assert!(limits.live(Uuid::new_v4(), vehicle, &config).is_err());
        drop(first);
        assert!(limits.live(user, vehicle, &config).is_ok());
        assert!(limits.live.lock().unwrap().users.is_empty());
    }
}
