//! Shared harness for the casper consensus-pipeline integration tests.

use std::sync::Arc;

use rchain_casper::runtime_manager::{MergeableStore, RuntimeManager};
use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
use rchain_models::sorted::SortedProc;
use rchain_rholang::merging::DeployMergeableDataCodec;
use rchain_rholang::runtime::{ReplayRhoRuntime, RhoRuntime};
use rchain_rholang::scheduler::EffectMode;
use rchain_rholang::storage::RhoMatch;
use rchain_rspace::factory::create_history_repository;
use rchain_rspace::hot_store::InMemHotStore;
use rchain_rspace::rspace::RSpace;
use rchain_shared::store_manager::{database, InMemoryStoreManager};
use rchain_shared::typed_store::BytesCodec;

/// Assemble a full `RuntimeManager` (play + replay runtimes + mergeable store) over an in-memory
/// store, running the plain sequential scheduler.
pub async fn build_runtime_manager() -> RuntimeManager {
    build_runtime_manager_with_mode(EffectMode::Sequential).await
}

/// `build_runtime_manager` under the given effect-scheduler mode (Laws 20–22): the play runtime is
/// created via `create_with_effect_mode`, the replay runtime stays sequential (already enforced by
/// `ReplayRhoRuntime::create`), and the manager records the mode for its block-path hard-reject.
pub async fn build_runtime_manager_with_mode(mode: EffectMode) -> RuntimeManager {
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
    let rho = RhoRuntime::create_with_effect_mode(
        play,
        history.clone(),
        SortedProc::default(),
        true,
        mode,
    )
    .await
    .expect("rho runtime");
    let replay = ReplayRhoRuntime::create(Arc::new(replay), history.clone(), SortedProc::default())
        .await
        .expect("replay runtime");
    let mergeable: MergeableStore = Arc::new(
        database(
            &manager,
            "mergeable",
            Arc::new(BytesCodec),
            Arc::new(DeployMergeableDataCodec),
        )
        .await
        .expect("mergeable store"),
    );
    RuntimeManager::new(rho, replay, history, mergeable, mode)
}
