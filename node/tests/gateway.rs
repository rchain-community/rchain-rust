//! Node-level gateway test: one `rnode` that is a member of two shards.
//!
//! Exercises the deferred "Layer 2" item end to end through the node's own HTTP surface: both
//! shards reach genesis under their own shard ids, the membership is reported, a deploy for a
//! non-member shard is refused, and a cross-shard transaction is coordinated by the node itself
//! (`POST /api/v1/txn`) rather than by an off-chain client.

mod common;

use std::time::Duration;

use serde_json::{json, Value};

use common::{free_ports, gateway_conf, start, temp_dir, VALIDATOR_PRIV_HEX};

/// Poll an endpoint until it returns 200 with a JSON body.
async fn poll_json(client: &reqwest::Client, url: &str, what: &str) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() {
                if let Ok(value) = resp.json::<Value>().await {
                    return value;
                }
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what} never became available at {url}"
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Two shards in one node: both reach genesis under their **own** full shard id, and the membership
/// is reported with the primary first.
#[test]
fn gateway_node_runs_two_shards_with_their_own_genesis() {
    common::test_runtime().block_on(async {
        let dir = temp_dir("gateway-two-shards");
        let ports = free_ports(4);
        let http_port = ports[0];
        let conf = gateway_conf(&dir, &ports, VALIDATOR_PRIV_HEX);
        assert_eq!(conf.casper.shards.len(), 2, "two memberships");
        let node = start(&conf, ports[2], http_port).await;
        let base = format!("http://127.0.0.1:{http_port}");
        let client = reqwest::Client::new();

        let shards = poll_json(&client, &format!("{base}/api/v1/shards"), "shards").await;
        assert_eq!(shards["shardCount"], 2);
        assert_eq!(shards["primaryShard"], "/root");
        assert_eq!(shards["shards"][0]["shardId"], "/root");
        assert_eq!(shards["shards"][0]["primary"], true);
        assert_eq!(shards["shards"][1]["shardId"], "/root/child");
        assert_eq!(shards["shards"][1]["primary"], false);
        // Each shard has its own chain (genesis at least), which is the regression for the genesis
        // shard-id fix: a genesis stamped with the bare name would match no membership.
        for shard in shards["shards"].as_array().unwrap() {
            assert!(
                shard["latestBlockNumber"].as_i64().is_some(),
                "each shard must report a height: {shard}"
            );
        }
        // `/api/status` still reports the primary shard's id (unchanged for existing tooling).
        let status = poll_json(&client, &format!("{base}/api/status"), "status").await;
        assert_eq!(status["shardId"], "/root");

        node.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    });
}

/// The node coordinates a cross-shard transaction itself: `POST /api/v1/txn` escrows on both of its
/// own shards and commits, and the durable record is readable back by id.
#[test]
fn gateway_node_coordinates_a_two_shard_transaction() {
    common::test_runtime().block_on(async {
        let dir = temp_dir("gateway-txn");
        let ports = free_ports(4);
        let http_port = ports[0];
        let conf = gateway_conf(&dir, &ports, VALIDATOR_PRIV_HEX);
        let node = start(&conf, ports[2], http_port).await;
        let base = format!("http://127.0.0.1:{http_port}");
        let client = reqwest::Client::new();

        // Wait for both shards to be up before transacting.
        poll_json(&client, &format!("{base}/api/v1/shards"), "shards").await;

        let txn_id = "aabbccdd";
        let body = json!({
            "txnId": txn_id,
            "legs": [
                { "shardId": "/root",       "amount": 30, "to": "gatewayDest" },
                { "shardId": "/root/child", "amount": 40, "to": "gatewayDest" }
            ]
        });
        let resp = client
            .post(format!("{base}/api/v1/txn"))
            .json(&body)
            .send()
            .await
            .expect("POST /api/v1/txn");
        let status = resp.status();
        let record: Value = resp.json().await.expect("record json");
        assert_eq!(status, 200, "record: {record}");
        assert_eq!(record["state"], "committed", "record: {record}");
        assert_eq!(record["txnId"], txn_id);
        assert_eq!(record["legs"].as_array().unwrap().len(), 2);
        let votes = record["votes"].as_array().unwrap();
        assert_eq!(votes.len(), 2);
        assert!(votes.iter().all(|v| v["vote"] == "ready"), "{votes:?}");
        assert!(
            record["recordHash"].as_str().is_some_and(|h| !h.is_empty()),
            "the record is content-addressed: {record}"
        );

        // The same record comes back by id, and a re-run is idempotent (no second escrow).
        let fetched = poll_json(
            &client,
            &format!("{base}/api/v1/txn/{txn_id}"),
            "txn record",
        )
        .await;
        assert_eq!(fetched, record);
        let replay = client
            .post(format!("{base}/api/v1/txn"))
            .json(&body)
            .send()
            .await
            .expect("re-run");
        let replayed: Value = replay.json().await.unwrap();
        assert_eq!(replayed, record, "a re-run returns the same record");

        // Nothing is left in flight.
        let in_flight = poll_json(&client, &format!("{base}/api/v1/txn"), "in-flight list").await;
        assert_eq!(in_flight["inFlight"].as_array().unwrap().len(), 0);

        node.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    });
}

/// A transaction naming a shard this node does not validate is refused, with the membership.
#[test]
fn gateway_node_refuses_a_non_member_shard() {
    common::test_runtime().block_on(async {
        let dir = temp_dir("gateway-non-member");
        let ports = free_ports(4);
        let http_port = ports[0];
        let conf = gateway_conf(&dir, &ports, VALIDATOR_PRIV_HEX);
        let node = start(&conf, ports[2], http_port).await;
        let base = format!("http://127.0.0.1:{http_port}");
        let client = reqwest::Client::new();
        poll_json(&client, &format!("{base}/api/v1/shards"), "shards").await;

        let resp = client
            .post(format!("{base}/api/v1/txn"))
            .json(&json!({
                "txnId": "deadbeef",
                "legs": [ { "shardId": "/elsewhere", "amount": 1, "to": "dest" } ]
            }))
            .send()
            .await
            .expect("POST /api/v1/txn");
        assert_eq!(resp.status(), 400);
        let err: Value = resp.json().await.unwrap();
        let err = err.as_str().unwrap_or_default();
        assert!(err.contains("/elsewhere"), "{err}");
        assert!(err.contains("/root, /root/child"), "{err}");

        // A malformed transaction id is a bad request, not a panic.
        let resp = client
            .post(format!("{base}/api/v1/txn"))
            .json(&json!({ "txnId": "zz", "legs": [ { "shardId": "/root", "amount": 1, "to": "d" } ] }))
            .send()
            .await
            .expect("POST /api/v1/txn");
        assert_eq!(resp.status(), 400);

        node.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    });
}
