// @feature installation
// @spec docs/features/installation.md
use std::path::Path;
use std::process::Command;

fn git(root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(["-c", "core.fsmonitor=false"])
        .arg("-C")
        .arg(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(arguments)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn watch_source(root: &Path) {
    for path in [
        "src",
        "skills",
        "migrations",
        "pi-extension/dist",
        "Cargo.toml",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    for key in ["NEXUS_RELEASE_REVISION", "TARGET"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    for name in ["HEAD", "index"] {
        if let Some(path) = git(root, &["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
        }
    }
    if let Some(branch) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = git(root, &["rev-parse", "--git-path", &branch]) {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
        }
    }
}

fn main() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    watch_source(&root);
    let source = git(&root, &["rev-parse", "HEAD"]);
    let release = std::env::var("NEXUS_RELEASE_REVISION")
        .ok()
        .filter(|value| !value.is_empty());
    let dirty = match &release {
        Some(release) => {
            assert!(
                release.len() == 40
                    && release
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
                "release identity requires a full lowercase Git revision"
            );
            if let Some(source) = &source {
                assert_eq!(
                    release, source,
                    "release revision must match the checked-out source"
                );
                assert!(
                    git(
                        &root,
                        &["status", "--porcelain", "--untracked-files=normal"]
                    )
                    .is_some_and(|state| state.is_empty()),
                    "release builds require a clean source checkout"
                );
            }
            false
        }
        None => git(
            &root,
            &["status", "--porcelain", "--untracked-files=normal"],
        )
        .is_none_or(|state| !state.is_empty()),
    };
    let revision = release.or(source).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=NEXUS_BUILD_REVISION={revision}");
    println!("cargo:rustc-env=NEXUS_BUILD_DIRTY={dirty}");
    let target = std::env::var("TARGET").unwrap();
    let package = std::env::var("CARGO_PKG_VERSION").unwrap();
    let suffix = if dirty { "; dirty" } else { "" };
    println!("cargo:rustc-env=NEXUS_BUILD_TARGET={target}");
    println!("cargo:rustc-env=NEXUS_BUILD_VERSION={package} ({revision}; {target}{suffix})");
}
