//! The value route into RSpace is depth-bounded (AUDIT C100, law 50 applied to values).
//!
//! The parser bound (`MAX_AST_DEPTH`) covers a *parsed term*. A rholang **program** can build a value
//! the parser never saw: the contract below folds its accumulator into a deeper tuple each iteration,
//! so the value's depth grows with the iteration count while the reduce-step cost grows with it too —
//! `O(n)` steps for depth `n`, inside `DEFAULT_MAX_REDUCE_STEPS` (100_000) and inside the explore
//! path's own cap (10_000). Every consumer of that value then recurses once per level.
//!
//! **Measured before the guard, on the node's own 32 MiB worker stack**: a fold to depth 101 costs
//! 7.3 s of CPU in a debug build, and a fold to depth 401 **aborts the process**
//! (`thread 'tokio-rt-worker' has overflowed its stack` → SIGABRT) inside `eval_single_expr`'s
//! recursion over the value. So this test's failure mode was a SIGABRT, which is why the assertion is
//! unambiguous: a passing run here cannot be a passing run of the old code.
//!
//! The three cases are a control pair plus the old abort: under the bound must still evaluate, over it
//! must be *refused* — an error, not a silent empty result, because a produce that quietly does
//! nothing is a different bug (the space would answer as if the datum had been stored).

use std::io::Write as _;

mod common;

/// The node's worker stack (`node/src/main.rs`).
const NODE_STACK: usize = 32 * 1024 * 1024;

/// A contract that calls itself `n` times, wrapping its accumulator in a 2-tuple each time and
/// producing it: the stored value's depth is `n + 1`.
///
/// The spelling follows the node's own contracts (`casper/src/genesis/resources/rgov/Chat.rho`): bare
/// name patterns, `*` to evaluate a name into the data.
fn fold_program(n: usize) -> String {
    format!(
        r#"new loop, done in {{
             contract loop(acc, k) = {{
               if (*k == 0) {{ done!(*acc) }} else {{ loop!((*acc, 1), *k - 1) }}
             }} |
             loop!(0, {n})
           }}"#
    )
}

async fn evaluate_with(concurrent: bool, n: usize) -> Result<usize, String> {
    let rt = common::build_runtime(concurrent).await;
    let rand = rchain_crypto::hash::blake2b512_random::Blake2b512Random::from_init(&[0u8; 32]);
    match rt.evaluate(&fold_program(n), &rand).await {
        Ok(result) => {
            if result.errors.is_empty() {
                Ok(n)
            } else {
                Err(format!("{:?}", result.errors[0]))
            }
        }
        Err(e) => Err(format!("{e:?}")),
    }
}

/// The concurrent path (`build_runtime(true)`), which is what the node's block path uses.
async fn evaluate(n: usize) -> Result<usize, String> {
    evaluate_with(true, n).await
}

#[test]
fn a_value_built_at_runtime_is_refused_above_the_bound() {
    std::thread::Builder::new()
        .stack_size(NODE_STACK)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .thread_stack_size(NODE_STACK)
                .enable_all()
                .build()
                .expect("a tokio runtime")
                .block_on(async {
                    // A shallow fold is legitimate and must still work: the guard must refuse the
                    // depth, not the language feature.
                    let small = evaluate(20).await;
                    println!("fold 20 -> {small:?}");
                    let _ = std::io::stdout().flush();
                    assert!(small.is_ok(), "a 20-deep fold is a normal program: {small:?}");

                    // 401 is the depth that aborted the process before the guard. It now has to be an
                    // error — and specifically a *refusal*, not a silent success.
                    let deep = evaluate(400).await;
                    println!("fold 400 -> {deep:?}");
                    let _ = std::io::stdout().flush();
                    assert!(
                        deep.is_err(),
                        "a 400-deep runtime value reached the space; before the guard this aborted \
                         the process, so an Ok here is the guard not firing: {deep:?}"
                    );

                    // The refusal must name the reason: an empty error list would mean the produce
                    // silently did nothing, which is the failure this asserts against.
                    let msg = deep.unwrap_err();
                    assert!(
                        msg.contains("nested too deeply"),
                        "the refusal has to say what it refused: {msg}"
                    );
                })
        })
        .expect("spawn")
        .join()
        .expect("the probe thread must not abort — that is the whole point");
}

/// **The refusal is the reducer's, not the scheduler's.** With per-term concurrency off (the
/// sequential reference the concurrent path is differentially tested against), the same fold must be
/// refused with the same message: if only one of the two reported the error, the "refusal" would be a
/// concurrency artifact — a spawned task swallowing the produce error — rather than the guard.
///
/// The depth is 300 rather than the abort case's 400 because the point here is the *scheduler*, not
/// the old abort, and the fold's cost grows super-linearly with depth (the doc's own datum: 7.3 s at
/// depth 101 in a debug build). 300 is the shallowest comfortable margin over the 256 bound.
#[test]
fn the_refusal_is_the_same_without_per_term_concurrency() {
    std::thread::Builder::new()
        .stack_size(NODE_STACK)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .thread_stack_size(NODE_STACK)
                .enable_all()
                .build()
                .expect("a tokio runtime")
                .block_on(async {
                    let seq = evaluate_with(false, 300).await;
                    println!("sequential fold 300 -> {seq:?}");
                    let _ = std::io::stdout().flush();
                    let msg = seq.expect_err(
                        "the sequential reference must refuse a 300-deep runtime value too — \
                         otherwise the concurrent refusal is the spinner, not the guard",
                    );
                    assert!(
                        msg.contains("nested too deeply"),
                        "the sequential refusal has to name the reason: {msg}"
                    );
                })
        })
        .expect("spawn")
        .join()
        .expect("the probe thread must not abort");
}
