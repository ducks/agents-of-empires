use std::fs;
use std::path::{Component, Path, PathBuf};

use aoe_domain::{ArenaManifest, ManifestError, MatchMode};
use serde::Serialize;
use thiserror::Error;

const TEMPLATE_ID: &str = "hello-service";
const TEMPLATE_FILES: &[(&str, &str, bool)] = &[
    (
        "arena.toml",
        include_str!("../../../examples/hello-service-arena/arena.toml"),
        false,
    ),
    (
        "flake.nix",
        include_str!("../../../examples/hello-service-arena/flake.nix"),
        false,
    ),
    (
        "flake.lock",
        include_str!("../../../examples/hello-service-arena/flake.lock"),
        false,
    ),
    (
        "CONTRACT.md",
        include_str!("../../../examples/hello-service-arena/CONTRACT.md"),
        false,
    ),
    (
        "README.md",
        include_str!("../../../examples/hello-service-arena/README.md"),
        false,
    ),
    (
        "nix/base.nix",
        include_str!("../../../examples/hello-service-arena/nix/base.nix"),
        false,
    ),
    (
        "instructions/builder-one.md",
        include_str!("../../../examples/hello-service-arena/instructions/builder-one.md"),
        false,
    ),
    (
        "instructions/builder-two.md",
        include_str!("../../../examples/hello-service-arena/instructions/builder-two.md"),
        false,
    ),
    (
        "instructions/builder-three.md",
        include_str!("../../../examples/hello-service-arena/instructions/builder-three.md"),
        false,
    ),
    (
        "verify/service-up.sh",
        include_str!("../../../examples/hello-service-arena/verify/service-up.sh"),
        true,
    ),
    (
        "verify/service-restart.sh",
        include_str!("../../../examples/hello-service-arena/verify/service-restart.sh"),
        true,
    ),
    (
        "verify/host-reboot.sh",
        include_str!("../../../examples/hello-service-arena/verify/host-reboot.sh"),
        true,
    ),
    (
        "adapters/oracle.sh",
        include_str!("../../../examples/hello-service-arena/adapters/oracle.sh"),
        true,
    ),
    (
        "scripts/prepare-credentials.sh",
        include_str!("../../../examples/hello-service-arena/scripts/prepare-credentials.sh"),
        true,
    ),
    (
        "tests/smoke.sh",
        include_str!("../../../examples/hello-service-arena/tests/smoke.sh"),
        true,
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArenaPackageReport {
    pub valid: bool,
    pub arena: Option<String>,
    pub schema_version: Option<u32>,
    pub root: PathBuf,
    pub manifest: PathBuf,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum ArenaPackageError {
    #[error("arena name must contain only lowercase ASCII letters, digits, and hyphens")]
    InvalidName,
    #[error("output directory is not empty: {0}")]
    OutputNotEmpty(String),
    #[error("arena package I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Create a self-contained starter arena package.
///
/// # Errors
///
/// Returns an error for an invalid name, non-empty destination, or file-system failure.
pub fn init_arena(name: &str, output: &Path) -> Result<PathBuf, ArenaPackageError> {
    if !valid_id(name) {
        return Err(ArenaPackageError::InvalidName);
    }
    if output.exists() && fs::read_dir(output)?.next().is_some() {
        return Err(ArenaPackageError::OutputNotEmpty(
            output.display().to_string(),
        ));
    }
    fs::create_dir_all(output)?;
    for (relative, template, executable) in TEMPLATE_FILES {
        let destination = output.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, template.replace(TEMPLATE_ID, name))?;
        set_executable(&destination, *executable)?;
    }
    Ok(output.join("arena.toml"))
}

/// Validate a portable arena package and all controller-visible references.
///
/// A directory input resolves to `arena.toml`; a file input is used directly.
/// Schema problems and missing package files are returned together in the report.
///
/// # Errors
///
/// Returns an error only when the manifest cannot be read for reasons other than absence.
pub fn validate_arena_package(path: &Path) -> Result<ArenaPackageReport, ArenaPackageError> {
    let manifest_path = if path.is_dir() {
        path.join("arena.toml")
    } else {
        path.to_owned()
    };
    let root = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_owned();
    let mut report = ArenaPackageReport {
        valid: false,
        arena: None,
        schema_version: None,
        root: root.clone(),
        manifest: manifest_path.clone(),
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    if !manifest_path.is_file() {
        report
            .errors
            .push(format!("missing {}", manifest_path.display()));
        return Ok(report);
    }

    let source = fs::read_to_string(&manifest_path)?;
    let manifest = match ArenaManifest::parse(&source) {
        Ok(manifest) => manifest,
        Err(ManifestError::Validation(errors)) => {
            report
                .errors
                .extend(errors.into_iter().map(|error| error.to_string()));
            return Ok(report);
        }
        Err(error) => {
            report.errors.push(error.to_string());
            return Ok(report);
        }
    };
    report.arena = Some(manifest.arena.id.clone());
    report.schema_version = Some(manifest.schema_version);

    require_file(&root, "flake.nix", &mut report.errors);
    if manifest.arena.mode == MatchMode::BuildRace {
        require_file(&root, "CONTRACT.md", &mut report.errors);
    }
    for territory in &manifest.territories {
        require_file(
            &root,
            &format!("instructions/{}.md", territory.id),
            &mut report.errors,
        );
        validate_flake_reference(
            &root,
            &territory.nixos_config,
            &territory.id,
            &mut report.errors,
        );
    }
    if let Some(build) = &manifest.build {
        for milestone in &build.milestones {
            validate_package_path(
                &root,
                &milestone.verifier,
                &format!("verifier for milestone {}", milestone.id),
                true,
                &mut report.errors,
            );
        }
    }

    recommend_file(&root, "README.md", &mut report.warnings);
    recommend_file(&root, "adapters/oracle.sh", &mut report.warnings);
    recommend_file(&root, "tests/smoke.sh", &mut report.warnings);
    if !root.join("flake.lock").is_file() {
        report
            .warnings
            .push("flake.lock is absent; pin dependencies before publishing the arena".into());
    }
    report.valid = report.errors.is_empty();
    Ok(report)
}

fn validate_flake_reference(
    root: &Path,
    reference: &str,
    territory: &str,
    errors: &mut Vec<String>,
) {
    let local = reference
        .split_once('#')
        .map_or(reference, |(path, _)| path);
    if local.is_empty() {
        errors.push(format!("territory {territory} has an empty Nix flake path"));
        return;
    }
    validate_package_path(
        root,
        local,
        &format!("Nix flake for territory {territory}"),
        false,
        errors,
    );
}

fn validate_package_path(
    root: &Path,
    relative: &str,
    label: &str,
    executable: bool,
    errors: &mut Vec<String>,
) {
    let path = Path::new(relative);
    if path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
        errors.push(format!(
            "{label} must stay inside the arena package: {relative}"
        ));
        return;
    }
    let candidate = root.join(path);
    if !candidate.exists() {
        errors.push(format!("{label} does not exist: {}", candidate.display()));
    } else if executable && !is_executable(&candidate) {
        errors.push(format!(
            "{label} is not executable: {}",
            candidate.display()
        ));
    }
}

fn require_file(root: &Path, relative: &str, errors: &mut Vec<String>) {
    if !root.join(relative).is_file() {
        errors.push(format!("required package file is missing: {relative}"));
    }
}

fn recommend_file(root: &Path, relative: &str, warnings: &mut Vec<String>) {
    if !root.join(relative).is_file() {
        warnings.push(format!("recommended package file is missing: {relative}"));
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    if executable {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
