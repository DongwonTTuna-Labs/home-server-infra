use std::path::Path;

use serde::Serialize;

use crate::config::SupervisorConfig;
use crate::contracts::health::HealthStatus;
use crate::contracts::projection::ProjectionState;
use crate::errors::LifecycleError;
use crate::locks;
use crate::records;
use crate::runtime::provider_limit_state;
use crate::runtime::{DockerStatus, ProviderReadiness, RuntimeObservation, RuntimeProbe};
use crate::slots::{canonical_inventory, inventory, SlotConfig};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolStatusDecision {
    pub message: String,
    pub reason: Option<String>,
    pub result_kind: &'static str,
    pub slot_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SlotStatusView {
    pub slot_id: String,
    pub account_group: String,
    pub container: String,
    pub status: String,
    pub allocatable: bool,
    pub persisted_status: Option<String>,
    pub docker_status: DockerStatus,
    pub cdp_reachable: Option<bool>,
    pub provider_readiness: ProviderReadiness,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatusView {
    pub schema: String,
    pub state: String,
    pub state_dir: String,
    pub holders: usize,
    pub locks: usize,
    pub slot_pool_enabled: bool,
    pub slot_pool_total: u8,
    pub slots: Vec<SlotStatusView>,
}

pub fn build_status(
    config: &SupervisorConfig,
    runtime: &dyn RuntimeProbe,
) -> Result<StatusView, LifecycleError> {
    let slots = inventory(config)
        .into_iter()
        .map(|slot| build_slot_status(config, runtime, slot))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StatusView {
        schema: "gpt-webai.lifecycle.status.v2".to_string(),
        state: "idle".to_string(),
        state_dir: config.state_root.display().to_string(),
        holders: records::holder_count(&config.state_root),
        locks: records::lock_count(&config.state_root),
        slot_pool_enabled: config.slot_pool_enabled(),
        slot_pool_total: config.slot_count,
        slots,
    })
}

pub fn aggregate_r13_status(
    config: &SupervisorConfig,
    runtime: &dyn RuntimeProbe,
    state: &ProjectionState,
) -> PoolStatusDecision {
    let now = crate::config::now_ms();
    let slots = canonical_inventory(config);
    let classified = slots
        .iter()
        .map(|slot| {
            let obstruction = crate::allocator::classify_slot(state, &slot.slot_id.0, now)
                .unwrap_or(Some(crate::allocator::scan::SkipReason::StateInvalid));
            (slot, obstruction)
        })
        .collect::<Vec<_>>();

    if let Some((slot, _)) = classified
        .iter()
        .find(|(_, obstruction)| obstruction.is_none())
    {
        return PoolStatusDecision {
            message: format!(
                "pool readiness: {} is allocatable across the canonical ten-slot domain",
                slot.slot_id.0
            ),
            reason: None,
            result_kind: "status.ready",
            slot_id: Some(slot.slot_id.0.clone()),
        };
    }

    let recorded = classified
        .iter()
        .filter_map(|(slot, _)| {
            state.slots.get(&slot.slot_id.0).and_then(|record| {
                HealthStatus::parse(&record.health_status).map(|health| (slot, health))
            })
        })
        .collect::<Vec<_>>();

    let observations = if recorded.is_empty() {
        observe_unrecorded_slots_with_single_retry(runtime, &classified)
    } else {
        Vec::new()
    };
    let attempted = observations
        .iter()
        .filter(|(_, observation)| {
            !matches!(
                observation.provider_readiness,
                ProviderReadiness::NotChecked
            )
        })
        .collect::<Vec<_>>();
    if !attempted.is_empty()
        && attempted.iter().all(|(_, observation)| {
            matches!(
                observation.provider_readiness,
                ProviderReadiness::Unreachable
            )
        })
    {
        let slot = &attempted[0].0.slot_id.0;
        return PoolStatusDecision {
            message: format!(
                "pool readiness: every attempted runtime probe failed; lowest probed slot={slot}"
            ),
            reason: Some("status.runtime_probe_failed".to_string()),
            result_kind: "status.runtime_probe_failed",
            slot_id: Some(slot.clone()),
        };
    }

    if recorded.len() == crate::allocator::CANONICAL_SLOTS.len()
        && recorded
            .iter()
            .all(|(_, health)| health.is_authentication_block())
    {
        let (slot, health) = recorded[0];
        return PoolStatusDecision {
            message: format!(
                "pool readiness: all canonical slots are authentication-blocked; lowest slot={} healthStatus={}",
                slot.slot_id.0, health
            ),
            reason: None,
            result_kind: "status.blocked",
            slot_id: Some(slot.slot_id.0.clone()),
        };
    }

    let determining_slot = classified
        .iter()
        .find(|(slot, obstruction)| {
            let transient_health = state
                .slots
                .get(&slot.slot_id.0)
                .and_then(|record| HealthStatus::parse(&record.health_status))
                .is_some_and(|health| {
                    matches!(
                        health,
                        HealthStatus::ProviderLimit
                            | HealthStatus::SchemaDrift
                            | HealthStatus::Unknown
                            | HealthStatus::Unreachable
                    )
                });
            let non_health_obstruction = matches!(
                obstruction,
                Some(
                    crate::allocator::scan::SkipReason::Leased
                        | crate::allocator::scan::SkipReason::RuntimeOwned
                        | crate::allocator::scan::SkipReason::Cooldown
                        | crate::allocator::scan::SkipReason::StateInvalid
                )
            );
            transient_health || non_health_obstruction
        })
        .or_else(|| classified.first())
        .map(|(slot, _)| slot.slot_id.0.clone());
    let detail = determining_slot
        .as_deref()
        .and_then(|slot_id| state.slots.get(slot_id))
        .map(|slot| format!(" healthStatus={}", slot.health_status))
        .unwrap_or_default();
    PoolStatusDecision {
        message: format!(
            "pool readiness: no canonical slot is allocatable; lowest obstruction={}{}",
            determining_slot.as_deref().unwrap_or("none"),
            detail
        ),
        reason: None,
        result_kind: "status.degraded",
        slot_id: determining_slot,
    }
}

fn observe_unrecorded_slots_with_single_retry<'a>(
    runtime: &dyn RuntimeProbe,
    classified: &[(&'a SlotConfig, Option<crate::allocator::scan::SkipReason>)],
) -> Vec<(&'a SlotConfig, RuntimeObservation)> {
    let mut observations = classified
        .iter()
        .map(|(slot, _)| (*slot, runtime.observe(slot)))
        .collect::<Vec<_>>();
    let retry_indices = observations
        .iter()
        .enumerate()
        .filter_map(|(index, (_, observation))| {
            observation
                .provider_readiness
                .health_status()
                .and_then(|health| {
                    crate::allocator::health::map_health(health, None).retry_after_ms
                })
                .map(|delay_ms| (index, delay_ms))
        })
        .collect::<Vec<_>>();
    if let Some(delay_ms) = retry_indices.iter().map(|(_, delay)| *delay).max() {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        for (index, _) in retry_indices {
            let slot = observations[index].0;
            observations[index].1 = runtime.observe(slot);
        }
    }
    observations
}

fn build_slot_status(
    config: &SupervisorConfig,
    runtime: &dyn RuntimeProbe,
    slot: SlotConfig,
) -> Result<SlotStatusView, LifecycleError> {
    let state_file = config
        .state_root
        .join("slots")
        .join(format!("{}.state", slot.slot_id.0));
    let values = records::read_key_value_file(&state_file)?;
    let persisted_status = values.get("status").cloned();
    let RuntimeObservation {
        docker_status,
        cdp_reachable,
        provider_readiness,
    } = runtime.observe(&slot);
    let persisted_for_reconcile = persisted_for_reconcile(&values, &state_file, &docker_status);
    let mut status =
        reconciled_slot_status(persisted_for_reconcile, &docker_status, &provider_readiness);
    if locks::slot_lock_exists(config.state_root.as_path(), &slot.slot_id.0) {
        status = "leased".to_string();
    }
    let allocatable = matches!(status.as_str(), "ready" | "standby");
    Ok(SlotStatusView {
        slot_id: slot.slot_id.0,
        account_group: slot.account_group.0,
        container: slot.container,
        status,
        allocatable,
        persisted_status,
        docker_status,
        cdp_reachable,
        provider_readiness,
    })
}

fn persisted_for_reconcile<'a>(
    values: &'a std::collections::BTreeMap<String, String>,
    state_file: &Path,
    docker_status: &DockerStatus,
) -> Option<&'a str> {
    let persisted = values.get("status").map(String::as_str);
    if matches!(
        docker_status,
        DockerStatus::Exited | DockerStatus::Missing | DockerStatus::Skipped
    ) && provider_limit_state::recheck_due(values, state_file, crate::config::now_system_time())
    {
        return Some("standby");
    }
    persisted
}

pub fn reconciled_slot_status(
    persisted: Option<&str>,
    docker_status: &DockerStatus,
    provider_readiness: &ProviderReadiness,
) -> String {
    match docker_status {
        DockerStatus::Exited => {
            return match persisted {
                Some("auth.needs_login" | "auth.needs_pro" | "provider.limit" | "degraded") => {
                    persisted.unwrap().to_string()
                }
                Some("standby") => "standby".to_string(),
                _ => "exited".to_string(),
            };
        }
        DockerStatus::Missing => return "standby".to_string(),
        DockerStatus::Skipped => {
            return persisted.unwrap_or("standby").to_string();
        }
        DockerStatus::Unknown => {
            if matches!(
                persisted,
                Some("busy" | "leased" | "repairing" | "warming" | "degraded" | "reseed_login")
            ) {
                return persisted.unwrap().to_string();
            }
            return "unknown".to_string();
        }
        DockerStatus::Running => {}
    }

    if matches!(
        provider_readiness,
        ProviderReadiness::Ready | ProviderReadiness::ReadyModelCorrectionRequired
    ) && matches!(
        persisted,
        Some("auth.needs_login" | "auth.needs_pro" | "provider.limit" | "degraded" | "warming")
    ) {
        return "ready".to_string();
    }

    match persisted {
        Some("busy" | "leased" | "repairing" | "warming" | "degraded" | "reseed_login") => {
            persisted.unwrap().to_string()
        }
        Some("auth.needs_login" | "auth.needs_pro" | "provider.limit") => {
            persisted.unwrap().to_string()
        }
        Some("standby") => "standby".to_string(),
        _ => match provider_readiness {
            ProviderReadiness::Ready | ProviderReadiness::ReadyModelCorrectionRequired => {
                "ready".to_string()
            }
            ProviderReadiness::LoginRequired => "auth.needs_login".to_string(),
            ProviderReadiness::SubscriptionRequired => "auth.needs_pro".to_string(),
            ProviderReadiness::ProviderLimit => "provider.limit".to_string(),
            ProviderReadiness::Unreachable
            | ProviderReadiness::SchemaDrift
            | ProviderReadiness::Unknown => "degraded".to_string(),
            ProviderReadiness::NotChecked => "warming".to_string(),
        },
    }
}

pub fn write_legacy_kv(
    status: &StatusView,
    mut writer: impl std::io::Write,
) -> std::io::Result<()> {
    writeln!(writer, "state={}", status.state)?;
    writeln!(writer, "state_dir={}", status.state_dir)?;
    writeln!(writer, "holders={}", status.holders)?;
    writeln!(writer, "locks={}", status.locks)?;
    writeln!(
        writer,
        "slot_pool_enabled={}",
        if status.slot_pool_enabled { 1 } else { 0 }
    )?;
    writeln!(writer, "slot_pool_total={}", status.slot_pool_total)?;
    for slot in &status.slots {
        let key = slot.slot_id.replace('-', "_");
        writeln!(writer, "{key}_account_group={}", slot.account_group)?;
        writeln!(writer, "{key}_status={}", slot.status)?;
        writeln!(
            writer,
            "{key}_allocatable={}",
            if slot.allocatable { 1 } else { 0 }
        )?;
        writeln!(writer, "{key}_docker_status={:?}", slot.docker_status)?;
        writeln!(
            writer,
            "{key}_provider_readiness={:?}",
            slot.provider_readiness
        )?;
    }
    Ok(())
}

pub fn source_state_file(state_root: &Path, slot_id: &str) -> std::path::PathBuf {
    state_root.join("slots").join(format!("{slot_id}.state"))
}
