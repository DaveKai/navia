// Stamps the build's commit for Settings > About and adds the Windows icon resource. Build scripts run on the host, so
// the target comes from Cargo's environment, not cfg!().
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    stamp_commit();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=assets/windows/app.rc");
    println!("cargo:rerun-if-changed=assets/windows/app.ico");
    // app.rc plus the version resource Explorer shows under Properties > Details. It's written here because the version
    // comes from Cargo.
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("assets/windows");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let mut nums: Vec<u32> = version.split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect();
    nums.resize(4, 0);
    let commas = nums.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let license = std::env::var("CARGO_PKG_LICENSE").unwrap_or_default();
    let base = std::fs::read_to_string(dir.join("app.rc")).expect("app.rc").replace("\"app.ico\"", &format!("{:?}", dir.join("app.ico").display().to_string()));
    let rc = format!(
        "{base}\n1 VERSIONINFO\nFILEVERSION {commas}\nPRODUCTVERSION {commas}\nBEGIN\n  BLOCK \"StringFileInfo\"\n  BEGIN\n    BLOCK \"040904B0\"\n    BEGIN\n      VALUE \"CompanyName\", \"Navia\"\n      VALUE \"FileDescription\", \"Navia\"\n      VALUE \"FileVersion\", \"{version}\"\n      VALUE \"InternalName\", \"Navia\"\n      VALUE \"LegalCopyright\", \"Licensed under {license}\"\n      VALUE \"OriginalFilename\", \"Navia.exe\"\n      VALUE \"ProductName\", \"Navia\"\n      VALUE \"ProductVersion\", \"{version}\"\n    END\n  END\n  BLOCK \"VarFileInfo\"\n  BEGIN\n    VALUE \"Translation\", 0x409, 1200\n  END\nEND\n"
    );
    let path = out.join("navia.rc");
    std::fs::write(&path, rc).expect("writing the resource script");
    embed_resource::compile(&path, embed_resource::NONE)
        .manifest_optional()
        .expect("compiling the Windows resources");
}

/// Sets `NAVIA_COMMIT` and `NAVIA_BUILD`, the short commit and commit count a test build shows after the version. Both
/// are empty for a release, which is HEAD tagged v<version>. Set them in the environment to override, for example when
/// building from an archive without git.
fn stamp_commit() {
    println!("cargo:rerun-if-env-changed=NAVIA_COMMIT");
    println!("cargo:rerun-if-env-changed=NAVIA_BUILD");
    let commit = std::env::var("NAVIA_COMMIT").unwrap_or_else(|_| {
        let git = |args: &[&str]| Command::new("git").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
        if git(&["describe", "--exact-match", "--tags", "--match", &format!("v{version}"), "HEAD"]).is_some() {
            return String::new();
        }
        // Rebuild when HEAD moves. A worktree's HEAD lives in its own git dir.
        if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
            println!("cargo:rerun-if-changed={head}");
        }
        if let Some(refs) = git(&["rev-parse", "--symbolic-full-name", "HEAD"]).filter(|r| r.starts_with("refs/")).and_then(|r| git(&["rev-parse", "--git-path", &r])) {
            println!("cargo:rerun-if-changed={refs}");
        }
        git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_default()
    });
    println!("cargo:rustc-env=NAVIA_COMMIT={commit}");
    let build = std::env::var("NAVIA_BUILD").unwrap_or_else(|_| {
        if commit.is_empty() {
            return String::new();
        }
        Command::new("git").args(["rev-list", "--count", "HEAD"]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    });
    println!("cargo:rustc-env=NAVIA_BUILD={build}");
}
