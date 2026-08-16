use std::collections::HashMap;
use std::fs;

use aoe_controller::{read_provenance, write_provenance};
use aoe_domain::ArenaManifest;

const MANIFEST: &str = include_str!("../../../arenas/first-build/arena.toml");

#[test]
fn provenance_changes_with_verifier_content() {
    let root = std::env::temp_dir().join(format!("aoe-provenance-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("verify")).expect("dirs");
    let manifest_source = MANIFEST
        .replace("verify/service-up.sh", "verify/check.sh")
        .replace("verify/write-read.sh", "verify/check.sh")
        .replace("verify/service-restart.sh", "verify/check.sh")
        .replace("verify/host-reboot.sh", "verify/check.sh");
    let manifest_path = root.join("arena.toml");
    fs::write(&manifest_path, &manifest_source).expect("manifest");
    fs::write(root.join("verify/check.sh"), "#!/bin/sh\nexit 0\n").expect("verifier");
    let manifest = ArenaManifest::parse(&manifest_source).expect("parse");

    let first_dir = root.join("first");
    fs::create_dir(&first_dir).expect("first");
    let adapter = root.join("adapter.sh");
    fs::write(&adapter, "#!/bin/sh\n").expect("adapter");
    let adapters = HashMap::from([("test".into(), adapter)]);
    let first = write_provenance(&manifest_path, &manifest, &adapters, &first_dir)
        .expect("first provenance");
    fs::write(root.join("verify/check.sh"), "#!/bin/sh\nexit 1\n").expect("change verifier");
    let second_dir = root.join("second");
    fs::create_dir(&second_dir).expect("second");
    let second = write_provenance(&manifest_path, &manifest, &adapters, &second_dir)
        .expect("second provenance");

    assert_ne!(first.verifier_sha256, second.verifier_sha256);
    assert_ne!(first.compatibility_key, second.compatibility_key);
    assert_eq!(
        read_provenance(&first_dir.join("match.json"))
            .expect("read")
            .arena_id,
        manifest.arena.id
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn provenance_includes_the_fog_player_brief() {
    let root = std::env::temp_dir().join(format!("aoe-fog-provenance-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("verify")).expect("dirs");
    let manifest_source = MANIFEST
        .replace(
            "[network]",
            "[fog_of_war]\nplayer_brief = \"brief.md\"\n\n[network]",
        )
        .replace("verify/service-up.sh", "verify/check.sh")
        .replace("verify/write-read.sh", "verify/check.sh")
        .replace("verify/service-restart.sh", "verify/check.sh")
        .replace("verify/host-reboot.sh", "verify/check.sh");
    let manifest_path = root.join("arena.toml");
    fs::write(&manifest_path, &manifest_source).expect("manifest");
    fs::write(root.join("brief.md"), "public objective\n").expect("brief");
    fs::write(root.join("verify/check.sh"), "#!/bin/sh\nexit 0\n").expect("verifier");
    let manifest = ArenaManifest::parse(&manifest_source).expect("parse");
    let adapter = root.join("adapter.sh");
    fs::write(&adapter, "#!/bin/sh\n").expect("adapter");
    let adapters = HashMap::from([("test".into(), adapter)]);
    let first_dir = root.join("first");
    fs::create_dir(&first_dir).expect("first");
    let first = write_provenance(&manifest_path, &manifest, &adapters, &first_dir)
        .expect("first provenance");
    fs::write(root.join("brief.md"), "changed objective\n").expect("change brief");
    let second_dir = root.join("second");
    fs::create_dir(&second_dir).expect("second");
    let second = write_provenance(&manifest_path, &manifest, &adapters, &second_dir)
        .expect("second provenance");
    assert!(first.player_brief_sha256.is_some());
    assert_ne!(first.player_brief_sha256, second.player_brief_sha256);
    assert_ne!(first.compatibility_key, second.compatibility_key);
    fs::remove_dir_all(root).expect("cleanup");
}
