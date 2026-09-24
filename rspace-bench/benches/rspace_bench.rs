//! Criterion benchmarks for the RSpace / rholang reduce path (port of `legacy/rspace-bench`).
//!
//! Ports `EvalBench` (the `mvcepp.rho` reduce path), `WideBench` (`wide-setup.rho` + `wide.rho`),
//! `KeyBench` (`Blake2b256Hash` key encoding), and raw `RSpaceBench`/`ReplayRSpaceBench`
//! (produce/consume on the play/replay spaces). The Scala `AddressBookExample` toy type has no Rust
//! equivalent; the raw benches use the real `Par`/`BindPattern`/`ListParWithRandom`/
//! `TaggedContinuation` types instead.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use rchain_block_storage::dag::finalizer::Message;
use rchain_block_storage::dag::message_state::DagMessageState;
use rchain_block_storage::dag::metadata_store::{add_block_to_dag_state_mut, BlockInfo, DagState};
use rchain_block_storage::dag::representation::DagRepresentation;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::ast::{Expr, Par, Var};
use rchain_models::block_hash::BlockHash;
use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
use rchain_models::sorted::SortedProc;
use rchain_models::validator::Validator;
use rchain_rholang::runtime::{ReplayRhoRuntime, RhoRuntime};
use rchain_rholang::scheduler::EffectMode;
use rchain_rholang::storage::RhoMatch;
use rchain_rspace::factory::create_history_repository;
use rchain_rspace::hot_store::InMemHotStore;
use rchain_rspace::rspace::RSpace;
use rchain_rspace::tuple_space::Tuplespace;
use rchain_shared::refined::{BlockHeight, SeqNum};
use rchain_shared::store_manager::InMemoryStoreManager;

const MVCEPP: &str = include_str!("resources/mvcepp.rho");
const WIDE: &str = include_str!("resources/wide.rho");
const WIDE_SETUP: &str = include_str!("resources/wide-setup.rho");

/// Assemble a play + replay runtime pair over a fresh in-memory store (mirror of
/// `rholang/tests/common/mod.rs::build_runtime_pair`).
async fn build_runtime_pair() -> (RhoRuntime, ReplayRhoRuntime) {
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

fn eval_bench(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (runtime, _replay) = rt.block_on(build_runtime_pair());
    let rand = Blake2b512Random::default_random();

    c.bench_function("eval/mvcepp", |b| {
        b.iter(|| {
            rt.block_on(async {
                runtime
                    .evaluate(MVCEPP, &rand)
                    .await
                    .expect("mvcepp reduce");
            })
        })
    });
}

fn wide_bench(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let rand = Blake2b512Random::default_random();

    c.bench_function("eval/wide", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (runtime, _replay) = build_runtime_pair().await;
                runtime
                    .evaluate(WIDE_SETUP, &rand)
                    .await
                    .expect("wide setup");
                runtime.evaluate(WIDE, &rand).await.expect("wide reduce");
            })
        })
    });
}

fn key_bench(c: &mut Criterion) {
    let hashes: Vec<Blake2b256Hash> = (0..1001)
        .map(|i| Blake2b256Hash::create(&[i as u8; 32]))
        .collect();

    c.bench_function("key/prepare_codec", |b| {
        b.iter(|| {
            for h in &hashes {
                let _ = h.to_byte_array();
            }
        })
    });

    c.bench_function("key/prepare_raw", |b| {
        b.iter(|| {
            for h in &hashes {
                let _ = h.as_bytes();
            }
        })
    });
}

fn channel(s: &str) -> SortedProc {
    SortedProc::new(Par {
        exprs: vec![Expr::GString(s.to_string())],
        ..Default::default()
    })
}

fn wildcard() -> Par {
    Par {
        exprs: vec![Expr::EVar(Box::new(Var::Wildcard))],
        ..Default::default()
    }
}

fn datum() -> ListParWithRandom {
    ListParWithRandom {
        pars: vec![SortedProc::default()],
        random_state: Blake2b512Random::default_random(),
    }
}

fn pattern() -> BindPattern {
    BindPattern {
        patterns: vec![SortedProc::new(wildcard())],
        remainder: None,
        free_count: 0,
    }
}

fn rspace_bench(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (runtime, _replay) = rt.block_on(build_runtime_pair());
    let space = runtime.space().clone();
    let ch = channel("friends#1");

    c.bench_function("rspace/consume", |b| {
        b.iter(|| {
            rt.block_on(async {
                space
                    .consume(
                        &[ch.clone()],
                        &[pattern()],
                        TaggedContinuation::Empty,
                        true,
                        BTreeSet::new(),
                    )
                    .await
                    .expect("consume");
            })
        })
    });

    c.bench_function("rspace/produce", |b| {
        b.iter(|| {
            rt.block_on(async {
                space
                    .produce(ch.clone(), datum(), false)
                    .await
                    .expect("produce");
            })
        })
    });
}

fn replay_rspace_bench(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (_runtime, replay) = rt.block_on(build_runtime_pair());
    let space = replay.space().clone();
    let ch = channel("consume");

    c.bench_function("replay/consume", |b| {
        b.iter(|| {
            rt.block_on(async {
                space
                    .consume(
                        &[ch.clone()],
                        &[pattern()],
                        TaggedContinuation::Empty,
                        true,
                        BTreeSet::new(),
                    )
                    .await
                    .expect("replay consume");
            })
        })
    });

    c.bench_function("replay/produce", |b| {
        b.iter(|| {
            rt.block_on(async {
                space
                    .produce(ch.clone(), datum(), false)
                    .await
                    .expect("replay produce");
            })
        })
    });
}

// ---- Channel-scheduler benches (Laws 20–22) ----

/// Assemble a play runtime under the given scheduler mode (mirror of
/// `rholang/tests/common/mod.rs::build_runtime_with_mode`). Fresh per iteration so every
/// measurement runs against a clean space.
async fn build_runtime_with_mode(concurrent: bool, mode: EffectMode) -> RhoRuntime {
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

/// A play `RSpace` over a hot store with the given shard count.
async fn build_play_space_with_shards(
    shards: usize,
) -> Arc<RSpace<SortedProc, BindPattern, ListParWithRandom, TaggedContinuation>> {
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
    let hot = Arc::new(InMemHotStore::with_shards(reader.base(), shards));
    let (play, _replay) = RSpace::create_with_replay(history.clone(), hot, Arc::new(RhoMatch));
    play
}

/// The scheduler-mode benchmark label ("dfs" = the sequential reference reducer).
fn mode_label(mode: EffectMode) -> &'static str {
    match mode {
        EffectMode::Sequential | EffectMode::ForkJoin => "dfs",
        EffectMode::Gate => "gate",
        EffectMode::Relaxed => "relaxed",
        EffectMode::RelaxedValidated => "relaxed-validated",
    }
}

/// `new c in { c!(0) | contract c(@x) = { match x { N => { @"done"!(x) } _ => { c!(x + 1) } } } }`
/// — an n-round pingpong chain on a single channel. Every round's produce and the contract's
/// consume hit the same channel, so relaxed mode serializes the whole chain through the claim
/// queue (the scheduler's serialization cost), and the rounds are inherently sequential work.
fn pingpong_term(n: usize) -> String {
    format!(
        r#"new c in {{ c!(0) | contract c(@x) = {{ match x {{ {n} => {{ @"done"!(x) }} _ => {{ c!(x + 1) }} }} }} }}"#
    )
}

/// `new c0, .., cN in { for (@x <- c0) { @["out", "c0"]!(x) } | .. | c0!(1) | .. | cN!(1) }` — N
/// fully independent channels, one produce + one consume each (the relaxed scheduler's parallel
/// case: no two effects share a channel).
fn fanout_term(n: usize) -> String {
    let names: Vec<String> = (0..n).map(|i| format!("c{i}")).collect();
    let consumes = names
        .iter()
        .map(|c| format!(r#"for (@x <- {c}) {{ @["out", "{c}"]!(x) }}"#))
        .collect::<Vec<_>>()
        .join(" | ");
    let produces = names
        .iter()
        .map(|c| format!("{c}!(1)"))
        .collect::<Vec<_>>()
        .join(" | ");
    format!("new {} in {{ {consumes} | {produces} }}", names.join(", "))
}

/// One scheduler benchmark fn: `term` evaluated on a fresh runtime per iteration, under `mode`,
/// on a tokio runtime with `workers` worker threads.
fn sched_eval(c: &mut Criterion, name: &str, term: &str, mode: EffectMode, workers: usize) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let term = term.to_string();
    let rand = Blake2b512Random::default_random();
    c.bench_function(
        &format!("sched/{name}/{}/workers{workers}", mode_label(mode)),
        |b| {
            b.iter(|| {
                rt.block_on(async {
                    let runtime = build_runtime_with_mode(true, mode).await;
                    runtime.evaluate(&term, &rand).await.expect("reduce");
                })
            })
        },
    );
}

/// The machine's hardware parallelism, falling back to 1 (the tokio default).
fn num_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// The striped vs unstriped hot-store comparison (SHARDS=64 vs the pre-stripe single mutex):
/// 128 concurrent produces on 128 distinct channels spread over all shards, so the unstriped
/// store serializes every access through one mutex while the striped store runs them in parallel.
fn store_contention(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    for (label, shards) in [("striped", 64usize), ("unstriped", 1)] {
        let space = rt.block_on(build_play_space_with_shards(shards));
        let channels: Vec<SortedProc> = (0..128).map(|i| channel(&format!("s{i}"))).collect();
        c.bench_function(&format!("sched/store_contention/{label}"), |b| {
            b.iter(|| {
                rt.block_on(async {
                    let mut tasks = Vec::with_capacity(channels.len());
                    for ch in &channels {
                        let space = space.clone();
                        let ch = ch.clone();
                        tasks.push(tokio::spawn(async move {
                            space.produce(ch, datum(), false).await.expect("produce");
                        }));
                    }
                    for task in tasks {
                        task.await.expect("produce task");
                    }
                })
            })
        });
    }
}

fn sched_bench(c: &mut Criterion) {
    // Per-workload per-mode at the machine's default worker count: pingpong (single-channel
    // serialization) and fanout (cross-channel parallelism) across sizes.
    for n in [2usize, 4, 8, 16] {
        let term = pingpong_term(n);
        // `relaxed-validated` (Laws 23–25) dispatches as relaxed at the rholang level — the
        // validation oracle lives on the casper block path — so its rows pin dispatch parity
        // with the pure relaxed arm.
        for mode in [
            EffectMode::Sequential,
            EffectMode::Gate,
            EffectMode::Relaxed,
            EffectMode::RelaxedValidated,
        ] {
            sched_eval(c, &format!("pingpong/{n}"), &term, mode, num_workers());
        }
    }
    for n in [8usize, 32, 128] {
        let term = fanout_term(n);
        for mode in [
            EffectMode::Sequential,
            EffectMode::Gate,
            EffectMode::Relaxed,
            EffectMode::RelaxedValidated,
        ] {
            sched_eval(c, &format!("fanout/{n}"), &term, mode, num_workers());
        }
    }

    // Mode sweep at workers 1–8 on the fanout/32 workload: the scheduler's scaling behaviour.
    let sweep = fanout_term(32);
    for workers in 1..=8usize {
        for mode in [
            EffectMode::Sequential,
            EffectMode::Gate,
            EffectMode::Relaxed,
            EffectMode::RelaxedValidated,
        ] {
            sched_eval(c, "sweep/fanout32", &sweep, mode, workers);
        }
    }

    store_contention(c);
}

/// A chain of `n` messages — block `i` justifies block `i - 1` — built the way the storage's
/// restore folds a stored chain: `create_message` (which runs the finalizer) then the in-place
/// insert. Built as a `DagRepresentation` so the pass's own accounting can be read off it.
fn chain_representation(n: usize) -> DagRepresentation {
    let mut messages = DagMessageState::<BlockHash, Validator>::empty();
    let mut dag_state = DagState::empty();
    let mut previous: Option<Message<BlockHash, Validator>> = None;
    for i in 0..n {
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&(i as u64).to_le_bytes());
        let id = BlockHash::new(bytes);
        let justifications: BTreeSet<Message<BlockHash, Validator>> =
            previous.iter().cloned().collect();
        let height = BlockHeight::try_from(i as i64).expect("a bench chain's height");
        let msg = messages.create_message(
            id,
            height,
            Validator::new([0u8; 65]),
            SeqNum::try_from(i as i64).expect("a bench chain's seq"),
            BTreeMap::new(),
            &justifications,
        );
        add_block_to_dag_state_mut(
            &BlockInfo {
                hash: id,
                parents: msg.parents.clone(),
                block_num: height,
                validation_failed: false,
            },
            &mut dag_state,
        );
        messages.insert_msg_mut(&msg);
        previous = Some(msg);
    }
    DagRepresentation {
        dag_set: dag_state.dag_set,
        child_map: dag_state.child_map,
        height_map: dag_state.height_map,
        dag_message_state: messages,
        fringe_states: BTreeMap::new(),
    }
}

/// The `dag` group: the two curves the pass's constants question needs — N against time, and N
/// against the accounting the DAG publishes.
///
/// - `fold_chain/N` is the restore shape of AUDIT C55 (`create` folds every stored block through
///   `insert_msg_mut`), at sizes small enough to read as a curve.
/// - `accounting/N` is the cost of the gauges Stage 6 publishes per insert: the sums are of `len()`s,
///   so this is the Θ(N) pass over the message map, not over the sets.
/// - The `logical_bytes`/`seen_entries` values are printed once per N: they are the numbers H6's
///   resident floor is stated in (Σ|seen| × 32 B is the Θ(N²) term), and the reason the bench exists
///   is to settle those constants if a later pass reopens the floor.
fn dag_bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("dag");
    group.sample_size(10);
    for n in [250usize, 500, 1000] {
        group.bench_with_input(BenchmarkId::new("fold_chain", n), &n, |b, &n| {
            b.iter(|| chain_representation(n).dag_message_state.msg_map.len());
        });
        let representation = chain_representation(n);
        println!(
            "dag curve: N={n} messages={} seen_entries={} index_entries={} logical_bytes={}",
            representation.message_count(),
            representation.seen_entries(),
            representation.index_entries(),
            representation.logical_bytes()
        );
        group.bench_with_input(BenchmarkId::new("accounting", n), &n, |b, &n| {
            let representation = chain_representation(n);
            b.iter(|| {
                (
                    representation.message_count(),
                    representation.seen_entries(),
                    representation.index_entries(),
                    representation.logical_bytes(),
                )
            });
        });
    }
    group.finish();
}

criterion_group!(eval, eval_bench);
criterion_group!(wide, wide_bench);
criterion_group!(key, key_bench);
criterion_group!(rspace, rspace_bench);
criterion_group!(replay, replay_rspace_bench);
criterion_group!(sched, sched_bench);
criterion_group!(dag, dag_bench);
criterion_main!(eval, wide, key, rspace, replay, sched, dag);
