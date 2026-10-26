// Stamps the fork commit into take-manifest.json (design D19).
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    for path in ["HEAD", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(head) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--git-path", &head])
    {
        println!("cargo:rerun-if-changed={path}");
    }
    let commit = std::env::var("GITHUB_SHA")
        .ok()
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| {
            println!("cargo:warning=No git commit found; take manifests will say 0000…");
            "0".repeat(40)
        });
    println!("cargo:rustc-env=CAP_FORK_COMMIT={}", commit.to_lowercase());
}
