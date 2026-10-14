//! **A dev-only drill helper** — perturb one block's per-block claim, so a *running* node diverges from
//! its peers for a reason that survives the restore rule's revalidation.
//!
//! Why this exists, and why it is not a product seam: the self-reset's trigger needs a
//! `FailureCause::Divergence`, which the code defines as *"this node's state or replay disagrees"*. A
//! **record** edit cannot stage one — `restore_divergent_justifications` re-runs `validate_checks` and
//! *clears* a merely-marked block (`casper/src/multi_parent_casper.rs:1178`, §109) — and a **network
//! partition** stages nothing either, because the merge is a pure function of the parent set (measured:
//! `spec/audit/evidence/reset-arm/e1-partition.md`). What is left is an edit of the **state the node
//! computes from**: the per-block claim the merge reads as its base.
//!
//! The merge's reader takes the greatest `(height, block_hash)` claim among a fringe's members
//! (`multi_parent_casper.rs:302`), so the claim that decides the base is the **newest** block's — and
//! that is the one this tool perturbs. Every peer holds the true claim; the tampered node reads its own
//! and computes a different pre-state for its peers' blocks.
//!
//! It runs against a **stopped** node's data directory (LMDB takes a single writer), reuses the tree's
//! own codec so it cannot drift from the on-disk shape, and is an *example* rather than an `rnode`
//! subcommand: a store-rewriting subcommand on the shipped binary is a product seam the type-system gate
//! exists to catch.
//!
//! Usage: `tamper-fringe-claim <data-dir>`

use std::path::PathBuf;
use std::sync::Arc;

use rchain_block_storage::dag::codecs::{BlockHashCodec, BlockMetadataCodec};
use rchain_models::block::state_hash::StateHash;
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_shared::store_manager::database;
use rchain_shared::typed_store::KeyValueTypedStore;

#[tokio::main]
async fn main() -> Result<(), String> {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .ok_or_else(|| "usage: tamper-fringe-claim <data-dir>".to_string())?
        .into();

    let store_manager = rchain_casper::storage::rnode_key_value_store_manager(&dir);
    let kv: Arc<dyn KeyValueTypedStore<BlockHash, BlockMetadata>> = Arc::new(
        database(
            &store_manager,
            "block-metadata",
            Arc::new(BlockHashCodec),
            Arc::new(BlockMetadataCodec),
        )
        .await
        .map_err(|e| e.to_string())?,
    );

    let all = kv.to_map().await.map_err(|e| e.to_string())?;
    let top = all
        .values()
        .map(|m| m.block_num)
        .max()
        .ok_or_else(|| "the metadata store holds no blocks".to_string())?;

    // **Every block at the store's greatest height**, not the store's own newest by some tie-break. The
    // merge selects the greatest `(height, hash)` justification (`newest_justification`, and the claim
    // reader at `multi_parent_casper.rs:302`), and a validator's justifications are the *net's* tips: the
    // greatest-height one may be another node's block, which this store also holds. Tampering only "this
    // node's own tip" therefore leaves the selected block untouched — measured, and why the first run of
    // this drill staged nothing. Tampering *all* the max-height blocks makes the selection tampered
    // whichever block it lands on.
    let targets: Vec<(BlockHash, BlockMetadata)> = all
        .into_iter()
        .filter(|(_, m)| m.block_num == top)
        .collect();

    // **Two fields, because the merge reads one or the other depending on the fringe.** While nothing has
    // finalised the base is the newest justification's `post_state_hash` (`:334-338`, the
    // `prev_fringe.is_empty()` branch, which a fresh devnet is in); once a fringe exists it is the claim
    // (`:302`). Writing both makes the node's base wrong whichever branch it takes.
    let block_store = rchain_block_storage::block_store::create(&store_manager)
        .await
        .map_err(|e| e.to_string())?;

    let mut claim_rows = Vec::new();
    let mut body_rows = Vec::new();
    for (hash, meta) in &targets {
        let mut claim = *meta.fringe_state_hash.as_bytes();
        claim[0] ^= 0xff;
        claim_rows.push((
            *hash,
            BlockMetadata {
                fringe_state_hash: StateHash::new(claim),
                ..meta.clone()
            },
        ));
        if let Some(mut body) = block_store
            .get(&[*hash])
            .await
            .map_err(|e| e.to_string())?
            .pop()
            .flatten()
        {
            let mut post = *body.post_state_hash.as_bytes();
            post[0] ^= 0xff;
            body.post_state_hash = StateHash::new(post);
            body_rows.push((*hash, body));
        }
    }
    kv.put(&claim_rows).await.map_err(|e| e.to_string())?;
    if !body_rows.is_empty() {
        block_store
            .put(&body_rows)
            .await
            .map_err(|e| e.to_string())?;
    }

    println!(
        "tampered {} block(s) at height {} ({} with bodies): the greatest-height justification this \
         store holds now claims a state its peers do not share",
        targets.len(),
        i64::from(top),
        body_rows.len()
    );
    Ok(())
}
