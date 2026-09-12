//! The effect scheduler: DFS-path addressing and the scheduler modes of Laws 20–22
//! (`docs/src/formal/channel-scheduler.md`, `spec/Rchain/Scheduler.lean`).
//!
//! Every effect in a reduction carries a [`DfsPath`] — the sequence of child indices from the root
//! of the reduction tree (`[4]` is the 4th sibling of a `Par`; `[4, 0]` its first child). The
//! lexicographic order on paths is exactly the sequential reducer's depth-first order
//! (`[4] < [4, 0] < [5]`), which makes it the linearization key for Laws 20–22:
//!
//! * **Law 20** — a per-channel claim queue keeps same-channel operations in DFS path order
//!   (Phase 2/5: `rspace::concurrent::channel_queue`).
//! * **Law 21** — the gate scheduler runs the op at path `p` only after every op at a smaller
//!   path completes (Phase 4); the depth-2 counterexample (`one_hop_depth2_diverges`) shows the
//!   one-hop "next-step footprint" variant is unsound.
//! * **Law 22** — the matched datum is concrete at dispatch, so a continuation's first-step
//!   footprint is computable then — not before.

/// A depth-first-search path: the sequence of child indices from the root of the reduction tree.
///
/// Deriving `Ord` gives the lexicographic (dictionary) order the scheduler linearizes by: a
/// parent precedes its children, and `[4, 0]` precedes `[4, 1]` and `[5]`. Paths grow by one
/// segment per nested `Par`; a `u16` segment suffices because a `Par` holds at most `i16::MAX`
/// terms (`reduce::resolve_children`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DfsPath(pub Vec<u16>);

impl DfsPath {
    /// The root path `[]`, carried by the top-level evaluation.
    pub fn root() -> Self {
        DfsPath(Vec::new())
    }

    /// The path of the `i`th child: this path extended with the child's index.
    pub fn child(&self, i: u16) -> Self {
        let mut path = self.0.clone();
        path.push(i);
        DfsPath(path)
    }
}

/// The effect-scheduler mode (Laws 20–22). The reducer's *default* is [`EffectMode::Sequential`]:
/// the plain DFS loop, which is the sound reference every other mode must refine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EffectMode {
    /// The sequential DFS loop — the reference reducer (no scheduler concurrency).
    #[default]
    Sequential,
    /// Level-1 fork-join only: pure term resolution is concurrent (`reduce::resolve_children`);
    /// effect application stays sequential and observably identical to `Sequential`.
    ForkJoin,
    /// Law 21: deterministic gate mode — the task for effect `i` runs only after the tasks for
    /// effects `0..i−1` have completed. Sequential-equivalent; not a speedup, the sound carrier.
    Gate,
    /// Law 20: relaxed mode — per-channel DFS op order is preserved by the claim queue,
    /// cross-channel interleaving is free. Off-chain (exploratory) use only: the relaxed
    /// interleaving must never reach a block's event log (Law 11 replay / Law 16 content
    /// addressing).
    Relaxed,
    /// Laws 23–25 (on-chain validated speculation): the relaxed scheduler on the block path,
    /// where every block-path run is checked against the sequential DFS reference (state hash +
    /// per-channel COMM subsequences) and falls back to the sequential trace on divergence.
    /// `docs/src/formal/onchain-scheduling.md`.
    RelaxedValidated,
}

impl std::str::FromStr for EffectMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dfs" => Ok(EffectMode::Sequential),
            "gate" => Ok(EffectMode::Gate),
            "relaxed" => Ok(EffectMode::Relaxed),
            "relaxed-validated" => Ok(EffectMode::RelaxedValidated),
            other => Err(format!(
                "'{other}': expected one of dfs, gate, relaxed, relaxed-validated"
            )),
        }
    }
}
