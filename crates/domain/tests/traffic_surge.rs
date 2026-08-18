use std::path::PathBuf;

use aoe_domain::{ArenaManifest, MatchMode};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../arenas/traffic-surge")
}

#[test]
fn traffic_surge_manifest_and_assets_are_complete() {
    let root = root();
    let manifest = ArenaManifest::load(root.join("arena.toml")).expect("valid surge arena");
    assert_eq!(manifest.arena.mode, MatchMode::BuildRace);
    assert_eq!(manifest.territories.len(), 3);
    let fog = manifest.fog_of_war.as_ref().expect("fog-of-war boundary");
    assert!(root.join(&fog.player_brief).is_file());
    assert!(fog.hide_topology_until_observed);
    assert!(fog.guest_leak_audit.is_some());

    let build = manifest.build.expect("build contract");
    assert_eq!(build.completion_milestone, "host-reboot");
    assert_eq!(
        build
            .milestones
            .iter()
            .map(|milestone| milestone.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "baseline-ready",
            "priority-first-surge",
            "bounded-optional",
            "recover-peak",
            "restart-under-load",
            "host-reboot",
        ]
    );
    for pair in build.milestones.windows(2) {
        assert_eq!(pair[1].depends_on, vec![pair[0].id.clone()]);
    }
    for milestone in build.milestones {
        assert!(root.join(milestone.verifier).is_file());
    }
}

#[test]
fn controller_generates_real_pressure_against_an_intentionally_serial_service() {
    let guest = std::fs::read_to_string(root().join("nix/base.nix")).expect("guest module");
    assert!(guest.contains("HTTPServer((\"0.0.0.0\", 8080)"));
    assert!(!guest.contains("ThreadingHTTPServer"));
    assert!(guest.contains("time.sleep(0.45)"));

    let first_surge = std::fs::read_to_string(root().join("verify/priority-first-surge.sh"))
        .expect("first surge verifier");
    assert!(first_surge.contains("seq 1 8"));
    assert!(first_surge.contains("--max-time 1.5"));
    assert!(first_surge.contains("/optional/first-"));
    assert!(first_surge.contains("/priority/$record"));

    let peak =
        std::fs::read_to_string(root().join("verify/recover-peak.sh")).expect("peak verifier");
    assert!(peak.contains("seq 1 24"));
    assert!(peak.contains("seq 1 4"));
}

#[test]
fn verifier_preserves_historical_and_accepted_priority_state() {
    let baseline =
        std::fs::read_to_string(root().join("verify/baseline-ready.sh")).expect("baseline");
    let reboot = std::fs::read_to_string(root().join("verify/host-reboot.sh")).expect("reboot");
    for (record, value) in [
        ("history-amber-a91", "ledger-alpha"),
        ("history-cobalt-f73", "ledger-beta"),
        ("history-umber-2d4", "ledger-gamma"),
    ] {
        assert!(baseline.contains(record));
        assert!(baseline.contains(value));
        assert!(reboot.contains(record));
        assert!(reboot.contains(value));
    }
    assert!(reboot.contains("restart-under-load"));
}

#[test]
fn real_fleet_changes_only_agents() {
    let root = root();
    let oracle = ArenaManifest::load(root.join("arena.toml")).expect("oracle manifest");
    let real = ArenaManifest::load(root.join("agents-real.toml")).expect("real manifest");
    assert_eq!(oracle.arena, real.arena);
    assert_eq!(oracle.network, real.network);
    assert_eq!(oracle.rules, real.rules);
    assert_eq!(oracle.build, real.build);
    assert_eq!(oracle.visualization, real.visualization);
    assert_eq!(oracle.fog_of_war, real.fog_of_war);
    assert_eq!(oracle.classes, real.classes);
    assert_eq!(oracle.territories, real.territories);
    assert_eq!(
        real.agents
            .iter()
            .map(|agent| agent.model.as_str())
            .collect::<Vec<_>>(),
        vec![
            "deepseek/deepseek-v4-flash-0731",
            "openai/gpt-5.6-luna",
            "z-ai/glm-5.2"
        ]
    );
    assert!(real.agents.iter().all(|agent| agent.adapter == "claux"));
}
