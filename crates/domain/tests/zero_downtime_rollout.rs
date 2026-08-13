use std::path::PathBuf;

use aoe_domain::{ArenaManifest, MatchMode};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../arenas/zero-downtime-rollout")
}

#[test]
fn rollout_manifest_and_assets_are_complete() {
    let root = root();
    let manifest = ArenaManifest::load(root.join("arena.toml")).expect("valid rollout arena");
    assert_eq!(manifest.arena.mode, MatchMode::BuildRace);
    assert_eq!(manifest.territories.len(), 3);
    let build = manifest.build.expect("build contract");
    assert_eq!(build.milestones.len(), 5);
    assert_eq!(build.completion_milestone, "host-reboot");
    assert_eq!(build.milestones[1].id, "uninterrupted-cutover");
    for milestone in build.milestones {
        assert!(root.join(milestone.verifier).is_file());
    }
    for territory in manifest.territories {
        assert!(
            root.join("instructions")
                .join(format!("{}.md", territory.id))
                .is_file()
        );
    }
}

#[test]
fn rollout_verifier_watches_continuity_and_original_state() {
    let cutover = std::fs::read_to_string(root().join("verify/uninterrupted-cutover.sh"))
        .expect("cutover verifier");
    let monitor = std::fs::read_to_string(root().join("verify/continuity-monitor.sh"))
        .expect("continuity monitor");
    assert!(monitor.contains("customer-alpha-73c"));
    assert!(monitor.contains("customer-beta-a19"));
    assert!(monitor.contains("sleep 0.05"));
    assert!(monitor.contains("$audit/failures"));
    assert!(cutover.contains("failures"));
    assert!(cutover.contains("rollout-v1.service rollout-v2.service rollout-proxy.service"));
    assert!(cutover.contains("/var/lib/rollout/upstream"));
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
