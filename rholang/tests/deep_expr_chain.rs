//! A **parsed** deep expression chain is evaluated without recursing (AUDIT C101).
//!
//! `1 + 1 + … + 1` is a single parser nesting level with a spine as long as the chain — which is why
//! the parser's own guards do not bound it (`MAX_PARSE_DEPTH` bounds nesting, `MAX_CHAIN_LENGTH`
//! bounds the links, and this term is inside both) — while the evaluator spent one Rust frame per
//! link. Measured on the node's own 32 MiB worker stack (`node/src/main.rs`), in a debug build: a
//! chain as a send's datum evaluated at 320 links and **aborted the process** at 350
//! (`thread 'tokio-rt-worker' has overflowed its stack`, SIGABRT) inside `eval_single_expr`'s
//! recursion over the expression. C100's value guard cannot see it: the abort happens while the datum
//! is being *built*, before `produce` is reached.
//!
//! So this test's failure mode before the fix was a SIGABRT, which is what makes a passing run
//! unambiguous. The value is checked too — the contract below fails the deploy (a division by zero)
//! unless the chain really evaluated to the number it should, so "no error" cannot mean "evaluated to
//! something else and did not crash".

use std::io::Write as _;

mod common;

/// The node's worker stack (`node/src/main.rs`).
const NODE_STACK: usize = 32 * 1024 * 1024;

/// A contract that computes `1 + 1 + … + 1` (`links` of them) as a send's datum, receives it, and
/// divides by zero unless it is exactly `links + 1`. The receive makes the value observable through
/// `evaluate`'s error list, which is the only channel this harness has.
fn chain_program(links: usize) -> String {
    let expected = links + 1;
    format!(
        r#"new a in {{
             a!(1{chain}) |
             for (@x <- a) {{ if (x == {expected}) {{ Nil }} else {{ 1 / 0 }} }}
           }}"#,
        chain = " + 1".repeat(links),
    )
}

async fn evaluate(links: usize) -> Result<(), String> {
    let rt = common::build_runtime(true).await;
    let rand = rchain_crypto::hash::blake2b512_random::Blake2b512Random::from_init(&[0u8; 32]);
    match rt.evaluate(&chain_program(links), &rand).await {
        Ok(result) => {
            if result.errors.is_empty() {
                Ok(())
            } else {
                Err(format!("{:?}", result.errors[0]))
            }
        }
        Err(e) => Err(format!("{e:?}")),
    }
}

#[test]
fn a_deep_parsed_chain_evaluates_without_overflowing_the_stack() {
    std::thread::Builder::new()
        .stack_size(NODE_STACK)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .thread_stack_size(NODE_STACK)
                .enable_all()
                .build()
                .expect("a tokio runtime")
                .block_on(async {
                    // The control: a shallow chain is an ordinary program, and it must still be one.
                    let small = evaluate(10).await;
                    println!("chain 10 -> {small:?}");
                    let _ = std::io::stdout().flush();
                    assert!(
                        small.is_ok(),
                        "a 10-link chain is a normal program: {small:?}"
                    );

                    // 350 is the depth that aborted the process before the fix, and 500 is near the
                    // chain guard's own ceiling (`MAX_CHAIN_LENGTH = 512`), so the spine is exercised
                    // across its whole legal range.
                    for links in [350usize, 500] {
                        let deep = evaluate(links).await;
                        println!("chain {links} -> {deep:?}");
                        let _ = std::io::stdout().flush();
                        assert!(
                            deep.is_ok(),
                            "a {links}-link chain aborted the process before this fix, so an error \
                             here is either the old abort or a wrong value: {deep:?}"
                        );
                    }
                })
        })
        .expect("spawn")
        .join()
        .expect("the evaluator must not overflow the stack — that is the whole point");
}
