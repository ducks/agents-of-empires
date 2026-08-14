use std::fs;
use std::path::{Path, PathBuf};

use aoe_controller::{init_arena, validate_arena_package};

fn temp_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("aoe-arena-package-{}-{name}", std::process::id()))
}

#[test]
fn bundled_example_is_a_valid_package() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/hello-service-arena");
    let report = validate_arena_package(&root).expect("validate");
    assert!(report.valid, "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(report.arena.as_deref(), Some("hello-service"));
}

#[test]
fn init_creates_a_valid_renamed_package() {
    let root = temp_dir("init");
    let _ = fs::remove_dir_all(&root);
    let manifest = init_arena("cache-race", &root).expect("init");
    assert_eq!(manifest, root.join("arena.toml"));
    let source = fs::read_to_string(&manifest).expect("manifest");
    assert!(source.contains("id = \"cache-race\""));
    let report = validate_arena_package(&root).expect("validate");
    assert!(report.valid, "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn validator_rejects_references_outside_the_package() {
    let root = temp_dir("escape");
    let _ = fs::remove_dir_all(&root);
    init_arena("escape-test", &root).expect("init");
    let manifest = root.join("arena.toml");
    let source = fs::read_to_string(&manifest).expect("read").replacen(
        "verify/service-up.sh",
        "../answer.sh",
        1,
    );
    fs::write(&manifest, source).expect("write");
    let report = validate_arena_package(&root).expect("validate");
    assert!(!report.valid);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("must stay inside"))
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn init_refuses_to_overwrite_an_existing_package() {
    let root = temp_dir("existing");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("directory");
    fs::write(root.join("keep.txt"), "mine").expect("fixture");
    assert!(init_arena("safe-test", &root).is_err());
    assert_eq!(
        fs::read_to_string(root.join("keep.txt")).expect("preserved"),
        "mine"
    );
    fs::remove_dir_all(root).expect("cleanup");
}
