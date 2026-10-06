//! The node's OCapN listener and its bridge (issue #249): a foreign peer dials a running node.
//!
//! Two gates, because they fail independently. The first is the listener's own: a node configured
//! with `api-server.ocapn-listen` answers a CapTP handshake and serves its fixtures. The second is
//! the bridge: a delivery to a chain-backed capability becomes a signed deploy, and that deploy
//! lands in a block, and the value the deployed contract wrote to the deploy's reply channel comes
//! back as the CapTP promise's fulfilment — the balance it reads is the wallet's, less the phlo that
//! very deploy spent.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use rchain_casper::protocol::client::{GrpcProposeService, ProposeService};
use rchain_node::api::ocapn::{ERTP_SWISS, REV_VAULT_BALANCE_SWISS};
use rchain_ocapn::bootstrap::Bootstrap;
use rchain_ocapn::captp::{Deliver, Desc};
use rchain_ocapn::conn::{Identity, Session};
use rchain_ocapn::locator::PeerLocator;
use rchain_ocapn::netlayer::Netlayer;
use rchain_ocapn::syrup::Value;
use rchain_ocapn::tcp_testing_only::TcpTestingOnly;
use rchain_ocapn::unix::UnixNetlayer;

/// The echo fixture's swiss number, as the conformance suite spells it.
const ECHO_SWISS: &[u8] = b"IO58l1laTyhcrgDKbEzFOO32MDd6zE5w";

/// Wait until the node's OCapN port accepts a connection. `start` returns as soon as the program is
/// spawned, and the listeners bind after the store replay — the same gap `wait_for_genesis` covers
/// for the HTTP API. The probe connection is dropped at once; the listener treats the close as a
/// session that ended, which it is.
async fn wait_for_ocapn(port: u16) {
    for _ in 0..300 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("the OCapN listener on 127.0.0.1:{port} never came up");
}

/// Wait until the node is out of read-only mode, i.e. genesis has been created and announced. The
/// OCapN port binds at "replay complete — starting listeners", which is *before* that, so a propose
/// driven straight after `wait_for_ocapn` is answered `ReadOnlyMode`. `deploy_block.rs` waits the
/// same way, over the HTTP API.
async fn wait_for_genesis(base: &str) {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Ok(resp) = client.get(format!("{base}/api/blocks")).send().await {
            if resp.status().is_success() {
                if let Ok(serde_json::Value::Array(blocks)) = resp.json::<serde_json::Value>().await
                {
                    if !blocks.is_empty() {
                        return;
                    }
                }
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "genesis never appeared: {base}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn locator(port: u16) -> PeerLocator {
    PeerLocator {
        designator: "rnode".to_string(),
        transport: "tcp-testing-only".to_string(),
        hints: BTreeMap::from([
            ("host".to_string(), "127.0.0.1".to_string()),
            ("port".to_string(), port.to_string()),
        ]),
    }
}

/// A `unix` locator: the transport name and the socket path, and no host at all — which the dial
/// policy reads as "cannot be judged", the right answer for a peer the filesystem admitted.
fn unix_locator(path: &std::path::Path) -> PeerLocator {
    PeerLocator {
        designator: "rnode".to_string(),
        transport: "unix".to_string(),
        hints: BTreeMap::from([("path".to_string(), path.display().to_string())]),
    }
}

/// Wait until the node's `unix` OCapN socket accepts a connection. There is no port to poll — the
/// socket file is the bind, and a successful connect is the proof it is listening.
async fn wait_for_ocapn_unix(path: &std::path::Path) {
    for _ in 0..300 {
        if tokio::net::UnixStream::connect(path).await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!(
        "the OCapN unix listener at {} never came up",
        path.display()
    );
}

#[test]
fn a_peer_dials_the_node_and_fetches_a_fixture() {
    let dir = common::temp_dir("ocapn-listener");
    // [http, admin-http, grpc-internal, protocol, grpc-external, ocapn]
    let ports = common::free_ports(6);
    let mut conf = common::deploy_conf(&dir, &ports);
    conf.api_server.ocapn_listen = Some(format!("127.0.0.1:{}", ports[5]));

    common::test_runtime().block_on(async {
        let node = common::start(&conf, ports[2], ports[0]).await;
        wait_for_ocapn(ports[5]).await;

        // Dial the node the way any OCapN peer would: a fresh session key, our own empty bootstrap.
        let dialer = TcpTestingOnly::bind("127.0.0.1:0")
            .await
            .expect("bind the dialing side");
        let connection = dialer
            .new_outgoing_connection(&locator(ports[5]))
            .await
            .expect("dial the node");
        let identity = Identity::fresh(locator(0)).expect("a session key");
        let mut client = Session::dial(connection, &identity, Arc::new(Bootstrap::default()))
            .await
            .expect("the node should complete the handshake");

        // Fetch the echo fixture from the node's bootstrap (its export 0), asking for the reply at
        // our own export 0 — the shape the conformance suite's `fetch_object` uses.
        let fetch = Deliver {
            to: Desc::Export(0u64.into()),
            args: vec![
                Value::Symbol("fetch".into()),
                Value::Bytes(ECHO_SWISS.to_vec()),
            ],
            answer_pos: None,
            resolve_me_desc: Some(Desc::ImportObject(0u64.into())),
        };
        client
            .send_message(&fetch.to_syrup())
            .await
            .expect("send the fetch");

        let reply = client
            .recv_message()
            .await
            .expect("read the reply")
            .expect("a reply, not a closed connection");
        let delivered = Deliver::from_syrup(&reply).expect("a delivery");
        assert_eq!(delivered.to, Desc::Export(0u64.into()));
        assert_eq!(
            delivered.args[0],
            Value::Symbol("fulfill".into()),
            "the fixture should have been fulfilled, not refused"
        );
        assert!(
            matches!(delivered.args[1], Value::Record(_)),
            "the node should hand back a descriptor for the object; got {:?}",
            delivered.args[1]
        );

        drop(client);
        node.shutdown();
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// **The node serves a peer over `unix`** (issue #249): the transport that is *not* a testing one,
/// authenticated by the socket's file mode rather than by a network address.
///
/// This pins the **transport**, and deliberately stops at the bootstrap: the peer dials over UDS,
/// completes the CapTP handshake, and fetches the node's **ERTP capability** from its bootstrap. The
/// bridge's own round trip — a delivery becoming a deploy and a block — is the two tests below, and it
/// is transport-agnostic: a delivery is handled inside the session loop, on whichever transport the
/// session arrived, so those tests' transport does not need duplicating here.
#[test]
fn a_peer_dials_the_node_over_unix_and_reaches_its_ertp_capability() {
    let dir = common::temp_dir("ocapn-unix");
    let ports = common::free_ports(6);
    let mut conf = common::deploy_conf(&dir, &ports);
    // A key, so the node publishes its chain-backed capabilities — ERTP among them — on the bootstrap.
    conf.dev.deployer_private_key = Some(common::VALIDATOR_PRIV_HEX.to_string());
    let node_socket = dir.join("ocapn.sock");
    conf.api_server.ocapn_listen_unix = Some(node_socket.display().to_string());
    // One transport at a time: this node listens on unix only.
    conf.api_server.ocapn_listen = None;

    common::test_runtime().block_on(async {
        let node = common::start(&conf, ports[2], ports[0]).await;
        wait_for_ocapn_unix(&node_socket).await;

        // A dialing side needs a listener of its own (the trait bears on one); it is never accepted on.
        let client_socket = dir.join("client.sock");
        let dialer = UnixNetlayer::bind(&client_socket)
            .await
            .expect("bind the dialing side");
        let connection = dialer
            .new_outgoing_connection(&unix_locator(&node_socket))
            .await
            .expect("dial the node over unix");
        let identity = Identity::fresh(unix_locator(&client_socket)).expect("a session key");
        let mut client = Session::dial(connection, &identity, Arc::new(Bootstrap::default()))
            .await
            .expect("the node should complete the handshake over unix");

        // Fetch the ERTP capability from the node's bootstrap (its export 0), reply to our export 0 —
        // the same shape the first test uses for the echo fixture.
        let fetch = Deliver {
            to: Desc::Export(0u64.into()),
            args: vec![
                Value::Symbol("fetch".into()),
                Value::Bytes(ERTP_SWISS.to_vec()),
            ],
            answer_pos: None,
            resolve_me_desc: Some(Desc::ImportObject(0u64.into())),
        };
        client
            .send_message(&fetch.to_syrup())
            .await
            .expect("send the fetch");

        let reply = client
            .recv_message()
            .await
            .expect("read the reply")
            .expect("a reply, not a closed connection");
        let delivered = Deliver::from_syrup(&reply).expect("a delivery");
        assert_eq!(
            delivered.args[0],
            Value::Symbol("fulfill".into()),
            "the ERTP capability should be fulfilled over unix, not refused; got {:?}",
            delivered.args.get(1)
        );
        assert!(
            matches!(delivered.args[1], Value::Record(_)),
            "the node should hand back a descriptor for the capability; got {:?}",
            delivered.args[1]
        );

        drop(client);
        node.shutdown();
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// The bridge: a CapTP delivery to a chain-backed capability becomes a signed deploy, and the value
/// that deploy wrote to its reply channel comes back as the CapTP promise's fulfilment.
#[test]
fn a_captp_delivery_to_a_chain_backed_capability_resolves_from_a_block() {
    let dir = common::temp_dir("ocapn-bridge");
    let ports = common::free_ports(6);
    let mut conf = common::deploy_conf(&dir, &ports);
    conf.api_server.ocapn_listen = Some(format!("127.0.0.1:{}", ports[5]));
    // The bridge signs with the node's deployer key and needs a block to carry the deploy.
    conf.dev_mode = true;
    conf.dev.deployer_private_key = Some(common::VALIDATOR_PRIV_HEX.to_string());
    conf.propose_on_deploy = true;

    common::test_runtime().block_on(async {
        let node = common::start(&conf, ports[2], ports[0]).await;
        wait_for_ocapn(ports[5]).await;
        wait_for_genesis(&format!("http://127.0.0.1:{}", ports[0])).await;

        let dialer = TcpTestingOnly::bind("127.0.0.1:0")
            .await
            .expect("bind the dialing side");
        let connection = dialer
            .new_outgoing_connection(&locator(ports[5]))
            .await
            .expect("dial the node");
        let identity = Identity::fresh(locator(0)).expect("a session key");
        let mut client = Session::dial(connection, &identity, Arc::new(Bootstrap::default()))
            .await
            .expect("the node should complete the handshake");

        // 1. Fetch the chain-backed capability from the bootstrap.
        let capability = fetch(&mut client, REV_VAULT_BALANCE_SWISS).await;

        // 2. Deliver to it: one REV address in. Everything after this is the node's own doing — it
        //    builds the term, signs the deploy, submits it, and waits for the block.
        let call = Deliver {
            to: capability,
            args: vec![Value::String(common::DEPLOYER_REV_ADDR.to_string())],
            answer_pos: None,
            resolve_me_desc: Some(Desc::ImportObject(1u64.into())),
        };
        client
            .send_message(&call.to_syrup())
            .await
            .expect("send the call");

        // The bridged deploy lands because **the node proposes it**: `propose_on_deploy` is on, and
        // that is the mechanism this test is exercising. So this explicit nudge — the shape
        // `deploy_block.rs` uses, kept as a second chance — normally arrives while the node's own
        // propose is already running, and the node answers `Failure: another propose is in progress`.
        // **That is not a failure here**, and asserting on it is how this test failed twice on CI: the
        // refusal says a propose is in flight, which is a block on its way, and the assertion that
        // matters is the reply below — the only thing that says a block carried the *deploy*. Both
        // answers have been measured here; only these two are acceptable, and neither is required.
        let propose = GrpcProposeService::connect("127.0.0.1", ports[2] as i32, 16 * 1024 * 1024)
            .await
            .expect("propose gRPC");
        match propose.propose(false).await {
            Ok(_) => {}
            Err(errors)
                if errors
                    .iter()
                    .any(|e| e.contains("another propose is in progress")) => {}
            Err(errors) => panic!("propose failed: {errors:?}"),
        }

        // 3. The reply is the balance the deployed `revVault.getBalance` put on its reply channel —
        //    read from a block and carried back over CapTP.
        //
        //    The value is the genesis wallet's balance **less the phlo this very deploy spent**
        //    (1_000_000_000_000 − 1_000_000). That is a stronger check than "a number arrived": the
        //    deploy paid for itself out of the vault it was reading, so the assertion pins the whole
        //    round trip rather than its shape.
        //
        //    **Bounded**, because "no block was produced" has to be a failure with the reason in it
        //    rather than a job that sits here until its timeout: the deploy lands only if a block
        //    carries it, and that is the node's half of this test.
        let reply = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            client.recv_message(),
        )
        .await
        .expect(
            "the bridge should answer within 120s — a delivered call becomes a deploy and a block",
        )
        .expect("read the reply")
        .expect("a reply, not a closed connection");
        let delivered = Deliver::from_syrup(&reply).expect("a delivery");
        assert_eq!(
            delivered.args[0],
            Value::Symbol("fulfill".into()),
            "the bridge refused the delivery: {:?}",
            delivered.args.get(1)
        );
        assert_eq!(
            delivered.args[1],
            Value::Int(999_999_000_000i64.into()),
            "the balance should be the genesis wallet's less this deploy's phlo"
        );

        drop(client);
        node.shutdown();
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// **The ERTP round trip over CapTP** (issue #249, clause 4): a peer holds an RChain *issuer* and
/// calls it.
///
/// What makes this different from the REV balance capability above is the **direction the objects
/// travel**. A balance is data, and data crosses CapTP as Syrup. An issuer, a purse and a brand are
/// *capabilities* — unforgeable names and bundles — and `par_value::par_to_value` refuses them by
/// design, because mapping one to data would copy the authority. So the bridge registers what a call
/// returns into the chain's registry in the same deploy that produced it, and hands the peer a CapTP
/// **descriptor** for each capability inside that reply. The peer then calls *those*.
///
/// The test walks the path clause 4 names: the contract, a kit, the issuer, a purse, and finally the
/// balance of that purse — a value that came out of a block, objects deep, from a reference the peer
/// was handed rather than one it knew the name of.
#[test]
fn a_captp_peer_holds_an_ertp_issuer_and_calls_it() {
    let dir = common::temp_dir("ocapn-ertp");
    let ports = common::free_ports(6);
    let mut conf = common::deploy_conf(&dir, &ports);
    conf.api_server.ocapn_listen = Some(format!("127.0.0.1:{}", ports[5]));
    conf.dev_mode = true;
    conf.dev.deployer_private_key = Some(common::VALIDATOR_PRIV_HEX.to_string());
    conf.propose_on_deploy = true;

    common::test_runtime().block_on(async {
        let node = common::start(&conf, ports[2], ports[0]).await;
        wait_for_ocapn(ports[5]).await;
        wait_for_genesis(&format!("http://127.0.0.1:{}", ports[0])).await;

        let dialer = TcpTestingOnly::bind("127.0.0.1:0")
            .await
            .expect("bind the dialing side");
        let connection = dialer
            .new_outgoing_connection(&locator(ports[5]))
            .await
            .expect("dial the node");
        let identity = Identity::fresh(locator(0)).expect("a session key");
        let mut client = Session::dial(connection, &identity, Arc::new(Bootstrap::default()))
            .await
            .expect("the node should complete the handshake");

        // 1. The ERTP contract itself — a capability whose *methods ride the message*, which is what
        //    makes an object with several arms holdable.
        let ertp = fetch(&mut client, ERTP_SWISS).await;

        // 2. `makeIssuerKit` → a descriptor for each member of the kit. Three capabilities came back,
        //    so three descriptors did: the bridge found them by inspecting the reply.
        let kit = call_many(&mut client, &ertp, "makeIssuerKit", &[]).await;
        assert_eq!(
            kit.len(),
            3,
            "a kit is a brand, a mint and an issuer — three capabilities, three descriptors: {kit:?}"
        );
        let issuer = kit[2].clone();

        // 3. The issuer makes a purse — **one** capability, so the reply is one descriptor rather
        //    than a list of one.
        let purse = call(&mut client, &issuer, "makeEmptyPurse", &[]).await;

        // 4. **The purse answers.** `getCurrentAmount` is data, so it comes back as a value — and it
        //    is the answer of a *live ERTP purse* on chain, reached through a reference this peer was
        //    handed over CapTP and never knew the name of.
        // It is a *value* reply, so it arrives in its Syrup form: the Rholang pair `(true, 0)` as the
        // **tagged record** a tuple crosses as — `<desc:tagged 'rho:tuple' [true, 0]>` (AUDIT C226;
        // the law is `Rchain.syrup_decode_encode`). This is what makes the pair come back a pair:
        // sent as a bare list it would arrive at a contract's `@(brand, value)` pattern as an `EList`
        // and match nothing, which is why a peer could hold a purse and not fund it.
        let reply = call_value(&mut client, &purse, "getCurrentAmount", &[]).await;
        let Value::Record(parts) = &reply else {
            panic!("a purse answers the tagged tuple, got {reply:?}");
        };
        assert_eq!(parts[0], Value::Symbol("desc:tagged".into()), "{reply:?}");
        assert_eq!(parts[1], Value::Symbol("rho:tuple".into()), "{reply:?}");
        let Value::List(fields) = &parts[2] else {
            panic!("the tuple's fields are a list, got {reply:?}");
        };
        assert_eq!(fields.len(), 2, "a purse answers a pair: {reply:?}");
        assert_eq!(
            fields[0],
            Value::Bool(true),
            "a fresh purse must read zero, not refuse: {reply:?}"
        );
        assert_eq!(
            fields[1],
            Value::Int(0.into()),
            "a fresh purse is empty: {reply:?}"
        );

        drop(client);
        node.shutdown();
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A capability the peer holds, passed back as an argument** (C224 item 4).
///
/// The gap this closes: a peer could *hold* a capability — the kit above proves it — but could not
/// hand one to a contract, because a capability is a descriptor and `par_value` refuses the label it
/// carries. So the ERTP arms that take one were unreachable, and `amountMath.make(brand, value)` is
/// the cheapest of them to demonstrate: the brand is a capability the peer was handed, the value is
/// data, and the contract answers with the amount it made *from both*.
///
/// Before the fix this delivery broke with `a Symbol — Rholang has no symbol for it to land in`,
/// which named the label rather than the situation.
#[test]
fn a_captp_peer_passes_a_capability_back_as_an_argument() {
    let dir = common::temp_dir("ocapn-ertp-arg");
    let ports = common::free_ports(6);
    let mut conf = common::deploy_conf(&dir, &ports);
    conf.api_server.ocapn_listen = Some(format!("127.0.0.1:{}", ports[5]));
    conf.dev_mode = true;
    conf.dev.deployer_private_key = Some(common::VALIDATOR_PRIV_HEX.to_string());
    conf.propose_on_deploy = true;

    common::test_runtime().block_on(async {
        let node = common::start(&conf, ports[2], ports[0]).await;
        wait_for_ocapn(ports[5]).await;
        wait_for_genesis(&format!("http://127.0.0.1:{}", ports[0])).await;

        let dialer = TcpTestingOnly::bind("127.0.0.1:0")
            .await
            .expect("bind the dialing side");
        let connection = dialer
            .new_outgoing_connection(&locator(ports[5]))
            .await
            .expect("dial the node");
        let identity = Identity::fresh(locator(0)).expect("a session key");
        let mut client = Session::dial(connection, &identity, Arc::new(Bootstrap::default()))
            .await
            .expect("the node should complete the handshake");

        // A kit, so the peer holds a brand to pass back.
        let ertp = fetch(&mut client, ERTP_SWISS).await;
        let kit = call_many(&mut client, &ertp, "makeIssuerKit", &[]).await;
        let (brand, issuer) = (kit[0].clone(), kit[2].clone());

        // The issuer makes the amountMath; the amountMath is an object, so one descriptor.
        let amount_math = call(&mut client, &issuer, "getAmountMath", &[]).await;

        // **The brand crosses as an argument.** `kit[0]` is the peer's handle on the brand, which is
        // the node's own export — so it is sent back as `<desc:export N>`, and the node resolves it to
        // the registry location it minted for the kit.
        // The reply is an amount `(brand, 10)` — **one** capability in it, so one descriptor: the
        // contract put the brand the peer passed into the value it answered with. A `break` here
        // would panic inside the helper with the reason, which is what this asserted before the fix.
        let amount = call(
            &mut client,
            &amount_math,
            "make",
            &[brand.to_syrup(), Value::Int(10.into())],
        )
        .await;
        assert_ne!(
            amount, brand,
            "the amount is a new object holding the brand, not the brand itself"
        );

        drop(client);
        node.shutdown();
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// **An amount a peer builds itself** (AUDIT C226's observable).
///
/// The gap this closes: a peer could pass a capability as an argument (the test above), but not a
/// capability *inside* a value — an ERTP amount is `(brand, value)`, and the brand is a capability
/// it holds. `amountMath.getValue` matches its argument against `@(brand, value)`, so the pair has to
/// arrive as a **tuple** with the brand **named**: sent as a bare list it would arrive an `EList`
/// and match nothing, and with the brand as data it could not be the same brand the contract knows.
///
/// Before the fix the delivery broke with "a capability nested inside a value cannot be named yet".
#[test]
fn a_captp_peer_sends_an_amount_and_the_contract_reads_it() {
    let dir = common::temp_dir("ocapn-ertp-amount");
    let ports = common::free_ports(6);
    let mut conf = common::deploy_conf(&dir, &ports);
    conf.api_server.ocapn_listen = Some(format!("127.0.0.1:{}", ports[5]));
    conf.dev_mode = true;
    conf.dev.deployer_private_key = Some(common::VALIDATOR_PRIV_HEX.to_string());
    conf.propose_on_deploy = true;

    common::test_runtime().block_on(async {
        let node = common::start(&conf, ports[2], ports[0]).await;
        wait_for_ocapn(ports[5]).await;
        wait_for_genesis(&format!("http://127.0.0.1:{}", ports[0])).await;

        let dialer = TcpTestingOnly::bind("127.0.0.1:0")
            .await
            .expect("bind the dialing side");
        let connection = dialer
            .new_outgoing_connection(&locator(ports[5]))
            .await
            .expect("dial the node");
        let identity = Identity::fresh(locator(0)).expect("a session key");
        let mut client = Session::dial(connection, &identity, Arc::new(Bootstrap::default()))
            .await
            .expect("the node should complete the handshake");

        let ertp = fetch(&mut client, ERTP_SWISS).await;
        let kit = call_many(&mut client, &ertp, "makeIssuerKit", &[]).await;
        let (brand, issuer) = (kit[0].clone(), kit[2].clone());
        let amount_math = call(&mut client, &issuer, "getAmountMath", &[]).await;

        // **The amount, as the peer builds it**: the tagged tuple a Rholang pair crosses as, whose
        // first field is the brand *reference* the peer holds.
        let amount = Value::Record(vec![
            Value::Symbol("desc:tagged".into()),
            Value::Symbol("rho:tuple".into()),
            Value::List(vec![brand.to_syrup(), Value::Int(10.into())]),
        ]);
        let reply = call_value(&mut client, &amount_math, "getValue", &[amount]).await;

        // `(true, 10)`: the contract read the brand out of the tuple, found it *its own* (`if (aBrand
        // == brand)`), and answered the value. A brand that had crossed as data could not have been
        // the same name, and one inside a bare list would not have reached the pattern.
        let Value::Record(parts) = &reply else {
            panic!("getValue answers a tagged tuple, got {reply:?}");
        };
        let Value::List(fields) = &parts[2] else {
            panic!("the tuple's fields are a list, got {reply:?}");
        };
        assert_eq!(
            fields,
            &vec![Value::Bool(true), Value::Int(10.into())],
            "the contract must read the amount the peer built: {reply:?}"
        );

        drop(client);
        node.shutdown();
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// Call `method` on `capability` and read the reply, which must be **one** descriptor.
///
/// One object is one descriptor, not a one-element list: `E(obj).makeEmptyPurse()` should hand the
/// caller a purse, and a caller made to destructure a collection to reach the only thing in it is a
/// worse peer. So this reads a bare delivery, not the list [`call_many`] reads.
async fn call(client: &mut Session, capability: &Desc, method: &str, args: &[Value]) -> Desc {
    let mut call_args = vec![Value::Symbol(method.to_string())];
    call_args.extend_from_slice(args);
    let reply = deliver_and_read(client, capability, call_args).await;
    match &reply {
        Value::Record(_) => match Desc::from_syrup(&reply).expect("a descriptor") {
            Desc::ImportObject(position) => Desc::Export(position),
            other => panic!("`{method}` answered {other:?}, not a descriptor"),
        },
        other => panic!("`{method}` answered {other:?}, not one descriptor"),
    }
}

/// Call `method` on `capability` and read a reply that must be **several** descriptors.
async fn call_many(
    client: &mut Session,
    capability: &Desc,
    method: &str,
    args: &[Value],
) -> Vec<Desc> {
    let mut call_args = vec![Value::Symbol(method.to_string())];
    call_args.extend_from_slice(args);
    let reply = deliver_and_read(client, capability, call_args).await;
    match &reply {
        Value::List(descriptors) => descriptors
            .iter()
            .map(|d| match Desc::from_syrup(d).expect("a descriptor") {
                Desc::ImportObject(position) => Desc::Export(position),
                other => panic!("expected a descriptor, got {other:?}"),
            })
            .collect(),
        other => panic!("`{method}` answered {other:?}, not descriptors"),
    }
}

/// Call `method` on `capability` and read a **value** reply.
async fn call_value(
    client: &mut Session,
    capability: &Desc,
    method: &str,
    args: &[Value],
) -> Value {
    let mut call_args = vec![Value::Symbol(method.to_string())];
    call_args.extend_from_slice(args);
    deliver_and_read(client, capability, call_args).await
}

/// Deliver `args` to `capability`, and return what the node fulfilled the delivery with.
async fn deliver_and_read(client: &mut Session, capability: &Desc, args: Vec<Value>) -> Value {
    let call = Deliver {
        to: capability.clone(),
        args,
        answer_pos: None,
        resolve_me_desc: Some(Desc::ImportObject(1u64.into())),
    };
    client
        .send_message(&call.to_syrup())
        .await
        .expect("send the call");
    let reply = client
        .recv_message()
        .await
        .expect("read the reply")
        .expect("a reply, not a closed connection");
    let delivered = Deliver::from_syrup(&reply).expect("a delivery");
    match delivered.args.as_slice() {
        [Value::Symbol(verb), value] if verb == "fulfill" => value.clone(),
        [Value::Symbol(verb), reason] if verb == "break" => {
            panic!("the bridge broke the call: {reason:?}")
        }
        other => panic!("expected a fulfilment, got {other:?}"),
    }
}

/// Fetch an object by swiss number from the peer's bootstrap, returning the descriptor to address
/// it by.
async fn fetch(client: &mut Session, swiss: &[u8]) -> Desc {
    let request = Deliver {
        to: Desc::Export(0u64.into()),
        args: vec![Value::Symbol("fetch".into()), Value::Bytes(swiss.to_vec())],
        answer_pos: None,
        resolve_me_desc: Some(Desc::ImportObject(0u64.into())),
    };
    client
        .send_message(&request.to_syrup())
        .await
        .expect("send the fetch");
    let reply = client
        .recv_message()
        .await
        .expect("read the fetch reply")
        .expect("a reply");
    let delivered = Deliver::from_syrup(&reply).expect("a delivery");
    assert_eq!(delivered.args[0], Value::Symbol("fulfill".into()));
    match Desc::from_syrup(&delivered.args[1]).expect("a descriptor") {
        Desc::ImportObject(position) => Desc::Export(position),
        other => panic!("expected a descriptor for the object, got {other:?}"),
    }
}

/// **The node starts a session of its own** (issue #249): `POST /api/v1/ocapn/dial` on the admin
/// server makes the node dial a peer the caller names and fetch the object at the swiss number it
/// gives.
///
/// The peer is the **node's own OCapN listener**, dialled over loopback under a designator of the
/// test's choosing — so the assertion pins the whole path (admin route → the dialer → the enlivener's
/// dial-and-fetch → a real CapTP session → a descriptor back) with no second node to stand up. The
/// object fetched is the echo fixture the listener publishes on every bootstrap.
///
/// The second case is the one a caller gets wrong: a **link-local target is refused with its reason**,
/// and the guard is the *target policy* rather than an origin — a dial the node starts has no peer
/// origin, so Law 62's rule has nothing to judge and the metadata endpoint must be refused by the
/// address rule alone. The route's own gate (`enable-ocapn-dial`) is asserted in `web::http`'s unit
/// tests, where the two statuses (404 and 503) can be told apart without a running node.
#[test]
fn the_admin_route_makes_the_node_dial_a_peer_and_refuses_a_link_local_target() {
    let dir = common::temp_dir("ocapn-dial");
    let ports = common::free_ports(6);
    let mut conf = common::deploy_conf(&dir, &ports);
    conf.api_server.ocapn_listen = Some(format!("127.0.0.1:{}", ports[5]));
    conf.api_server.enable_ocapn_dial = true;
    let dial = format!("http://127.0.0.1:{}/api/v1/ocapn/dial", ports[1]);
    // `base16`, because that is how the route takes a swiss number.
    let swiss = rchain_shared::base16::encode(ECHO_SWISS);

    common::test_runtime().block_on(async {
        let node = common::start(&conf, ports[2], ports[0]).await;
        wait_for_ocapn(ports[5]).await;
        let client = reqwest::Client::new();

        // 1. The node dials its own listener and fetches the echo fixture from its bootstrap.
        let ok = client
            .post(&dial)
            .json(&serde_json::json!({
                "designator": "self",
                "transport": "tcp-testing-only",
                "hints": { "host": "127.0.0.1", "port": ports[5].to_string() },
                "swiss": swiss,
            }))
            .send()
            .await
            .expect("the admin server should answer");
        assert_eq!(ok.status(), 200, "the dial should have fetched the fixture");
        let body: serde_json::Value = ok.json().await.expect("a JSON body");
        assert_eq!(body["peer"], "self.tcp-testing-only");
        assert!(
            body["fetched"].as_str().unwrap_or("").contains("Export"),
            "the answer names the object the peer exported: {body}"
        );

        // 2. **A link-local target is refused with its reason.** The metadata endpoint is never a
        //    CapTP peer, and the refusal is the policy's, before any connection is made.
        let refused = client
            .post(&dial)
            .json(&serde_json::json!({
                "designator": "metadata",
                "transport": "tcp-testing-only",
                "hints": { "host": "169.254.169.254", "port": "80" },
                "swiss": swiss,
            }))
            .send()
            .await
            .expect("the admin server should answer");
        assert_eq!(refused.status(), 502);
        let reason = refused.text().await.expect("a body");
        assert!(
            reason.contains("link-local"),
            "the refusal names why: {reason}"
        );

        node.shutdown();
    });
    let _ = std::fs::remove_dir_all(&dir);
}
