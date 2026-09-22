//! **Where an exploratory deploy's reply is read from** (AUDIT C38).
//!
//! `POST /api/v1/explore-deploy` runs a term and returns what it produced as JSON. Until this was
//! made explicit the reply was read from exactly one channel — the name derived from the deploy's
//! RNG, which is the term's **first `new`-bound name** — and a term that replied anywhere else got
//! an empty `expr`, indistinguishable from a term that produced nothing. `casper/tests/scheduler.rs`
//! had an assertion that documented the drop as expected: a term sending `42` on `@"chan"` was
//! asserted to return nothing.
//!
//! The rule is now stated (`spec/API-SCHEMA.md`) and the reply says **which** channel answered, so
//! "your reply is somewhere we do not read" is a fact a client can see rather than silence. These
//! tests pin the three outcomes by name.

mod common;

use common::build_runtime_manager;
use rchain_casper::runtime_manager::ReplySource;
use rchain_models::block::state_hash::StateHash;

async fn start_hash(rm: &rchain_casper::runtime_manager::RuntimeManager) -> StateHash {
    StateHash::from_slice(rm.get_history_repo().root().as_bytes())
}

/// `@"out"` — the channel the corpus, the examples and the system-process conformance tests use, and
/// the one a client reaches for first. **This is the case that was silently dropped.**
#[tokio::test]
async fn a_reply_on_the_out_channel_comes_back() {
    let rm = build_runtime_manager().await;
    let start = start_hash(&rm).await;
    let reply = rm
        .play_exploratory_deploy(r#"@"out"!(42)"#, &start)
        .await
        .expect("the exploratory deploy runs");
    assert_eq!(
        reply.source,
        ReplySource::Out,
        "a reply on `@\"out\"` must be read from `@\"out\"` — reading only the derived channel \
         returned an empty reply for every term written this way (AUDIT C38)"
    );
    assert_eq!(reply.data.len(), 1, "the datum `42` is the reply");
}

/// The first `new`-bound name — the derived channel, and the only one that used to be read.
#[tokio::test]
async fn a_reply_on_the_first_private_name_comes_back() {
    let rm = build_runtime_manager().await;
    let start = start_hash(&rm).await;
    let reply = rm
        .play_exploratory_deploy(r#"new result in { result!(7) }"#, &start)
        .await
        .expect("the exploratory deploy runs");
    assert_eq!(
        reply.source,
        ReplySource::FirstPrivateName,
        "the derived channel must keep working: a client that follows the convention sees no change"
    );
    assert_eq!(reply.data.len(), 1);
}

/// A term that really produced nothing on either channel says **so**, instead of the empty list that
/// used to mean three different things.
#[tokio::test]
async fn a_term_that_produces_nothing_says_which_rule_was_tried() {
    let rm = build_runtime_manager().await;
    let start = start_hash(&rm).await;
    let reply = rm
        .play_exploratory_deploy(r#"@"chan"!(42)"#, &start)
        .await
        .expect("the exploratory deploy runs");
    assert_eq!(
        reply.source,
        ReplySource::None,
        "a reply on some other channel is neither of the two the node reads, and the response now \
         says so rather than leaving an empty list to be guessed at"
    );
    assert!(reply.data.is_empty());
}

/// The rule's names are part of the client contract, so they are asserted literally.
#[tokio::test]
async fn the_reply_source_names_are_stable() {
    assert_eq!(ReplySource::FirstPrivateName.as_str(), "firstPrivateName");
    assert_eq!(ReplySource::Out.as_str(), "out");
    assert_eq!(ReplySource::None.as_str(), "none");
}
