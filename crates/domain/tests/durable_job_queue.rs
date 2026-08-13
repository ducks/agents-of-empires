use std::path::PathBuf;

use aoe_domain::{ArenaManifest, MatchMode};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../arenas/durable-job-queue")
}

#[test]
fn durable_job_queue_manifest_and_assets_are_complete() {
    let root = root();
    let manifest = ArenaManifest::load(root.join("arena.toml")).expect("valid queue arena");
    assert_eq!(manifest.arena.mode, MatchMode::BuildRace);
    assert_eq!(manifest.territories.len(), 3);
    let build = manifest.build.expect("build contract");
    assert_eq!(build.milestones.len(), 5);
    assert_eq!(build.completion_milestone, "host-reboot");
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
fn verifier_requires_original_jobs_and_exactly_one_attempt() {
    let source = std::fs::read_to_string(root().join("verify/recover-accepted.sh"))
        .expect("recovery verifier");
    for job in [
        "accepted-alpha-7d3",
        "accepted-beta-91e",
        "accepted-gamma-c42",
    ] {
        assert!(source.contains(job));
    }
    assert!(source.contains(".attempts==1"));
    assert!(source.contains(".status==\"completed\""));
}

#[test]
fn real_fleet_changes_only_arena_label_and_agents() {
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
