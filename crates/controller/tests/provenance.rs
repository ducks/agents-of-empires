use std::fs;

use aoe_controller::{read_provenance, write_provenance};
use aoe_domain::ArenaManifest;

const MANIFEST: &str = include_str!("../../runtime/tests/fixture.toml");

#[test]
fn provenance_changes_with_verifier_content() {
    let root = std::env::temp_dir().join(format!("aoe-provenance-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("verify")).expect("dirs");
    let manifest_source = MANIFEST.replace("controller/verify.sh", "verify/check.sh");
    let manifest_path = root.join("arena.toml");
    fs::write(&manifest_path, &manifest_source).expect("manifest");
    fs::write(root.join("verify/check.sh"), "#!/bin/sh\nexit 0\n").expect("verifier");
    let manifest = ArenaManifest::parse(&manifest_source).expect("parse");

    let first_dir = root.join("first");
    fs::create_dir(&first_dir).expect("first");
    let first = write_provenance(&manifest_path, &manifest, &first_dir).expect("first provenance");
    fs::write(root.join("verify/check.sh"), "#!/bin/sh\nexit 1\n").expect("change verifier");
    let second_dir = root.join("second");
    fs::create_dir(&second_dir).expect("second");
    let second =
        write_provenance(&manifest_path, &manifest, &second_dir).expect("second provenance");

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
