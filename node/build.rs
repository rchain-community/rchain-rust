//! Build provenance — the commit this binary was compiled from.
//!
//! The Scala node got this from sbt-buildinfo's generated `BuildInfo.gitHeadCommit`; there is no
//! equivalent for a Cargo build without a build script, so this is the port's substitute. It resolves
//! `git rev-parse HEAD` and hands the result to the compiler as `GIT_HEAD_COMMIT`, which
//! [`crate::web::version_info`] reads with `option_env!` and joins into the version string that the
//! HTTP routes (`/version`, `/status`) and the gRPC `Status` all serve.
//!
//! **A build that cannot resolve a commit must still build.** No `git` on `PATH`, a source tarball, a
//! shallow clone cut in a way `rev-parse` refuses — all of those leave the variable unset, the version
//! string falls back to `commit # unknown`, and the build succeeds with a warning saying why. What
//! must *not* happen is a build script that fails the build over a provenance nicety.
//!
//! The `rerun-if-changed` lines are the part that is easy to get wrong. Cargo re-runs a build script
//! when a watched file changes, and a commit does **not** touch `HEAD` on a branch: `HEAD` holds a
//! *symbolic* ref (`ref: refs/heads/dev`) and it is `refs/heads/dev` that moves. So both are watched,
//! plus `packed-refs`, for a ref that has been packed by `git gc`. Without that, a build after a
//! commit would keep serving the previous commit — which is worse than serving none, because it is
//! wrong rather than unknown.

use std::process::Command;

/// Run a git command, returning its trimmed stdout when it succeeds and is non-empty.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn main() {
    // **An environment variable the build script reads is not a tracked input unless it says so.**
    // Without this line cargo keeps the previous `cargo:rustc-env` and never re-runs the script when
    // the variable changes — measured: `GIT_HEAD_COMMIT=deadbeef… cargo build` produced a binary
    // still reporting the earlier git-resolved commit. The container build hides it (a fresh build
    // has no cached script output), so it would have shipped as a local-only trap.
    println!("cargo:rerun-if-env-changed=GIT_HEAD_COMMIT");

    if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        if let Some(reference) = git(&["symbolic-ref", "--quiet", "HEAD"]) {
            println!("cargo:rerun-if-changed={git_dir}/{reference}");
        }
        println!("cargo:rerun-if-changed={git_dir}/packed-refs");
    }

    // **An explicitly-provided commit wins, and it is how the container image gets one.** The
    // Dockerfile builds from a context that excludes `.git` (`.dockerignore`), so `git rev-parse`
    // below cannot answer there and the image used to serve `commit # unknown` — the exact gap this
    // build script exists to close, reintroduced by the packaging. `tools/devnet.sh build` passes the
    // revision as a build argument; taking it here rather than only as an ambient variable makes the
    // precedence explicit, and `cargo:rustc-env` is what carries it to `option_env!`.
    if let Ok(provided) = std::env::var("GIT_HEAD_COMMIT") {
        let provided = provided.trim();
        if !provided.is_empty() {
            println!("cargo:rustc-env=GIT_HEAD_COMMIT={provided}");
            return;
        }
    }

    match git(&["rev-parse", "HEAD"]) {
        Some(commit) => println!("cargo:rustc-env=GIT_HEAD_COMMIT={commit}"),
        None => println!(
            "cargo:warning=could not resolve the git commit for this build; \
             `/version` will report `commit # unknown`"
        ),
    }
}
