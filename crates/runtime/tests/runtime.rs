use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use aoe_domain::{ArenaManifest, TerritoryConfig};
use aoe_runtime::{
    ArenaSupervisor, NetworkAssignment, NetworkPlan, RuntimeError, TerritoryDriver, TerritoryHandle,
};
use async_trait::async_trait;

const MANIFEST: &str = include_str!("fixture.toml");

#[derive(Default)]
struct FakeDriver {
    fail: Option<String>,
    running: Mutex<HashSet<String>>,
    stopped: Mutex<Vec<String>>,
}

#[async_trait]
impl TerritoryDriver for FakeDriver {
    async fn boot(
        &self,
        territory: &TerritoryConfig,
        network: &NetworkAssignment,
    ) -> Result<TerritoryHandle, RuntimeError> {
        if self.fail.as_deref() == Some(&territory.id) {
            return Err(RuntimeError::Boot {
                territory: territory.id.clone(),
                detail: "planned failure".into(),
            });
        }
        self.running
            .lock()
            .expect("running lock")
            .insert(territory.id.clone());
        Ok(TerritoryHandle {
            territory: territory.id.clone(),
            process_id: None,
            network: network.clone(),
        })
    }

    async fn stop(&self, handle: &TerritoryHandle) -> Result<(), RuntimeError> {
        self.running
            .lock()
            .expect("running lock")
            .remove(&handle.territory);
        self.stopped
            .lock()
            .expect("stopped lock")
            .push(handle.territory.clone());
        Ok(())
    }
}

#[test]
fn network_plan_has_unique_restricted_assignments() {
    let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
    let plan = NetworkPlan::from_manifest(&manifest, 24000, 23900).expect("network plan");
    assert_eq!(plan.assignments.len(), 2);
    assert_ne!(plan.assignments[0].ssh_port, plan.assignments[1].ssh_port);
    assert_ne!(plan.assignments[0].arena_ip, plan.assignments[1].arena_ip);
    let options = plan.assignments[0].qemu_net_opts();
    assert!(options.contains("restrict=on"));
    assert!(options.contains("mcast=230.77.0.1:23900"));
    assert!(!options.contains("hostfwd=tcp::"));
}

#[tokio::test]
async fn supervisor_boots_and_stops_all_territories() {
    let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
    let plan = NetworkPlan::from_manifest(&manifest, 24000, 23900).expect("network plan");
    let driver = Arc::new(FakeDriver::default());
    let mut supervisor = ArenaSupervisor::new(Arc::clone(&driver));
    supervisor
        .boot_all(&manifest, &plan)
        .await
        .expect("boot all");
    assert_eq!(supervisor.handles().len(), 2);
    assert!(supervisor.stop_all().await.is_empty());
    assert!(supervisor.handles().is_empty());
    assert_eq!(driver.stopped.lock().expect("lock").len(), 2);
}

#[tokio::test]
async fn supervisor_stops_one_territory() {
    let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
    let plan = NetworkPlan::from_manifest(&manifest, 24000, 23900).expect("network plan");
    let driver = Arc::new(FakeDriver::default());
    let mut supervisor = ArenaSupervisor::new(Arc::clone(&driver));
    supervisor
        .boot_all(&manifest, &plan)
        .await
        .expect("boot all");
    supervisor.stop("gate").await.expect("stop gate");
    assert!(!supervisor.handles().contains_key("gate"));
    assert!(supervisor.handles().contains_key("archive"));
    assert_eq!(driver.stopped.lock().expect("lock").as_slice(), ["gate"]);
}

#[tokio::test]
async fn partial_boot_cleans_up_successful_guests() {
    let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
    let plan = NetworkPlan::from_manifest(&manifest, 24000, 23900).expect("network plan");
    let driver = Arc::new(FakeDriver {
        fail: Some("archive".into()),
        ..FakeDriver::default()
    });
    let mut supervisor = ArenaSupervisor::new(Arc::clone(&driver));
    assert!(matches!(
        supervisor.boot_all(&manifest, &plan).await,
        Err(RuntimeError::PartialBoot(_))
    ));
    assert!(driver.running.lock().expect("lock").is_empty());
    assert_eq!(driver.stopped.lock().expect("lock").as_slice(), ["gate"]);
}
