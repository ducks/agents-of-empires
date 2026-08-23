use std::path::PathBuf;

use aoe_domain::{ArenaManifest, MatchMode};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../arenas/blueprint-build")
}

#[test]
fn blueprint_manifest_and_player_artifacts_are_complete() {
    let root = root();
    let manifest = ArenaManifest::load(root.join("arena.toml")).expect("valid blueprint arena");
    assert_eq!(manifest.arena.mode, MatchMode::BuildRace);
    assert_eq!(manifest.territories.len(), 3);
    let fog = manifest.fog_of_war.as_ref().expect("fog boundary");
    assert!(root.join(&fog.player_brief).is_file());
    assert_eq!(fog.player_artifacts, vec!["blueprint.png"]);
    assert!(root.join(&fog.player_artifacts[0]).is_file());
    let build = manifest.build.expect("build contract");
    assert_eq!(build.milestones.len(), 5);
    assert_eq!(build.completion_milestone, "host-reboot");
    for milestone in build.milestones {
        assert!(root.join(milestone.verifier).is_file());
    }
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
    assert!(real.agents.iter().all(|agent| agent.adapter == "claux"));
}

#[test]
fn real_fleet_uses_models_verified_with_image_input() {
    let real = ArenaManifest::load(root().join("agents-real.toml")).expect("real manifest");
    let models: Vec<_> = real
        .agents
        .iter()
        .map(|agent| agent.model.as_str())
        .collect();
    assert_eq!(
        models,
        [
            "stealth/ox-alpha",
            "openai/gpt-5.6-luna",
            "moonshotai/kimi-k3",
        ]
    );
}

#[test]
fn referee_stops_worker_before_testing_acceptance() {
    let source = std::fs::read_to_string(root().join("verify/accept-under-stop.sh"))
        .expect("accept verifier");
    assert!(source.contains("systemctl stop blueprint-worker.service"));
    assert!(source.contains("--data-binary"));
    assert!(source.contains("== 409"));
}
