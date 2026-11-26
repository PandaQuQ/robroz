//! Embeds the short git commit for `robroz-host --version`.
//!
//! `ROBROZ_GIT_SHA` wins (the Docker build sets it, because `.git` is not in
//! the build context). Otherwise `git rev-parse` is asked. Without either the
//! sha is `unknown`; the build never fails over it.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn short(sha: &str) -> Option<String> {
    let sha = sha.trim();
    let ok = !sha.is_empty() && sha.chars().all(|c| c.is_ascii_hexdigit());
    ok.then(|| sha.chars().take(7).collect())
}

fn main() {
    println!("cargo:rerun-if-env-changed=ROBROZ_GIT_SHA");
    // Rebuild when the checked-out commit moves.
    for path in ["HEAD", "refs", "packed-refs"] {
        if let Some(p) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    if let Some(head) = git(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(p) = git(&["rev-parse", "--git-path", &head]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    let sha = std::env::var("ROBROZ_GIT_SHA")
        .ok()
        .and_then(|s| short(&s))
        .or_else(|| git(&["rev-parse", "--short=7", "HEAD"]).and_then(|s| short(&s)))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=ROBROZ_GIT_SHA_SHORT={sha}");
}
