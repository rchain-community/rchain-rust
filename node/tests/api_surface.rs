//! The node's **read surface**, exercised route by route against a booted node.
//!
//! This is batch 1 of the thin-coverage pass (`spec/TEST-COVERAGE.md`'s Definition-of-done item 11):
//! the files it pins were the top of the emitted ranking, not a hand-picked list —
//! `node/src/web/http.rs` (229 missed lines), `casper/src/api/block_api_impl.rs` (119, 44% missed) and
//! `node/src/api/grpc/{deploy_grpc_service_v1,tonic}.rs` — and every one of them is reachable from a
//! real node in-process, which is what `common::start` already provides. So the tests here drive the
//! **node's own routes** and let the call chain behind them (route → `WebApiImpl` → `shard_routing` →
//! `BlockApi`) be the thing that runs, rather than calling the API impls directly and re-testing the
//! routing nowhere.
//!
//! **What the assertions are about.** Each route gets its success shape *and* its refusal, because a
//! route is only pinned when its failure arm is: `json_result` maps every `BlockApiException` to
//! **400 with the reason as the body**, so an unknown hash must be a *defined* 400 and never a 5xx —
//! that clause is the one that catches a handler that panics or unwraps instead of reporting. The
//! refusals are grouped below for that reason, and the reporting route's 404 is a documented
//! behaviour (M6: the flag is read *and* enforced) rather than an accident of this node's config.
//!
//! Not here, on purpose: the admin routes (`/api/v1/propose`, the faucet) mutating state, the faucet's
//! budget (pinned by `web_api_impl`'s own tests), and the deploy path end to end
//! (`node/tests/deploy_block.rs`). One node boot is shared by every assertion, which is why they are
//! one test: booting costs seconds and the routes are independent reads.

mod common;

use std::time::Duration;

use serde_json::Value;

use common::{free_ports, standalone_conf, start, temp_dir, VALIDATOR_PRIV_HEX};

/// Poll until the HTTP server answers `/version` (the server is up before genesis is).
async fn wait_for_server(client: &reqwest::Client, base: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(resp) = client.get(format!("{base}/version")).send().await {
            if resp.status().is_success() {
                return;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "HTTP server did not come up"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Poll until `/api/blocks` reports the genesis block, and return it.
async fn wait_for_genesis(client: &reqwest::Client, base: &str) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(resp) = client.get(format!("{base}/api/blocks")).send().await {
            if resp.status().is_success() {
                if let Ok(Value::Array(a)) = resp.json::<Value>().await {
                    if let Some(first) = a.into_iter().next() {
                        return first;
                    }
                }
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "genesis block never appeared"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[test]
fn the_read_routes_answer_and_their_refusals_are_defined() {
    common::test_runtime().block_on(async {
        let dir = temp_dir("api-surface");
        let ports = free_ports(4);
        let http_port = ports[0];
        let mut conf = standalone_conf(&dir, &ports, Some(VALIDATOR_PRIV_HEX));
        // Dev mode admits the exploratory routes the way a devnet does; the read routes below do not
        // depend on it, but a node that refuses to boot would fail this test for the wrong reason.
        conf.dev_mode = true;
        let node = start(&conf, ports[2], http_port).await;
        let base = format!("http://127.0.0.1:{http_port}");
        let client = reqwest::Client::new();

        wait_for_server(&client, &base).await;
        let genesis = wait_for_genesis(&client, &base).await;
        let genesis_hash = genesis["blockHash"]
            .as_str()
            .expect("a light block info carries its hash")
            .to_string();

        let get = |path: String| {
            let client = client.clone();
            async move { client.get(path).send().await.expect("GET") }
        };

        // --- the block routes -----------------------------------------------------------------
        //
        // `/api/blocks/:depth` and `/api/blocks/:start/:end` are two different queries over the same
        // store: the first walks back from the head, the second is a height range. Both answer a JSON
        // array of light block info, and the genesis is in both on a fresh chain.
        let by_depth = get(format!("{base}/api/blocks/1")).await;
        assert_eq!(by_depth.status(), 200, "GET /api/blocks/1");
        let depth_json: Value = by_depth.json().await.expect("blocks by depth json");
        assert!(
            depth_json.is_array(),
            "a depth query answers a block array: {depth_json}"
        );

        let by_heights = get(format!("{base}/api/blocks/0/1")).await;
        assert_eq!(by_heights.status(), 200, "GET /api/blocks/0/1");
        let heights_json: Value = by_heights.json().await.expect("blocks by heights json");
        let heights = heights_json.as_array().expect("a height range is an array");
        assert!(
            !heights.is_empty(),
            "heights 0..=1 must include the genesis: {heights_json}"
        );

        // `/api/block/:hash` by the hash the list just gave us: the round trip is the point, because
        // it is the only read that proves the hash in a light block info resolves to the full block.
        // The answer is the Scala `BlockInfo` — the block *and* its deploys — so the hash to compare
        // is the nested one. (Measured, not assumed: the first version of this assertion read
        // `blockHash` off the response and failed, which is how the nesting was found.)
        let block = get(format!("{base}/api/block/{genesis_hash}")).await;
        assert_eq!(block.status(), 200, "GET /api/block/<genesis>");
        let block_json: Value = block.json().await.expect("block json");
        assert_eq!(
            block_json["blockInfo"]["blockHash"].as_str(),
            Some(genesis_hash.as_str()),
            "the block returned must be the one asked for: {block_json}"
        );
        assert!(
            block_json["deploys"].is_array(),
            "and the block's deploys travel with it: {block_json}"
        );

        // `/api/last-finalized-block` on a node that has produced only its genesis: the DAG has **no
        // finalized fringe**, and `last_finalized_block_unsafe` refuses with its own reason rather
        // than inventing an answer (`block-storage/src/dag/representation.rs:68`, whose unit test
        // pins the empty-DAG case). So the refusal is the behaviour to assert — as a *reported* 400
        // naming the reason, and explicitly not a 200 carrying an arbitrary block. The success path
        // needs a node whose finalizer has settled a fringe (`casper/tests/finalization.rs` drives the
        // finalizer directly); recorded in the register's classification table rather than faked here.
        let finalized = get(format!("{base}/api/last-finalized-block")).await;
        assert_eq!(
            finalized.status(),
            400,
            "a chain with no finalized fringe refuses the read"
        );
        let finalized_json: Value = finalized.json().await.expect("last finalized json");
        assert_eq!(
            finalized_json.as_str(),
            Some("Finalized fringe is not available."),
            "and names the reason: {finalized_json}"
        );

        // `/api/is-finalized/:hash` — a JSON boolean, and on this node `false` for the genesis.
        // `finalized_blocks_set` is the union of `seen` over the *latest fringe*, which is empty until
        // the finalizer settles one, so `false` here is the same fact the refusal above reports, told
        // as a value. It is also positive evidence that `true` is reachable: `casper/tests/
        // finalization.rs` asserts the fringe advancing over the same representation.
        let is_fin = get(format!("{base}/api/is-finalized/{genesis_hash}")).await;
        assert_eq!(is_fin.status(), 200, "a well-formed hash is answered");
        let is_fin_json: Value = is_fin.json().await.expect("is-finalized json");
        assert_eq!(
            is_fin_json.as_bool(),
            Some(false),
            "the genesis is not in a fringe that does not exist yet: {is_fin_json}"
        );

        // `/api/deploys` — the pooled (not-yet-included) deploys, as the Scala `DeploysResponse`:
        // a *document* wrapping the list, and an empty pool is an empty list inside it, not a 4xx.
        let deploys = get(format!("{base}/api/deploys")).await;
        assert_eq!(deploys.status(), 200, "GET /api/deploys");
        let deploys_json: Value = deploys.json().await.expect("deploys json");
        assert_eq!(
            deploys_json["deploys"].as_array().map(Vec::len),
            Some(0),
            "a fresh node's pool is empty and says so: {deploys_json}"
        );

        // --- the two refusals that are refusals -------------------------------------------------
        //
        // `json_result` maps a `BlockApiException` to 400 with the reason as the body, so a read that
        // cannot be answered is *reported* — and the reason must name what could not be found, which
        // is the clause that separates a diagnostic from a shrug. (Measured before it was asserted:
        // both carry the Scala's own phrasing plus the hash.)
        let unknown_hash = "0".repeat(64);
        for (label, resp, reason) in [
            (
                "GET /api/block/<unknown>",
                get(format!("{base}/api/block/{unknown_hash}")).await,
                "Failure to find block with hash",
            ),
            (
                "GET /api/deploy/<unknown>",
                get(format!("{base}/api/deploy/{unknown_hash}")).await,
                "Couldn't find block containing deploy with id",
            ),
        ] {
            let status = resp.status();
            let body: Value = resp.json().await.expect("a refusal carries its reason");
            assert_eq!(status, 400, "{label} refuses as a reported 400: {body}");
            let reason_body = body.as_str().expect("the body is the reason string");
            assert!(
                reason_body.contains(reason) && reason_body.contains(&unknown_hash),
                "{label} names what could not be found, and the hash it looked for: {reason_body}"
            );
        }

        // `/api/v1/deploy-status/:signature` is the third case, and it is a **200, not a refusal**:
        // `DeployExecStatus` is an enum whose `NotProcessed(Unknown)` arm *is* the answer for a deploy
        // the node has never seen (the Scala's own model — the query describes a state machine, and
        // "not processed, status unknown" is one of its states). This is worth pinning precisely
        // because it looks like the silent-negative class this tree has been closing (AUDIT
        // C53/C63/C67) and is the opposite of it: the value carries its own uncertainty, so a client
        // can tell it apart from "processed and failed".
        let dep_status = get(format!("{base}/api/v1/deploy-status/{unknown_hash}")).await;
        assert_eq!(
            dep_status.status(),
            200,
            "an unknown deploy is a state, not an error"
        );
        let dep_json: Value = dep_status.json().await.expect("deploy status json");
        assert_eq!(
            dep_json,
            serde_json::json!({ "NotProcessed": { "status": "Unknown" } }),
            "the deploy-status document says which state the deploy is in: {dep_json}"
        );

        // `/api/transactions/:hash` is 404 — not 400 — because reporting is off by default and the
        // route is *absent* rather than failing (M6: the flag was read but not enforced for this
        // route). A node with reporting enabled is a different surface, pinned by
        // `node/tests/gateway.rs`'s reporting cases.
        let tx = get(format!("{base}/api/transactions/{unknown_hash}")).await;
        assert_eq!(
            tx.status(),
            404,
            "with reporting disabled the transaction route does not exist"
        );

        // A malformed `data-at-name` body is rejected by the extractor before any handler runs: a
        // 4xx, and (the clause that matters) not a panic in a handler that assumed a well-formed
        // request.
        let bad = client
            .post(format!("{base}/api/data-at-name"))
            .json(&serde_json::json!({ "not_the_field_the_request_needs": 1 }))
            .send()
            .await
            .expect("POST /api/data-at-name");
        assert!(
            bad.status().is_client_error(),
            "a malformed data-at-name request is refused by name: {}",
            bad.status()
        );

        node.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    });
}
