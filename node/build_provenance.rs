// What commit a binary reports, as a policy rather than as a build-script side effect.
//
// `node/build.rs` is the one file in this workspace that cannot be tested — a build script's own
// `#[cfg(test)]` never runs under `cargo test`. So the decision lives here, as a pure function, and
// is `include!`d by **two** things: `build.rs`, which acts on it, and
// `node/tests/build_provenance.rs`, which tests it. This file is deliberately not a module of the
// library: neither it nor its tests have any business shipping in the binary.
//
// **These are outer comments, not `//!` docs, and that is forced by `include!`** — an inner doc
// comment is only legal before the first item of a file or module, and this source is spliced in
// *after* `build.rs`'s own `use`. A `//!` here is a compile error there, not a style choice.
//
// The rule it encodes is the one the wargame's W-N7-01 names. `GIT_HEAD_COMMIT` used to win outright
// and return early, so anyone building modified source could set it to the canonical hash and
// `/version` would name a commit the binary was not built from — a provenance string that lies is
// worse than one that admits ignorance. But the variable exists for a real reason: the Docker
// context excludes `.git/`, so `git rev-parse` cannot answer there and the image served
// `commit # unknown`. Both are true, and the resolution is precedence by *capability*: the variable
// is honoured **only when git cannot answer**, and when git can answer and the two disagree, git
// wins and the disagreement is reported loudly — because the build argument is then stale, which is
// itself a fact the operator needs.

/// Where the reported commit came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// `git rev-parse HEAD` answered.
    Git(String),
    /// The build script was given `GIT_HEAD_COMMIT` and git could not answer (the container build).
    Provided(String),
    /// Neither could answer; the version string says `commit # unknown`.
    Unknown,
}

impl Provenance {
    /// The commit to embed, or `None` for [`Provenance::Unknown`].
    pub fn commit(&self) -> Option<&str> {
        match self {
            Provenance::Git(commit) | Provenance::Provided(commit) => Some(commit),
            Provenance::Unknown => None,
        }
    }
}

/// Decide the commit to report, and whether the build argument was overridden.
///
/// The second element is `true` only when both sources answered and they **disagree** — the one case
/// that is a defect rather than a configuration, and the one the build script warns about.
pub fn resolve(git: Option<&str>, provided: Option<&str>) -> (Provenance, bool) {
    let git = nonempty(git);
    let provided = nonempty(provided);
    match (git, provided) {
        // Git answered: it is the tree that was actually compiled, so it wins — and a provided value
        // that contradicts it is reported rather than silently preferred.
        (Some(git), Some(provided)) => (Provenance::Git(git.to_string()), git != provided),
        (Some(git), None) => (Provenance::Git(git.to_string()), false),
        // Git could not answer — the Docker context excludes `.git/` — so the build argument is the
        // only source there is, and honouring it is what keeps the image from reporting `unknown`.
        (None, Some(provided)) => (Provenance::Provided(provided.to_string()), false),
        (None, None) => (Provenance::Unknown, false),
    }
}

/// A blank or whitespace-only value is no value: `ARG GIT_HEAD_COMMIT=""` in the Dockerfile puts a
/// set-but-empty variable in the environment, and treating that as a commit would embed `""`.
fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The container's path: git cannot answer (no `.git/` in the build context), so the build
    /// argument is used. Without this the image regresses to `commit # unknown`, which is the whole
    /// reason the argument exists.
    #[test]
    fn the_provided_commit_is_used_when_git_cannot_answer() {
        let (provenance, disagreed) = resolve(None, Some("bbbb"));
        assert_eq!(provenance, Provenance::Provided("bbbb".to_string()));
        assert!(!disagreed, "there is nothing to disagree with");
        assert_eq!(provenance.commit(), Some("bbbb"));
    }

    /// **A provided commit that contradicts git loses, and says so** (W-N7-01). This is the defect the
    /// row names: the variable used to win outright, so a build of modified source reporting the
    /// canonical hash was indistinguishable from a clean build of that hash. Remove the precedence —
    /// return `Provided` when there is one — and this goes red.
    #[test]
    fn an_explicit_commit_that_disagrees_loses_to_git_and_is_flagged() {
        let (provenance, disagreed) = resolve(Some("aaaa"), Some("bbbb"));
        assert_eq!(
            provenance,
            Provenance::Git("aaaa".to_string()),
            "git resolved the commit the tree was compiled from; the argument is a claim about it"
        );
        assert!(
            disagreed,
            "and the disagreement is the thing the operator has to be told"
        );
    }

    /// The ordinary local build: git answers, nothing was provided, nothing to report.
    #[test]
    fn git_alone_is_used_and_is_not_flagged() {
        let (provenance, disagreed) = resolve(Some("aaaa"), None);
        assert_eq!(provenance, Provenance::Git("aaaa".to_string()));
        assert!(!disagreed);
    }

    /// Agreement is not a disagreement: `tools/devnet.sh build` passes the very hash git would have
    /// resolved, and warning on that would be noise that trains an operator to ignore the warning.
    #[test]
    fn an_explicit_commit_that_agrees_is_not_flagged() {
        let (provenance, disagreed) = resolve(Some("aaaa"), Some("aaaa"));
        assert_eq!(provenance, Provenance::Git("aaaa".to_string()));
        assert!(!disagreed);
    }

    /// `ARG GIT_HEAD_COMMIT=""` is the shipped Dockerfile's default, so the empty string has to mean
    /// "not provided" — otherwise a plain `docker build` embeds `""` and `/version` reads
    /// `RChain Node 0.1.0 ()` rather than `commit # unknown`. Also: surrounding whitespace, which any
    /// `$(git rev-parse HEAD)` piped through a shell can carry.
    #[test]
    fn a_blank_or_whitespace_value_is_no_value() {
        assert_eq!(resolve(None, Some("")), (Provenance::Unknown, false));
        assert_eq!(resolve(None, Some("   ")), (Provenance::Unknown, false));
        assert_eq!(resolve(Some(""), Some("aaaa")), (Provenance::Provided("aaaa".to_string()), false));
        assert_eq!(
            resolve(Some(" aaaa "), None),
            (Provenance::Git("aaaa".to_string()), false)
        );
    }

    /// Neither source: the build script warns and the version string says so. `Unknown` is a real
    /// answer, not an error — a source tarball with no `git` must still build.
    #[test]
    fn neither_source_is_unknown_rather_than_a_failure() {
        let (provenance, disagreed) = resolve(None, None);
        assert_eq!(provenance, Provenance::Unknown);
        assert!(!disagreed);
        assert_eq!(provenance.commit(), None);
    }
}
