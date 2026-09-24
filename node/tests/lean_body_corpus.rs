//! Law 16c's `body` layer — the byte tie to `prost` (AUDIT C57's option (b)).
//!
//! `spec/conformance/body.tsv` is emitted by `lake exe rchain-corpus --layer body` from
//! `Rchain/Corpus.lean`'s `bodyCases`. Each row is the modelled subset of a `BlockMessageProto` plus
//! **the bytes the node produces for it**, and this consumer builds the proto from the row's fields,
//! encodes it with `prost`, and compares.
//!
//! **The expected bytes are the node's, observed first.** `the_bytes_the_node_produces` below prints
//! `prost`'s encoding of the candidate cases and asserts nothing; the rows in `Rchain/Corpus.lean` were
//! written from that output, and the Lean's own `native_decide` theorem then checks that the *model's*
//! encoder (`encodeProstBody`) reproduces them. So the row is the node's bytes rather than the model's,
//! and both sides are held to it independently — which is the difference between this layer and a
//! corpus that only agrees with itself.
//!
//! **What it ties, and the two rules it can fail on.** The port has no `BodyProto`: a block is one
//! `BlockMessageProto` with tags 1–17 (`models/proto/casper.proto:45-66`), and `hash_block`
//! (`casper/src/proto_util.rs:58-64`) clones the whole `BlockMessage`, clears exactly `blockHash` (32
//! zeros) and `sig` (empty), and encodes the rest with `prost`. The model's `encodeBody` narrows that to
//! six fields, so this layer covers the subset where the two structures correspond and pins:
//!
//! - **field order** — the model writes proto fields 4, 6, 5, 17, 9, 14; `prost` writes ascending
//!   (4, 5, 6, 9, 14, 17);
//! - **default-skipping** — `prost` omits a scalar or byte field equal to its default; the model writes
//!   unconditionally. A genesis block's `timestamp = 0` is the smallest instance.
//!
//! **The boundary, in two kinds, and they are not the same kind of omission.** *Fields omitted*:
//! `version`, `shardId`, `blockHash`, `preStateHash`, `postStateHash`, `bonds`, the three `rejected*`
//! sets and `sigAlgorithm` are in the proto and are not modelled here at all (modelling them is C57's
//! option (a)). *A field deliberately taken in the proto's shape*: `justifications` is `repeated bytes`
//! in the proto and a list of `Parent`s in the model, and those are different data rather than two
//! spellings of one thing — this consumer sets the proto's own `Vec<Vec<u8>>`.
//!
//! **Why the falsifier has to be a Rust mutation.** The Lean side's `native_decide` only shows that the
//! model is self-consistent; what makes this layer evidence is that the two encoders are independent. So
//! the check that it *can* fail is a mutation of `Case::proto` here — reorder the fields, or drop the
//! skipping — against the unchanged corpus. Both were run and recorded in the commit that added this
//! file.
//!
//! **This is the repository's first byte-level corpus layer.** Every other layer pins an identity
//! (which element sorts first, which token a spelling lexes to); a byte-level layer pins an *encoding*,
//! and a round trip cannot see either failure this one catches — `from_bytes (to_bytes b) = b` holds
//! under both field orders and under every skipping rule.

use prost::Message;
use rchain_models::proto::casper::{BlockMessageProto, RholangStateProto};

/// One case: the modelled subset as `spec/conformance/body.tsv` carries it.
struct Case {
    block_number: i64,
    sender: Vec<u8>,
    seq_num: i64,
    justifications: Vec<Vec<u8>>,
    timestamp: i64,
}

impl Case {
    /// The proto the node encodes: the six modelled fields set, `state` **present but empty**, every
    /// unmodelled field at its default — which `prost` then omits, which is the point.
    fn proto(&self) -> BlockMessageProto {
        BlockMessageProto {
            block_number: self.block_number,
            sender: self.sender.clone(),
            seq_num: self.seq_num,
            justifications: self.justifications.clone(),
            state: Some(RholangStateProto::default()),
            timestamp: self.timestamp,
            ..Default::default()
        }
    }

    fn bytes(&self) -> Vec<u8> {
        self.proto().encode_to_vec()
    }

    fn hex(&self) -> String {
        self.bytes().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A short label for the diagnostic print, so the observed line can be matched to its case.
    fn label(&self) -> String {
        format!(
            "bn={} sender={}B seq={} js={} ts={}",
            self.block_number,
            self.sender.len(),
            self.seq_num,
            self.justifications.len(),
            self.timestamp
        )
    }
}

/// 32 bytes of a recognisable pattern — the shape of a block hash or a validator id.
fn hash(seed: u8) -> Vec<u8> {
    (0..32u8).map(|i| seed.wrapping_add(i)).collect()
}

/// The candidate cases: one per rule, plus the smallest instances of each.
fn candidates() -> Vec<Case> {
    vec![
        // order: four non-default scalars/bytes, so the field order is observable from the second
        // field onwards.
        Case {
            block_number: 7,
            sender: hash(1),
            seq_num: 3,
            justifications: vec![hash(2), hash(3)],
            timestamp: 1000,
        },
        // skipping, at its smallest: everything default, so only the present-but-empty `state` is
        // written — the row that decides whether a present sub-message is skipped.
        Case { block_number: 0, sender: vec![], seq_num: 0, justifications: vec![], timestamp: 0 },
        // a genesis block with a sender: `timestamp = 0` is C57's smallest instance of the difference,
        // alongside `blockNumber`/`seqNum`.
        Case {
            block_number: 0,
            sender: hash(4),
            seq_num: 0,
            justifications: vec![hash(5)],
            timestamp: 0,
        },
        // short values: a one-byte sender and a one-byte justification pin the length prefixes at
        // their smallest non-zero.
        Case {
            block_number: 1,
            sender: vec![0x2a],
            seq_num: 2,
            justifications: vec![vec![0x7f]],
            timestamp: 0,
        },
        // a timestamp that is not the last field written, and no justifications at all: pins that the
        // repeated field contributes nothing when empty rather than an empty entry.
        Case {
            block_number: 42,
            sender: hash(6),
            seq_num: 1,
            justifications: vec![],
            timestamp: 9_999_999,
        },
    ]
}

/// **The observation — runs before anything is written down.** Prints `prost`'s bytes for each candidate
/// and asserts nothing: this is where the corpus rows' expected bytes come from, so that the row is the
/// node's answer rather than the model's.
#[test]
fn the_bytes_the_node_produces() {
    for c in candidates() {
        println!("body {}  ->  {}", c.label(), c.hex());
    }
}

/// One row of `spec/conformance/body.tsv`: `layer, blockNumber, senderHex, seqNum, timestamp,
/// justificationsHex (";"-separated), stateHex, bytesHex`.
struct Row {
    block_number: i64,
    sender: Vec<u8>,
    seq_num: i64,
    timestamp: i64,
    justifications: Vec<Vec<u8>>,
    state: Vec<u8>,
    bytes: Vec<u8>,
}

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "odd-length hex field: {s:?}");
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex field"))
        .collect()
}

fn rows() -> Vec<Row> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../spec/conformance/body.tsv");
    let text = std::fs::read_to_string(&path).expect("spec/conformance/body.tsv");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let c: Vec<&str> = line.split('\t').collect();
            assert_eq!(c.len(), 8, "body.tsv row has {} columns: {line:?}", c.len());
            assert_eq!(c[0], "body", "row is not a body row: {line:?}");
            Row {
                block_number: c[1].parse().expect("blockNumber"),
                sender: unhex(c[2]),
                seq_num: c[3].parse().expect("seqNum"),
                timestamp: c[4].parse().expect("timestamp"),
                justifications: if c[5].is_empty() {
                    Vec::new()
                } else {
                    c[5].split(';').map(unhex).collect()
                },
                state: unhex(c[6]),
                bytes: unhex(c[7]),
            }
        })
        .collect()
}

/// **The tie.** For every row, build the proto from the row's own fields — independently of the Lean,
/// which contributes only the expected bytes — encode it with `prost`, and compare. A row whose
/// expectation the node cannot reproduce, or whose fields the node cannot arrange into those bytes,
/// fails here with the row's index and the first differing byte.
///
/// The `state` column is the *sub-message's* serialization, decoded here rather than taken as opaque
/// bytes, so a row with a non-empty state would exercise the length prefix too; every current row is
/// the present-but-empty state, which is itself the case that decides whether `prost` skips it.
#[test]
fn the_node_encoder_reproduces_the_corpus_bytes() {
    let rows = rows();
    assert!(!rows.is_empty(), "the corpus is empty");
    for (i, row) in rows.iter().enumerate() {
        let proto = BlockMessageProto {
            block_number: row.block_number,
            sender: row.sender.clone(),
            seq_num: row.seq_num,
            justifications: row.justifications.clone(),
            state: Some(
                RholangStateProto::decode(&row.state[..]).expect("the row's state sub-message"),
            ),
            timestamp: row.timestamp,
            ..Default::default()
        };
        let got = proto.encode_to_vec();
        assert_eq!(
            got,
            row.bytes,
            "row {i}: prost produced {} bytes, the corpus observed {}",
            got.len(),
            row.bytes.len()
        );
    }
    // The layer is not degenerate: it must carry at least one row that is *all* defaults, because that
    // is the row the skipping rule is decided by, and at least one with several non-default fields,
    // because that is the row the order rule is decided by.
    assert!(
        rows.iter().any(|r| r.bytes.len() <= 2),
        "no row encodes to (nearly) nothing, so the default-skipping rule is not exercised"
    );
    assert!(
        rows.iter().any(|r| r.sender.len() >= 2 && r.justifications.len() >= 2),
        "no row has several non-default fields, so the field-order rule is not exercised"
    );
}
