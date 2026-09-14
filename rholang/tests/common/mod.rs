//! Shared harness for the rholang execution-pipeline integration tests.

// Each integration-test target uses a different subset of these helpers.
#![allow(dead_code)]

use std::sync::Arc;

use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
use rchain_models::sorted::SortedProc;
use rchain_rholang::runtime::{ReplayRhoRuntime, RhoRuntime};
use rchain_rholang::scheduler::EffectMode;
use rchain_rholang::storage::RhoMatch;
use rchain_rspace::factory::create_history_repository;
use rchain_rspace::hot_store::InMemHotStore;
use rchain_rspace::rspace::RSpace;
use rchain_shared::store_manager::InMemoryStoreManager;

/// Assemble a play + replay runtime pair over a fresh in-memory store.
pub async fn build_runtime_pair() -> (RhoRuntime, ReplayRhoRuntime) {
    let manager = InMemoryStoreManager::default();
    let history = create_history_repository::<
        SortedProc,
        BindPattern,
        ListParWithRandom,
        TaggedContinuation,
    >(&manager, "rspace")
    .await
    .expect("history repository");
    let reader = history.get_history_reader(history.root()).await;
    let hot = Arc::new(InMemHotStore::new(reader.base()));
    let (play, replay) = RSpace::create_with_replay(history.clone(), hot, Arc::new(RhoMatch));
    let rho = RhoRuntime::create(play, history.clone(), SortedProc::default())
        .await
        .expect("rho runtime");
    let replay = ReplayRhoRuntime::create(Arc::new(replay), history, SortedProc::default())
        .await
        .expect("replay runtime");
    (rho, replay)
}

/// Assemble a play runtime over a fresh in-memory store, with per-term concurrency toggled (used by
/// the concurrent-vs-sequential differential test). The effect scheduler is the sequential
/// reference.
pub async fn build_runtime(concurrent: bool) -> RhoRuntime {
    build_runtime_with_mode(concurrent, EffectMode::Sequential).await
}

/// Assemble a play runtime with both per-term concurrency and the effect-scheduler mode chosen
/// (used by the gate/relaxed scheduler differential tests).
pub async fn build_runtime_with_mode(concurrent: bool, mode: EffectMode) -> RhoRuntime {
    let manager = InMemoryStoreManager::default();
    let history = create_history_repository::<
        SortedProc,
        BindPattern,
        ListParWithRandom,
        TaggedContinuation,
    >(&manager, "rspace")
    .await
    .expect("history repository");
    let reader = history.get_history_reader(history.root()).await;
    let hot = Arc::new(InMemHotStore::new(reader.base()));
    let (play, _replay) = RSpace::create_with_replay(history.clone(), hot, Arc::new(RhoMatch));
    RhoRuntime::create_with_effect_mode(play, history, SortedProc::default(), concurrent, mode)
        .await
        .expect("rho runtime")
}

/// The rows of a golden file as `(id, value, provenance)`.
///
/// `#` lines are the file's legend and are skipped; the value is read **by position**, so the
/// provenance column (and any later one) cannot corrupt a vector. `execution.tsv`'s provenance is
/// `rust-regression-pinned` — there is no Scala oracle for the rholang pipeline — so a caller that
/// needs ground truth must check the third field rather than assume it.
pub fn golden_rows(target: &str) -> Vec<(String, String, String)> {
    let path = format!(
        "{}/testdata/differential/{target}.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut f = l.split('\t');
            (
                f.next().unwrap_or_default().to_string(),
                f.next().unwrap_or_default().to_string(),
                f.next().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// Look up a committed golden hex vector for `case` in `testdata/differential/<target>.tsv`.
pub fn load_golden(case: &str, target: &str) -> Option<String> {
    golden_rows(target)
        .into_iter()
        .find(|(id, _, _)| id == case)
        .map(|(_, value, _)| value)
}
