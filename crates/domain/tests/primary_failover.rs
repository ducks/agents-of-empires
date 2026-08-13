use std::path::PathBuf;

use aoe_domain::{ArenaManifest, MatchMode};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../arenas/primary-failover")
}

#[test]
fn failover_manifest_and_assets_are_complete() {
    let root = root();
    let manifest = ArenaManifest::load(root.join("arena.toml")).expect("valid failover arena");
    assert_eq!(manifest.arena.mode, MatchMode::BuildRace);
    assert_eq!(manifest.territories.len(), 3);
    let build = manifest.build.expect("build contract");
    assert_eq!(build.milestones.len(), 6);
    assert_eq!(build.completion_milestone, "host-reboot");
    assert_eq!(
        build
            .milestones
            .iter()
            .map(|milestone| milestone.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "failed-primary-preserved",
            "reads-restored",
            "old-primary-fenced",
            "writes-restored",
            "service-restart",
            "host-reboot",
        ]
    );
    for pair in build.milestones.windows(2) {
        assert_eq!(pair[1].depends_on, vec![pair[0].id.clone()]);
    }
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
fn fencing_and_original_state_are_externally_verified() {
    let fence = std::fs::read_to_string(root().join("verify/old-primary-fenced.sh"))
        .expect("fencing verifier");
    assert!(fence.contains("primary.failed"));
    assert!(fence.contains("primary.fenced"));
    assert!(fence.contains(":8081"));
    let reboot =
        std::fs::read_to_string(root().join("verify/host-reboot.sh")).expect("reboot verifier");
    assert!(reboot.contains("primary.fenced"));
    assert!(reboot.contains(":8081"));
    for value in ["red-original", "blue-original", "green-original"] {
        assert!(reboot.contains(value));
    }
    let reads =
        std::fs::read_to_string(root().join("verify/reads-restored.sh")).expect("read verifier");
    for record in [
        "customer-red-4a7",
        "customer-blue-8d2",
        "customer-green-f31",
    ] {
        assert!(reads.contains(record));
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
}
