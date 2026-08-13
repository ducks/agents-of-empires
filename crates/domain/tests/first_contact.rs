use std::fs;
use std::path::PathBuf;

use aoe_domain::ArenaManifest;

fn arena_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../arenas/first-contact")
}

#[test]
fn first_contact_manifest_is_valid_and_asymmetric() {
    let root = arena_root();
    let manifest = ArenaManifest::load(root.join("arena.toml")).expect("valid arena");
    assert_eq!(manifest.classes.len(), 3);
    assert_eq!(manifest.territories.len(), 3);
    assert_eq!(manifest.agents.len(), 3);
    let mut classes: Vec<_> = manifest
        .territories
        .iter()
        .map(|territory| territory.class.as_str())
        .collect();
    classes.sort_unstable();
    classes.dedup();
    assert_eq!(classes.len(), 3);
    assert!(!manifest.network.allow_public_internet);
}

#[test]
fn every_territory_has_instruction_oracle_and_nix_module() {
    let root = arena_root();
    let manifest = ArenaManifest::load(root.join("arena.toml")).expect("valid arena");
    for territory in &manifest.territories {
        assert!(
            root.join(format!("instructions/{}.md", territory.id))
                .is_file()
        );
        assert!(
            root.join(format!("controller/oracle-{}.sh", territory.id))
                .is_file()
        );
        assert!(root.join(format!("nix/{}.nix", territory.id)).is_file());
    }
}

#[test]
fn guest_definitions_do_not_reference_controller_assets() {
    let root = arena_root();
    for entry in fs::read_dir(root.join("nix")).expect("nix directory") {
        let path = entry.expect("entry").path();
        let source = fs::read_to_string(&path).expect("nix source");
        assert!(
            !source.contains("controller/"),
            "{} leaks controller assets",
            path.display()
        );
        assert!(
            !source.contains("oracle-"),
            "{} leaks oracle names",
            path.display()
        );
    }
}

#[test]
fn archivist_provisions_after_postgresql_setup() {
    let archivist =
        std::fs::read_to_string(arena_root().join("nix/archivist.nix")).expect("archivist module");
    assert!(archivist.contains("postgresql-setup.service"));
    assert!(archivist.contains("after = [ \"postgresql.service\" \"postgresql-setup.service\" ]"));
    assert!(
        archivist.contains("requires = [ \"postgresql.service\" \"postgresql-setup.service\" ]")
    );
}
