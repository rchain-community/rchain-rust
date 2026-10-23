# A quantum-resistant rholo: plan and alternatives

> **This page is a plan, not a description of the code.** Nothing on it is implemented. It is kept in the
> book at the owner's request, as an exception to the quarantine rule in `AGENTS.md`; every claim about
> *existing* code cites the file it describes, and every other statement is a proposal.

*Status: plan. Nothing here is implemented. Grounded in `rholo` (then `rchain-rust`) at `a22b7c2` (`dev`, 2026-10-07). Sizes and
speeds are order-of-magnitude figures from the published parameter sets and common benchmarks; re-measure
on the node's own hardware before fixing any budget.*

**Tracking issue:** [rhovision-org/rholo#274](https://github.com/rhovision-org/rholo/issues/274)
(work items and owner; this document holds the alternatives).

---

## 0. Summary

**What is exposed today.** Every signature on the consensus path is **secp256k1 ECDSA**, not Ed25519:
`from_algorithm` (`crypto/src/signatures/signatures_alg.rs:34`) resolves only `"secp256k1"` and
`"secp256k1:eth"`, and Ed25519 is deliberately disabled there (RCHAIN-3560). Ed25519 survives only off-chain
(OCapN session identity and handoff certificates, `ocapn/src/conn.rs:222`, `ocapn/src/handoff.rs:110`) and as
the `rho:crypto:ed25519Verify` system process. Shor's algorithm breaks all three ECC curves in use
(secp256k1, P-256 for TLS, Curve25519/Ed25519). The hashes (Blake2b-256, Keccak-256, SHA-256) are fine.

**What breaks if a large quantum computer appears.** An attacker recovers the private key of any public key it
has seen. On RChain that means:

1. **Vault theft.** Spending authority is `deployerId`, which is the deployer's public key
   (`models/src/normalizer_env.rs:42`). Every account that has ever deployed has published its key.
2. **Validator impersonation.** Blocks are signed with the validator's secp256k1 key
   (`casper/src/validate.rs:40`); a forged key lets the attacker equivocate *as* the victim, getting the
   victim slashed, or (with enough stake broken) finalize a fork.
3. **Long-range history rewrite.** Keys of validators that have since unbonded can be broken at leisure and
   used to build an alternative history from an old point. This is the one "harvest now, forge later"
   signature risk.

**What is not broken.** Unforgeable names are not signatures and not derived from deploy signatures (§3), the
RSpace trie and block hashes are Blake2b-256, and addresses are hashes of keys. All of these survive, with the
caveats in §2 and §3.

**Recommended path, in one paragraph.** Add **ML-DSA-44** (FIPS 204) as a new `sig_algorithm` for deploys and
**ML-DSA-65** for validators, behind the existing `from_algorithm` registry. Before that, make three
structural changes that are cheap now and expensive later: refer to keys by a **32-byte key hash** instead of
inlining them everywhere (the `bonds` map in every block, the random seed), give deploys a **content-hash ID**
instead of using the signature as the ID, and generalize the fixed `Validator([u8; 65])` type. Then run a
**dual-format transition** with a published secp256k1 sunset, and keep a pre-written **emergency cutover**
ready in case the sunset has to be pulled forward. Each choice has real alternatives, compared below.

**Three shortcuts, added 2026-10-07.** §16 sets out what can be done in the wallet rather than on the chain. §14 shows how to make the first fork the *last* coordinated binary
upgrade, by shipping every later step as dormant, height-activated or governance-activated code. §15 shows
that **opt-in quantum-secure vaults for high-value accounts need no fork at all**: a vault whose spending is
gated by a hash-based one-time signature checked in plain Rholang, using only `rho:crypto:blake2b256Hash`,
`ByteArray.nth` and an unforgeable-name-owned REV vault, all of which exist on the chain today.

---

## 1. Where keys and signatures flow today

| Surface | Algorithm | Where | What the key or signature is used for |
|---|---|---|---|
| Algorithm registry | secp256k1, secp256k1:eth | `crypto/src/signatures/signatures_alg.rs:34` | The single dispatch point for every on-chain verify. **This is the crypto-agility hook**: a new scheme is one match arm. |
| Signed-payload hash | Blake2b-256 (Keccak-256 + Ethereum prefix for `:eth`) | `crypto/src/signatures/signed.rs:62` | Deploys are signed over `blake2b256(serialized DeployData)`. |
| Deploy signature | per `sig_algorithm` field | `models/src/casper/protocol/casper_message.rs:160` (`SignedDeployData`), `:189` (`verify_signature`); API ingress `node/src/api/conversion.rs:292` | Authenticates the deployer. Every deploy in a block is re-verified on receipt (`casper/src/validate.rs:606`). |
| **Deploy identity** | the signature bytes | `casper/src/validate.rs:447` (`repeat_deploy`), `BlockMessage.rejected_deploys` (`casper_message.rs:780`), `/api/v1/deploy-status/{deploy_signature}` (`node/src/web/http.rs:1205`) | The signature *is* the deploy ID, normalized to low-S (`signatures_alg.rs:122`) to defeat ECDSA malleability. |
| Deployer authority | the public key itself | `GDeployerId { public_key }` (`models/src/ast.rs:455`), `normalizer_env.rs:42` | `rho:rchain:deployerId` is the capability vaults and the registry check. |
| REV address | Keccak-256 of the key → 20-byte ETH address → Keccak-256 + Blake2b checksum | `rholang/src/util/rev_address.rs:58` | Requires a 65-byte key (`key_length == VALIDATOR_LENGTH`). |
| Block signature | hard-coded `"secp256k1"` | `casper/src/proto_util.rs:99`, `casper/src/validator_identity.rs:29,45`; verify `casper/src/validate.rs:40` | Signs the 32-byte `hash_block` (`proto_util.rs:63`). |
| Validator identity | raw 65-byte uncompressed key | `models/src/validator.rs:10` (`Validator([u8; 65])`, `Copy`, ~72 constructor call sites across ~40 files) | Block `sender`, the `bonds` map carried in **every block** (`casper_message.rs:779`), latest-message tables, slashing. |
| Equivocation evidence | block sig | `casper_message.rs:497` (`is_signed_by`) | Slashing proof is the offender's own signed block hash. |
| Unforgeable-name seed | Blake2b-512 PRNG over (shard, block number, **sender key**, pre-state hash) | `casper/src/block_random_seed.rs:95`, `:174` (`var_size`) | Per-deploy names split by index (`split_byte`, max 255 per block, `proposer.rs:622`). |
| Rholang crypto | secp256k1, Ed25519 | `rholang/src/system_processes.rs:793,825`; `rho:registry:insertSigned:secp256k1` at `:758` | Contracts verifying signatures themselves; the genesis `Registry.rho` uses `secp256k1Verify`. |
| P2P transport | TLS (rustls with the `ring` provider) + P-256 self-signed certs | `comm/Cargo.toml:22`, `crypto/src/util/certificate_helper.rs:163` | Node ID = last 20 bytes of `keccak256(cert public key)` (`certificate_helper.rs:34`). |
| OCapN | Ed25519; X25519 for the Noise `XX` netlayer | `ocapn/src/conn.rs:222`, `ocapn/src/handoff.rs:110`; `ocapn/src/noise.rs` over `crypto/src/encryption/x25519.rs` | Session identity and handoff certificates (off-chain, live); Noise key agreement that encrypts CapTP traffic. |
| Encryption | Curve25519 `crypto_box` | `crypto/src/encryption/curve25519.rs` | No caller outside the crypto crate today. (The raw X25519 primitive beside it serves the OCapN Noise netlayer, row above.) |
| Formal model | abstract `sign`/`verify`/`sign_verify_roundtrip` axioms | `spec/laws.tsv`, Law 19 | Algorithm-agnostic, so the model needs no change for a new scheme (§9). |

Two constraints shape every library decision below. The workspace targets **wasm32** for the crypto crate
(`crypto/tests/wasm_law19.rs`, issue #98), and its own crates are `#![forbid(unsafe_code)]`. A C library
reached through FFI is therefore a bigger step here than in most Rust projects.

---

## 2. Threat model and timing (decision point D0)

The question that sets urgency is not "when is Q-day" but "what can an attacker do *before* we finish".

| Risk | Needs a quantum computer when? | Exposure today |
|---|---|---|
| Forge a deploy from an exposed key and drain a vault | At attack time | Every account that has ever deployed |
| Forge blocks as a bonded validator | At attack time | All bonded validators |
| Long-range attack with old, unbonded validator keys | Any time in the future | Every key that was ever bonded, forever |
| Decrypt recorded P2P traffic | Any time in the future | Low value: block and deploy content is public anyway |
| Decrypt recorded OCapN Noise sessions | Any time in the future | Higher: CapTP messages between peers are not public, and X25519 key agreement is broken by Shor, so traffic recorded today can be read later |
| Break an address whose key was never revealed | At attack time; Grover only, ~2^80 for the 160-bit ETH-address step | Small |

**Alternatives for the overall posture:**

| | A. Wait and prepare | B. Prepare now, migrate on a schedule | C. Migrate now, hard |
|---|---|---|---|
| What happens | Write the code and the emergency plan, ship nothing consensus-visible | Ship PQ algorithms as an option, run a long dual-format window, set a sunset date | Cut over to PQ-only at the next fork |
| Cost | Lowest | Moderate | Highest: every wallet and tool breaks at once |
| Risk if Q-day comes early | High: migration happens under attack, unmigrated funds have no safe path | Low: most value has rotated before the sunset | Lowest |
| Fit for this project | Poor: the long-range risk accrues now | **Recommended** | Poor: no PQ wallet ecosystem for REV yet |

The long-range risk is the argument against A: every block signed with secp256k1 today is history a future
attacker can try to rewrite. It is mitigated mainly by finality anchoring (§5.4), not by new signatures alone.

---

## 3. Unforgeable names (decision point D1)

**The premise needs correcting first.** On the block path, unforgeable names are **not** derived from deploy
signatures. They come from `BlockRandomSeed` (`casper/src/block_random_seed.rs`): Blake2b-512 over
`(shard_id, block_number, sender public key, pre_state_hash)`, split by the deploy's position in the block. The
Scala-era `tools::rng(signature)` (`casper/src/tools.rs:30`) was ported but nothing on the block path calls it.

So the quantum question for names is narrow:

- **In-language unforgeability is not cryptographic.** Rholang has no syntax to construct a `GPrivate`, so a
  quantum attacker gains nothing inside the calculus. Names stay unforgeable.
- **Unguessability** of a 32-byte name rests on Blake2b. Grover gives ~2^128 work for a preimage, which is
  acceptable.
- **Names exported as hashes** (`RevAddress::from_unforgeable`, `rev_address.rs`, Keccak-256 of the id) are
  likewise hash-protected only.
- **The real problem is size, not security.** The seed embeds the sender's key behind a **1-byte length
  prefix** (`var_size` in `block_random_seed.rs`). An ML-DSA-65 key is 1,952 bytes. Oversized components are
  now refused with `BlockRandomSeedError::ComponentTooLong` and propagated through block creation/replay
  instead of panicking, but the v1 seed format still cannot represent a PQ validator key.
- **Genesis names are safe.** Genesis seeds use an empty sender key (`from_shard_id`), so the system contract
  names (`rev_vault_unforgeable`, `transfer_unforgeable`, and the others) do not change under any option below.

| Option | What changes | Effect on existing names | Effect on the formal model |
|---|---|---|---|
| **D1-a. Put a 32-byte validator ID (key hash) in the seed** | New block version computes the seed from `blake2b256(sender key)` | Old blocks replay unchanged (old version keeps the old seed); new blocks get different names than the same block would under v1, which is fine since no v2 block exists yet | Law 11 (replay determinism) holds per version; the seed codec gains a version tag |
| D1-b. Widen the prefix to `u16` or a varint | Seed encoding only | Same version-gating need; v1 bytes must stay byte-identical | Same |
| D1-c. Keep the full key and drop the prefix (fixed width per algorithm) | Seed encoding is algorithm-dependent | Ties the name stream to the signature algorithm, which is the wrong coupling | Worse: a key rotation changes the name stream |

**Recommendation: D1-a.** It falls out of the key-ID change in §6 at no extra cost, and it decouples the name
stream from whichever signature scheme a validator uses.

---

## 4. Signature schemes (decision point D2)

### 4.1 The candidates

| Scheme | Standard | Public key | Signature | Sign | Verify | Maturity and caveats |
|---|---|---|---|---|---|---|
| secp256k1 ECDSA (today) | SEC 2 | 33 / 65 B | 64–72 B | ~50 µs | ~70 µs | Broken by Shor |
| **ML-DSA-44** (Dilithium2) | FIPS 204 (final, Aug 2024) | 1,312 B | 2,420 B | ~0.1–0.3 ms | ~0.05–0.1 ms | Lattice (Module-LWE). Simple integer arithmetic, constant-time is easy. Category 2 |
| **ML-DSA-65** (Dilithium3) | FIPS 204 | 1,952 B | 3,309 B | ~0.2–0.4 ms | ~0.1 ms | Category 3 |
| ML-DSA-87 | FIPS 204 | 2,592 B | 4,627 B | ~0.3–0.5 ms | ~0.15 ms | Category 5 |
| **FN-DSA-512** (Falcon-512) | FIPS 206 (draft at last check; confirm status) | 897 B | ~666 B | ~0.2–0.5 ms | ~0.03–0.05 ms | Smallest lattice signatures. Signing needs floating-point Gaussian sampling, which is hard to make constant-time and side-channel safe, and awkward on wasm |
| FN-DSA-1024 | FIPS 206 draft | 1,793 B | ~1,280 B | ~0.5–1 ms | ~0.1 ms | Category 5 |
| **SLH-DSA-128s** (SPHINCS+) | FIPS 205 (final) | 32 B | 7,856 B | ~100–500 ms | ~0.3–1 ms | Hash-based only: the most conservative assumption. Tiny keys, big and slow signatures |
| SLH-DSA-128f | FIPS 205 | 32 B | 17,088 B | ~5–20 ms | ~1–3 ms | Faster signing, bigger signatures |
| XMSS / LMS | RFC 8391 / 8554, SP 800-208 | ~32–64 B | ~2–5 KB | fast | fast | **Stateful**: reusing one-time state leaks the key. A validator restored from backup, or run active/standby, could sign twice from the same state |
| Hybrid secp256k1 + ML-DSA-44 | composite (IETF drafts) | ~1,377 B | ~2,490 B | sum | sum | Secure if *either* holds. Doubles code paths |

### 4.2 How each fares against this codebase's constraints

| Criterion | ML-DSA | FN-DSA | SLH-DSA | XMSS/LMS | Hybrid |
|---|---|---|---|---|---|
| Fits a full block (255 deploys) | ~950 KB of key+sig per block (-44) | ~400 KB | ~2 MB | ~1 MB | ~970 KB |
| Validator signing cadence | fine | fine | 128s too slow to sign blocks briskly; 128f is OK but 17 KB per block | fine, but statefulness is disqualifying for validators | fine |
| Pure-Rust, wasm32 | RustCrypto `ml-dsa` exists (pre-1.0, check audit status) | Weakest: floating point on wasm, few audited pure-Rust implementations | RustCrypto `slh-dsa` exists | few crates | as ML-DSA |
| Strong unforgeability (matters while the signature is the deploy ID, §6.2) | yes (SUF-CMA claimed) | yes | weaker claims; do not rely on it | n/a | per component |
| Assumption risk | lattices: newer | lattices + implementation risk | hashes only: lowest | hashes only | lowest of the pair |

### 4.3 Alternatives for which scheme goes where

| Option | Deploys (users) | Blocks (validators) | Pros | Cons |
|---|---|---|---|---|
| **D2-a. ML-DSA everywhere** | ML-DSA-44 | ML-DSA-65 | One family, one library, simple and fast, NIST final | Largest per-deploy cost of the lattice options; all eggs in lattices |
| D2-b. FN-DSA for users, ML-DSA for validators | FN-DSA-512 | ML-DSA-65 | Smallest deploys (~1.5 KB vs ~3.7 KB) | Two families; FN-DSA signing lives in wallets (JS, mobile, hardware) where side-channel-safe floating point is hardest; standard not final at last check |
| D2-c. Lattice for users, SLH-DSA for validators | ML-DSA-44 | SLH-DSA-128f | Validator security rests on hashes only, which is where a long-range forgery would hurt most | +17 KB per block; ~10 ms to sign |
| D2-d. Hybrid (ECDSA + ML-DSA) during transition only | composite | composite | Safe against a lattice break *and* a quantum break; good while lattices are young | More bytes and code; must be removed later, which is a second migration |
| D2-e. Algorithm menu: the registry accepts several, users choose | any of the above | one chosen | Lets wallets pick; future-proof | Every validator must verify all; larger audit surface; fee schedule must price signature bytes per algorithm |

**Recommendation: D2-a, with D2-e's mechanism kept open.** The `sig_algorithm` string and `from_algorithm`
registry already make algorithm choice a per-message field, so adding SLH-DSA later as a conservative option is
a match arm and a fee entry, not a redesign. Prefer pure PQ over hybrid for deploys (D2-d doubles the
migration), but consider hybrid for **validators** only, where the count is small and the long-range
consequences are largest.

---

## 5. Casper consensus (decision point D3)

### 5.1 What is signed

There are no separate attestation messages: every block is the sender's attestation, signed once over its
32-byte hash (`validate.rs:40`). Justifications are block hashes (`casper_message.rs:778`), not signatures, so
they do not grow. Per block the signature cost is **one block signature plus every deploy's signature and key**.

### 5.2 The bonds map is the hidden multiplier

Every block carries `bonds: BTreeMap<Validator, i64>` (`casper_message.rs:779`), i.e. one full public key per
bonded validator.

| Validators | secp256k1 (65 B) | ML-DSA-65 (1,952 B) | With 32-byte key IDs |
|---|---|---|---|
| 10 | 0.7 KB | 20 KB | 0.4 KB |
| 100 | 6.5 KB | 195 KB | 3.2 KB |
| 1,000 | 65 KB | 1.95 MB | 32 KB |

Inlining PQ keys here makes **every** block, including empty ones, grow with the validator set. This is the
strongest single argument for key IDs (§6.1).

### 5.3 Finality latency

| Factor | secp256k1 | ML-DSA-65 validators, ML-DSA-44 deploys | Comment |
|---|---|---|---|
| Block sign | ~50 µs | ~0.3 ms | negligible |
| Block verify (header) | ~70 µs | ~0.1 ms | negligible |
| Verify 255 deploys | ~18 ms | ~15–25 ms | comparable; can be parallelized across cores, deploys are independent |
| Bytes gossiped per full block (sig material only) | ~35 KB | ~950 KB | **this is the latency cost**: one extra round-trip-scale delay per hop on a slow link |
| Bytes per empty block, 100 validators, inline keys | ~6.5 KB | ~200 KB | avoidable with key IDs |

Finality needs a supermajority of validators to build on a block, so latency is roughly (gossip time + verify
time) × the number of rounds to finalize. With key IDs and ML-DSA, verify time is a wash and gossip grows only
with deploy volume. With SLH-DSA validators (D2-c), each block gains ~17 KB and ~10 ms signing; that is
acceptable at a cadence of seconds, not at sub-second cadence.

### 5.4 Long-range attacks and validator-key migration

| Option | How validators move to PQ keys | Pros | Cons |
|---|---|---|---|
| **D3-a. Rebond at an epoch boundary** | Validator submits a PoS deploy, signed with its old key *and* the new PQ key, naming the new key; takes effect at the next epoch | Uses existing PoS epoch machinery; stake never leaves | Must happen **before** Q-day: a rotation authorized only by the old key proves nothing afterwards. Epoch-boundary code has a known freeze when stake changes (issue #83), so it must be fixed first |
| D3-b. Unbond and rebond fresh | Normal unbond, then bond the PQ key | No new contract code | Stake is locked through the unbonding period; the net's validator set shrinks meanwhile, which is unsafe on a small net |
| D3-c. Hybrid validator keys from the start | Validators always sign with both | No rotation event; immediate protection | Bigger blocks; two verifies per block |

**Long-range defense is separate from the key choice.** Whatever is picked, nodes should refuse any fork that
reverts a block below their last finalized state, and new nodes should bootstrap from a trusted recent
finalized checkpoint (weak subjectivity) rather than from genesis. The node already restores from the last
finalized state (LFS); the plan is to make "never revert below LFS" an explicit, tested rule, and to publish
signed checkpoint hashes out-of-band.

**Recommendation:** D3-a, done early in the transition, with finality anchoring as a separate deliverable.

---

## 6. Data model changes that pay off regardless of scheme (decision point D4)

### 6.1 Key identity: inline keys or key IDs?

| Option | Description | Block size | Code change | Downsides |
|---|---|---|---|---|
| D4-a. Inline full keys everywhere (today's shape) | `Validator` becomes `Vec<u8>` or an enum per algorithm | grows with key size × (validators + deploys) | `Validator([u8; 65])` and its `Copy`/`Ord` uses (~72 sites, ~40 files) | Every block pays §5.2's cost |
| **D4-b. Key ID = blake2b256(alg ‖ key), keys stored on-chain** | `sender`, `bonds`, `deployerId` carry a 32-byte ID; the full key lives once in a key registry (PoS state for validators, a per-account record for deployers) | constant per validator; a deploy carries its key only on first use | Same `Validator` refactor, plus a key-registry lookup in verify | Verify needs state access; first deploy from an account is bigger |
| D4-c. Inline keys in deploys, IDs for validators | Hybrid of the two | deploys still pay ~1.3 KB of key each | Smaller change than D4-b | Deploy cost stays high; `deployerId` semantics stay key-shaped |

**Recommendation: D4-b for validators now, and for deployers in the same fork if the key registry is ready.**
The key ID also replaces the 65-byte key in the random seed (§3) and in `RevAddress` derivation (§7).

**Effect on `deployerId`.** Today `GDeployerId` carries the raw key. Under D4-b it carries the key ID. Contracts
compare `deployerId` by identity and never look inside it, so this is transparent to Rholang, except for
contracts that pass `deployerId` to `secp256k1Verify`-style checks, which need the full key and would read it
from the registry.

### 6.2 Deploy identity: signature or content hash?

The signature is the deploy ID today (§1). Under PQ schemes that is both **costly** (a 2.4–17 KB ID in
`rejected_deploys`, the dedup set, and the deploy-status URL) and **fragile**: ML-DSA's default signing is
randomized ("hedged"), so the same deploy signed twice has two different signatures. That is not a forgery,
but it means "same signature" no longer means "same deploy", which `repeat_deploy` (`validate.rs:447`) relies
on.

| Option | Deploy ID | Replay protection | Cost |
|---|---|---|---|
| D4-d. Keep signature as ID | sig bytes | relies on strong unforgeability and on deterministic signing; breaks for hedged ML-DSA re-signing | large IDs |
| **D4-e. ID = blake2b256(serialized DeployData ‖ deployer key ID)** | 32 bytes | replay is "same content", which is what was meant all along; the low-S normalization hack (`signatures_alg.rs:122`) becomes unnecessary for new deploys | API and storage change |
| D4-f. Both, during transition | sig for v1 deploys, hash for v2 | as above per version | two code paths during the window |

**Recommendation: D4-e for new-format deploys, D4-f for the transition.** The deploy-status API gains a
lookup-by-ID route and keeps the signature route for v1 deploys.

---

## 7. Hash functions (decision point D5)

| Hash | Used for | Quantum strength | Action |
|---|---|---|---|
| Blake2b-256 | block hash, RSpace trie, signed-payload hash, checksums | ~2^128 preimage (Grover); collision 2^128 classically and ~2^85 by the quantum BHT algorithm, which also needs ~2^85 of quantum memory, so it is costlier in practice than parallel classical search (Bernstein, *Cost analysis of hash collisions*, 2009) | **No change** |
| Blake2b-512 PRNG | unforgeable names | as above | No change |
| Keccak-256 | addresses, `:eth` signatures | as Blake2b-256 | No change |
| SHA-256 | `rho:crypto:sha256Hash` | as above | No change |
| **20-byte ETH address step** in `RevAddress` | `from_public_key` truncates to 160 bits before re-hashing | ~2^80 Grover preimage on an *unrevealed* key | Retire for PQ keys (below) |
| 20-byte P2P node ID | Kademlia identity, `certificate_helper.rs:34` | ~2^80 | Low priority; becomes key-ID based with §8 |

**Options for addresses:**

| Option | Description | Pros | Cons |
|---|---|---|---|
| D5-a. Keep the ETH-address derivation for PQ keys | `keccak(pq_key)[12..]` | Address format unchanged | Still requires `key_length == 65` today; keeps a 160-bit bottleneck |
| **D5-b. New address version byte: payload = key ID (32 B)** | `RevAddress` gets a version for PQ accounts; base58 checksum unchanged | Full 256-bit binding; same derivation as validators | New address format for wallets |
| D5-c. Migrate the hash functions too (e.g. SHA3 or Blake3) | Rehash state | none for quantum | Changes every state root for no security gain. **Not recommended** |

A hash migration is **not needed**. D5-b is recommended because it is the same key ID as §6.1.

---

## 8. Serialization, networking and storage ripple effects (decision point D6)

### 8.1 Serialization

- `Validator([u8; 65])` with `Copy` is the largest mechanical change. Introduce a `ValidatorId([u8; 32])` (D4-b),
  keep `Copy`, and move the full key to a lookup. Without key IDs the type must become a heap-allocated
  variable-length key, losing `Copy` across ~40 files.
- `var_size`'s 1-byte prefix in the random seed (§3) remains a 255-byte format limit, but the encoder now
  returns `BlockRandomSeedError::ComponentTooLong` instead of panicking. A PQ validator key still requires
  the versioned seed format in §3 before it can be used.
- Protobuf `bytes` fields carry any length, so `BlockMessage` and `DeployDataProto` need no wire-format change
  for bigger signatures; the version bump (`models/src/block_version.rs:6`) and `sig_algorithm` string carry the
  semantic change.
- JSON/HTTP DTOs (`node/src/api/dto.rs`) serialize keys as hex: a 1,312-byte key is 2.6 KB of hex. Prefer
  key IDs in responses.

### 8.2 Networking

| Item | Today | Options | Recommendation |
|---|---|---|---|
| Block gossip size | KB-scale | grows per §5.3; the transport already streams large blobs in chunks with a 256 MiB per-stream ceiling (`comm/src/transport/grpc_transport_receiver.rs`) | No limit change needed; measure gossip latency on the two-host testnet |
| TLS key exchange | ECDHE via rustls `ring` provider | (a) switch rustls to the `aws-lc-rs` provider, which offers hybrid X25519+ML-KEM-768; (b) leave as is | (a) is a dependency change only, no consensus impact. Lower priority: the traffic is public data |
| TLS peer authentication | P-256 self-signed certs, node ID = keccak(cert key) | (a) PQ X.509 certs (ML-DSA in TLS is not broadly supported by rustls yet); (b) keep TLS for the channel and add an application-level handshake where the node signs the TLS channel binding with an ML-DSA node key; (c) bind node ID to the validator key ID | (b) now, (c) for validator nodes. Peer spoofing affects routing, not consensus validity, since blocks are signed independently |
| OCapN sessions | Ed25519 identity; Noise `XX` with X25519 | (a) add ML-DSA as a session-identity algorithm and for handoff certificates; (b) add an ML-KEM-768 encapsulation to the Noise handshake (hybrid, as PQNoise-style patterns do), keeping X25519 so interop with today's peers still works; (c) leave both until the OCapN spec settles | (b) first: the Noise traffic is the one place here where "record now, decrypt later" exposes non-public data. (a) follows the OCapN spec community; neither is consensus-critical |

### 8.3 Storage

| Store | Growth | Mitigation options |
|---|---|---|
| Block store | dominated by deploy signatures and keys | (a) key IDs (§6.1); (b) **prune signatures** of deploys in blocks below the last finalized state, keeping the deploy ID and content: a finalized block's validity is settled by consensus, so the signature is only needed to re-validate from genesis; (c) keep full archives on a subset of archival nodes |
| Deploy dedup index | IDs per deploy in the expiration window | content-hash IDs (§6.2) keep this at 32 bytes |
| RSpace / tuple space | unchanged (hashes only) | none needed |
| Key registry | one key per account and validator, ~1.3–2 KB each | small relative to block data |

**Phlo pricing.** Signature verification and the extra bytes must be priced. The deploy's phlo charge should
include a per-algorithm verification cost and a per-byte cost for signature and key bytes, otherwise a deployer
choosing SLH-DSA-128f imposes 17 KB per deploy for free.

---

## 9. Formal spec and audit impact

- **Law 19** models signatures as abstract `sign`/`verify` with `sign_verify_roundtrip` (`spec/laws.tsv`). It is
  scheme-agnostic, so ML-DSA fits the existing axioms. The known-answer test pinning required by the translation
  contract (AGENTS.md, "Axiomatized crypto … pinned by known-answer test vectors") means each new scheme needs
  the NIST ACVP vectors as tests, including on wasm32 (`crypto/tests/wasm_law19.rs`).
- **New laws worth stating**, since each change above is consensus-visible: (1) deploy identity is the content
  hash and is invariant under re-signing; (2) the random seed is a function of the sender's key ID, not the
  key's encoding; (3) `from_algorithm` is total over the versioned algorithm set (a block version determines
  which algorithms are valid).
- Each consensus change belongs in the audit register with a C-number reserved when its branch opens, per
  AGENTS.md's rules.

---

## 10. Deploy format and migration strategy (decision point D7)

### 10.1 Transition shape

| | **D7-a. Dual-format window, then sunset** | D7-b. Hard cutover | D7-c. Indefinite coexistence |
|---|---|---|---|
| How | Block version 2 accepts secp256k1 *and* ML-DSA deploys; a later version rejects secp256k1 | One fork switches to PQ-only | Both forever; users choose |
| Wallet impact | gradual | everyone at once | none forced |
| Security at Q-day | protected if the sunset precedes it; an emergency fork can pull it forward | protected | **unprotected**: any revealed secp256k1 key can still sign |
| Complexity | two verify paths for a period | lowest code | two paths forever |
| Recommendation | **Yes** | only as the emergency playbook | No |

### 10.2 Moving funds from secp256k1 accounts to PQ accounts

This is the hardest part, because after Q-day an old-key signature no longer proves ownership.

| Option | Mechanism | Works after Q-day? | Cost |
|---|---|---|---|
| **M1. Transfer before sunset** | User sends REV from the old vault to a new PQ vault with an ordinary signed transfer | No; must happen before the sunset | Zero protocol work: it already works once PQ deploys exist |
| M2. Key-binding deploy | Old key signs "my new key is K_pq"; the vault's authority moves to K_pq | No; same timing as M1 | Small contract change; keeps the address |
| M3. Commit now, reveal later | Today, publish `hash(K_pq ‖ salt)` signed by the old key. After Q-day, reveal K_pq and salt, signed by K_pq | **Yes**, as long as the commitment predates Q-day | Small contract; needs a deadline for commitments |
| M4. Zero-knowledge proof of seed | Prove (in a STARK, which is hash-based) knowledge of the wallet seed that derives the secp256k1 key | **Yes**, even without prior commitment | Large engineering effort; depends on wallets using a derivable seed |
| M5. Freeze at sunset | Unmigrated vaults with revealed keys become non-spendable | protects against theft by freezing | Users who miss the window lose access until M3 or M4 exists |
| M6. Unrevealed addresses | An address whose key never appeared on-chain can still migrate after Q-day by revealing the key in a PQ-authorized deploy | mostly; a fast quantum attacker could front-run between reveal and inclusion | Needs the PQ deploy to carry the old key in the same message |

**Recommendation:** M1 as the default path, M3 as a cheap insurance policy opened early in the window, M5 at
sunset, M4 left as research.

### 10.3 Rholang-level effects

- `rho:registry:insertSigned:secp256k1` and the `secp256k1Verify`/`ed25519Verify` system processes stay for
  old contracts; add `rho:crypto:mlDsa44Verify` (and an `insertSigned:ml-dsa-44` variant) so contracts can
  verify PQ signatures. Contracts that verify secp256k1 signatures on-chain need their own migration, which the
  protocol cannot do for them.
- Genesis contracts (`casper/src/genesis/resources/*.rho`) that call `secp256k1Verify` are the ones to audit.

---

## 11. Phasing: incremental versus coordinated

| Phase | Work | Consensus change? | Depends on |
|---|---|---|---|
| **P0″. Wallet hygiene (§16)** | Wallet keeps high-value funds at never-revealed addresses, sweeps whole balances, flags exposed accounts, manages one-time keys and M3 commitments | **No** | nothing; a wallet release |
| **P0′. Opt-in PQ vaults (§15)** | `PQVault.rho` (hash-based one-time signatures in Rholang), client signer, phlo measurements, audit; deployed as an ordinary contract | **No** | nothing; can start immediately |
| **P0. Groundwork** | ML-DSA (and SLH-DSA) in `crypto` behind a feature, **not** in `from_algorithm`; NIST KAT tests on host and wasm32; benchmarks; size budget measured on a real block; `var_size` overflow returned as a typed error; `ValidatorId` type introduced internally | No | — |
| **P1. Node-local and transport** | rustls `aws-lc-rs` provider with hybrid ML-KEM; hybrid ML-KEM in the OCapN Noise handshake; application-level ML-DSA node handshake; deploy-status by content ID alongside signature; wallet/client libraries (`rnode` CLI, JS client) able to produce ML-DSA deploys against a dev net | No | P0 |
| **P2. Fork 1: block version 2** | `from_algorithm` gains `ml-dsa-44`/`ml-dsa-65` for v2 blocks; key IDs in `sender`, `bonds`, seed, `deployerId`; key registry; content-hash deploy IDs; new address version; phlo pricing for signature bytes; `mlDsa44Verify` system process; finality "never revert below LFS" rule | **Yes, coordinated**; a testnet restart today, so land it now (§14.2) | P0, P1; issue #83 fixed before validators rebond |
| **P3. Transition window** | Validators rebond with PQ keys at epoch boundaries (D3-a); users move funds (M1) or commit (M3); monitor the share of value still on revealed secp256k1 keys | No new fork | P2 |
| **P4. Fork 2: sunset** | v3 blocks reject secp256k1 deploys and validator signatures; freeze unmigrated revealed-key vaults (M5); M3 reveals accepted | **Yes, coordinated** | P3 metrics |
| **Emergency** | A pre-reviewed P4 branch kept rebased on `dev`, so a credible quantum break can be answered with a fork in days rather than months | Yes | P2 deployed |

§14 recommends folding P2 and P4 into a single agility fork, so the table's two coordinated forks become one. What each phase can be done without: P0 and P1 can merge to `dev` any time and ship in normal releases. P2 and
P4 need every validator to upgrade at a set block height. P3 is operational.

---

## 12. Decisions needed, with the default this plan takes

| # | Decision | Default in this plan | Main alternative |
|---|---|---|---|
| D0 | Posture | Prepare now, migrate on a schedule | Wait and prepare only |
| D1 | Name seed | 32-byte key ID in the seed, version-gated | Widen the length prefix |
| D2 | Signature schemes | ML-DSA-44 deploys, ML-DSA-65 validators | FN-DSA for deploys; SLH-DSA for validators; hybrid |
| D3 | Validator migration | Rebond at epoch boundary, plus LFS anchoring | Unbond and rebond; hybrid from day one |
| D4 | Data model | Key IDs and content-hash deploy IDs | Inline keys, signature IDs |
| D5 | Hashes | Keep all; new address version | Keep ETH-style addresses |
| D6 | Transport | Hybrid ML-KEM TLS, app-level ML-DSA node handshake | Wait for PQ certificates in rustls |
| D7 | Transition | Dual-format window, then sunset; M1 + M3 + M5 | Hard cutover; indefinite coexistence |

## 13. Open questions to check before committing

1. Audit status and API stability of the pure-Rust RustCrypto `ml-dsa` and `slh-dsa` crates, and whether
   FIPS 206 (FN-DSA) is final. These move faster than this document.
2. Measured block size and gossip latency on the two-host testnet with ML-DSA keys and signatures, before
   fixing the phlo price per signature byte.
3. Whether the key registry for deployers is acceptable from a privacy and UX angle, or deployers should keep
   inline keys (D4-c) while validators use IDs.
4. Whether to make validator keys hybrid (D2-d for validators only) given the long-range exposure.
5. How the testnet's genesis wallets file (`~/.rnode/genesis/wallets.txt`) and `bonds.txt` express PQ keys or
   key IDs.

---

## 14. Avoiding a hard fork later

**What cannot be avoided.** Block validation re-verifies every deploy through `from_algorithm` and rejects an
unknown `sig_algorithm` outright (`casper/src/validate.rs:606`). There is no "old nodes accept what they cannot
check" path of the kind Bitcoin soft forks use, so the first time a block contains an ML-DSA deploy or an
ML-DSA block signature, every validator must already run code that verifies it. One coordinated upgrade is
the floor for protocol-level PQ signatures.

**What can be avoided is the second and third.** The plan as written has two forks (P2 turns PQ on, P4 turns
secp256k1 off) and implies more for any later change. They collapse into one if the first fork ships the whole
transition as code that is already present and only waiting to be switched on.

| Technique | What ships in the one fork | What a later change becomes | Risk |
|---|---|---|---|
| **H1. Versioned algorithm table** | `from_algorithm(alg, block_number)` reads a table of `(algorithm, role, active_from, active_until)` instead of a fixed match | Adding SLH-DSA, or sunsetting secp256k1, is a row whose heights are already in the binary | Heights fixed at release time; changing them is a new release |
| **H2. Governance-set table** | The same table, read from on-chain state at the block's pre-state (the rgov machinery, `casper/src/genesis/rgov.rs`, already resolves governance weights) | A governance deploy flips a row; no binary change | Replay determinism (Law 11) must read the table from pre-state only; governance capture becomes a security risk |
| **H3. Dormant schemes** | ML-DSA-44/65 and SLH-DSA verifiers compiled in with `active_from = ∞` | Activation by H1 or H2 | Unexercised code paths; must still be tested on a dev net |
| **H4. Format headroom** | Key IDs (§6.1), content-hash deploy IDs (§6.2), a version byte in the random seed (§3), the PQ address version (§7), phlo pricing per signature byte by algorithm | None of these need changing again for a new scheme | More change in the first fork |
| **H5. Pre-written sunset rules** | The freeze-unmigrated-vaults rule (M5) and the M3 reveal path, dormant | The P4 sunset becomes an H1/H2 activation instead of a release | The rule's scope must be decided early |
| **H6. Contract-level PQ (§15)** | Nothing | Users protect themselves with no protocol change at all | Covers vaults only, not validators |

**Recommendation.** Fold P2 and P4 into one "agility fork" that carries H1, H3, H4 and H5, with the sunset height
set conservatively far out and **H2 as the only way to move it earlier**. Pulling the sunset forward is then a
governance vote, not an emergency release, which is exactly what the emergency playbook in §11 needed. Start
H6 now, since it needs no fork. **Land the agility fork now, not when a trigger fires** (§14.2): today a hard fork
costs only a testnet restart.

**The gate H1 needs now exists, without any scheme in it.** The block path checks a block's version against a
compiled schedule of `(version, from_height)` rows (`rchain_models::block_version::SCHEDULE`,
`models/src/block_version.rs`), and new consensus code asks `is_active(version, height)`. The shipped table is the
single row `(1, 0)`, so nothing is staged yet; the agility fork adds its row, and the algorithm table above can key
its `active_from` on the same heights.

What still forces a future fork: a break in ML-DSA itself that requires a scheme *not* compiled into the binary,
or a change to the block format beyond the headroom in H4. H3's dormant SLH-DSA is the hedge for the first.


### 14.1 Triggers: what evidence moves each step

The agility fork itself is not trigger-gated: it lands now (§14.2). What the triggers govern is the **sunset of
classical cryptography**, which the fork ships dormant. H2 makes it possible to pull the sunset forward with a
governance vote. These triggers say what evidence should prompt that vote, so the decision is made against criteria agreed in advance rather than under pressure. They
are cumulative, and each is a public, checkable event. The idea and the first two thresholds come from DarkWow's
[quantum threat model](https://github.com/PatrickMockridge/DarkWow/blob/linear-master/doc/src/arch/quantum-threat.md);
the actions are this plan's.

| # | Trigger | Observable signal | Action here |
|---|---|---|---|
| T0 | None: the state today | — | P0′, P0″, P0 and P1 proceed; the agility fork is built and tested |
| T1 | ≥ 1,500 error-corrected logical qubits demonstrated, two-qubit gate fidelity above 99.9% | Peer-reviewed result, or a NIST/NSA/NCSC advisory | Open the dual-format window if it is not already open (the agility fork itself is already live, §14.2); validators rebond with PQ keys; wallets start prompting exposed high-value accounts; set a deadline for M3 commitments |
| T2 | A quantum break of a small ECDLP instance (≥ 112-bit, e.g. secp112r1) | Published cryptanalysis | Governance moves the sunset to a short fixed horizon; no new secp256k1 validator bonds |
| T3 | Standards bodies deprecate ECDSA/EdDSA for new systems | A NIST IR or FIPS publication (NIST IR 8547 sets a deprecation timeline; check its current dates) | Sunset height no later than the date those algorithms are disallowed |
| T4 | A cryptographically relevant quantum computer is demonstrated (256-bit ECDLP, or RSA-2048 factored) | Published result | Emergency: activate the sunset now through H2; freeze unmigrated revealed-key vaults (M5); accept only PQ deploys; exclude validators without PQ keys at the next epoch |

The evidence for any trigger is recorded on the tracking issue before the governance deploy that acts on it.


### 14.2 Timing: land the agility fork now

There are no external validators yet. A **hard fork** — one that needs a new genesis — therefore costs a testnet
restart and nothing else, and every month that passes makes the same change dearer. So the agility fork is
recommended **now**, rather than gated on trigger T1. Once it has landed, every later post-quantum change it
anticipated (activating a dormant scheme, moving the sunset, accepting native ML-DSA in `PQVault`) is a table
activation or, at most, an R-node binary swap, not another genesis.

Two kinds of fork, in the sense this project uses the words:

| | **Soft fork** | **Hard fork** |
|---|---|---|
| What changes | The `rnode` binary; the chain continues from its current state | The genesis; the testnet restarts |
| Example | The deployer index (#277); activating a row the agility fork shipped dormant | The agility fork itself: block version 2, key IDs, content-hash deploy IDs, the seed version byte |
| When it can happen | Any time **until external validators exist**, since every validator is ours to upgrade | Now cheaply; after external validators join, only with their coordination and a migration of their state |
| What it costs | A coordinated binary replacement | A restart, plus the state-preservation step below |

Once external validators join, a binary change that alters what is valid becomes a coordinated upgrade too.
That is the deadline this section is racing.

### 14.3 State preservation across a hard fork

A hard fork restarts the testnet, so before it runs there has to be a documented export-and-replay step, so that
no prior information is lost. Three options, from cheapest to most complete:

| Option | What carries over | What is lost | Cost |
|---|---|---|---|
| **S1. Balances and bonds into the new genesis** | REV balances per address into `wallets.txt`, and stake per validator into `bonds.txt`, read at the last finalized state (LFS) | Contract state, registry entries, unforgeable names, history | Small: a read of the vault and PoS state at LFS, and a script that writes the two genesis files |
| **S2. Archive the old chain read-only** | Full history, served by one node that keeps the old block store and API | Nothing is deleted, but the old chain is no longer extended | Small, and composes with S1 or S3 |
| **S3. A state-carrying genesis** | All RSpace state: the new genesis starts from the old LFS post-state root, exported as the trie the node already transfers to joining nodes (the LFS sync path; `casper/tests/fringe_restore.rs` exercises the restore side) | Block history, unless S2 is also done | Medium: genesis has to accept an imported state root instead of building one. Names already in state are just values and survive; only **new** names use the v2 seed |

**Recommendation: S1 + S2 now, S3 if the testnet holds contract state anyone needs.** The checklist before
pulling the trigger: freeze deploys at a named block; record the LFS block hash and state root; export balances and
bonds (and the state trie, for S3); publish the hashes of every export, so the new genesis can be checked against
them; keep the archive node running.

---

## 15. Opt-in quantum-secure accounts now, without a fork

**Yes, this can be offered soon.** Everything it needs is already on the chain:

- `rho:crypto:blake2b256Hash` (`rholang/src/system_processes.rs:801`), with Keccak-256 and SHA-256 beside it.
- `ByteArray.nth`, which returns a byte as an integer (`rholang/src/reduce.rs:1138`), so a contract can read the
  bits of a hash with ordinary integer arithmetic.
- REV vaults owned by an unforgeable name rather than a key: `RevVault!("unforgeableAuthKey", unf, ret)`
  (`casper/src/genesis/resources/RevVault.rho:94`). `MultiSigRevVault.rho` in the same directory is a working
  precedent for a contract that holds a vault's authority and gates it with its own rules.

**The design.** A `PQVault` contract owns an unforgeable-name vault and refuses to move funds unless the request
carries a valid **hash-based one-time signature** (Lamport or Winternitz). Hash-based signatures need nothing but
a hash function to verify, and their security rests only on Blake2b's preimage resistance, which a quantum
computer weakens to about 2^128 work. That is the same assumption SLH-DSA rests on.

1. **Create.** The owner generates a one-time key pair offline and deploys `PQVault` with only the 32-byte hash of
   the public key. The contract creates a fresh unforgeable name, gets the vault's auth key for it, and keeps
   both private. The vault's REV address (`RevAddress.fromUnforgeable`) is published.
2. **Fund.** The owner transfers REV from their ordinary secp256k1 vault to that address. This is a normal
   transfer and must happen while secp256k1 is still safe.
3. **Spend.** The owner signs the message `(destination, amount, nonce, hash of next public key)` with the
   one-time key. Anyone can submit it in a deploy, with any key paying the phlo. The contract checks the hash of
   the revealed public key against its stored commitment, verifies the signature by hashing, then transfers,
   increments the nonce and stores the next key's hash.

**Why the submitting deploy's key does not matter.** After Q-day an attacker can forge any secp256k1 deploy, but
cannot produce a valid one-time signature, and cannot alter the signed message: the destination, amount and
next key are all under the signature. Replaying the same message is refused by the nonce. A quantum attacker
who breaks validator keys can censor or equivocate, but cannot make honest full nodes accept a state
transition the contract refused, since every node replays the deploy.

### 15.1 Signature choices for the contract

| Scheme | Signature + key revealed per spend | Hash calls to verify | Notes |
|---|---|---|---|
| **Lamport** (256-bit message hash) | ~16 KB (256 preimages + 256 sibling hashes) | ~257 | Simplest to write and audit. Recommended first version |
| Winternitz, w = 16 | ~2.1 KB (67 chains × 32 B) | ~500 on average | About 8× smaller deploys, more loop logic in Rholang |
| Merkle tree of one-time keys (XMSS-like) | ~2.1 KB + ~32 B × tree height | ~500 + height | Many spends from one commitment; the leaf index lives on-chain in the contract, so the backup-reuse danger of stateful schemes applies only to the signer's own records |
| Native ML-DSA via a new system process | 3.7 KB | 1 native call | Cheapest per spend, but adding a system process is itself a coordinated upgrade. Becomes available with the §14 fork; the contract can then accept it too |

### 15.2 Rules the owner must follow

- **Sign each one-time key exactly once.** Signing two different messages with the same Lamport or Winternitz key
  leaks enough of it to forge. If a spend deploy fails, resubmit the *same* signed message; never re-sign with
  different contents.
- **Fund only through the published address,** and keep the one-time secret keys offline.
- **Move funds in while secp256k1 is safe.** Once Q-day comes, a transfer *into* the PQ vault from an exposed
  secp256k1 vault is a race with the attacker.

### 15.3 What it does and does not protect

| Protected | Not protected |
|---|---|
| Funds in the PQ vault against a quantum forger of secp256k1 deploys | Funds left in ordinary vaults |
| Funds against stolen validator keys, as long as full nodes replay | Liveness: a quantum attacker holding validator keys can still stall or censor the chain until §5.4's measures are in |
| Without any node release | Contracts the owner calls *from* the PQ vault's funds once they leave it |

### 15.4 Work to make it available

| Step | Size |
|---|---|
| Write `PQVault.rho` (Lamport first), modelled on `MultiSigRevVault.rho` | small |
| Block-path tests in `casper/tests/`, in the style of `casper/tests/ertp.rs`: create, fund, spend, replayed message refused, wrong next-key refused, forged signature refused | small |
| Offline signer: key generation, message hashing and signing, as a Rust CLI subcommand and a JS function for wallets | small to medium |
| Measure phlo per spend on a dev net, and confirm the 16 KB deploy fits comfortably | small |
| Independent review of the contract, since a bug in it is a loss of funds | the long pole |
| Optional: register the contract under a well-known URI so wallets can find it, and later, at the §14 fork, accept native ML-DSA in the same vault | later |

None of this touches consensus code, so it can ship as soon as it is tested and reviewed, ahead of every phase
in §11.

---

## 16. Doing the work in the wallet instead of on the chain

The chain has to enforce *who may spend*; nothing in a wallet can replace that. But much of the rest of the
work can live in the wallet. Moving it there costs no fork and no consensus code, and it ships at the speed of
a wallet release. The question at each step is whether the chain needs to *know* something or only to *check*
it.

### 16.1 What can move to the wallet

| Work | On the chain today or in the plan | In the wallet instead | What the chain still does |
|---|---|---|---|
| **Limiting key exposure** | nothing; every deploy reveals the key | Keep high-value REV at an address whose key has **never signed a deploy**. Spend from it only by sweeping the whole balance to a fresh unrevealed address in the same deploy, so the key is exposed only between submission and finality. Use a separate hot key for everyday deploys | Nothing new. Works today against any attacker who cannot break a key within one block interval |
| **One-time-key bookkeeping** for PQ vaults (§15) | the contract stores the current key commitment | Key generation, the chain of next-key commitments, and the rule "never sign twice with one key" live in the wallet, which also keeps the failed-deploy resubmission of the *same* signed message | Stores one 32-byte commitment and a nonce |
| **Many spends from one commitment** | — | The wallet holds a Merkle tree of one-time keys and tracks the leaf index | Stores only the root; checks a Merkle path (about 32 B × height) |
| **Cheap-to-verify encodings** | the contract parses whatever it gets | The wallet sends the signature pre-arranged for a single pass in Rholang (preimages in bit order, the hash chain lengths already split), so the contract does the hashing and nothing else | Recomputes the message digest and the hashes; never trusts the layout without checking it |
| **Migration commitments** (M3) | a small contract stores commitments | The wallet derives the PQ key deterministically from the user's existing seed (one backup covers both keys), computes `hash(K_pq ‖ salt)`, and submits the commitment early | Stores the commitment; accepts the reveal later |
| **Proof of seed ownership** (M4) | a verifier on chain | The wallet generates the STARK proof that it knows the seed behind the secp256k1 key; all the heavy computation is client-side | Verifies a proof. Still needs an on-chain verifier, so it is the one item here that eventually needs a fork or a costly contract |
| **Long-range defense** (§5.4) | "never revert below LFS" in the node | Light-client wallets pin a recent finalized checkpoint and refuse a chain that contradicts it (weak subjectivity), from a source the user trusts | The node rule still matters for full nodes |
| **Choosing what to protect** | — | The wallet flags accounts whose key is revealed and whose balance is above a threshold, and offers a one-click move to a PQ vault | Nothing |
| **Signing format readiness** | the fork adds ML-DSA | The wallet ships ML-DSA signing early, behind a flag, against a dev net, so the day the fork activates no wallet release is needed | Verification arrives with the fork |

### 16.2 Alternatives for how much to put in the wallet

| Option | Description | Pros | Cons |
|---|---|---|---|
| W0. Chain does everything | Protocol PQ signatures, on-chain key registry, chain-side migration | One source of truth; wallets stay thin | Every step waits for a fork |
| **W1. Wallet-first hygiene plus the §15 contract** | The wallet does exposure limiting, one-time-key state, commitments and encodings; the chain only runs `PQVault.rho` | Available **now**; no node release; easy to iterate | Protection depends on users running a wallet that does it; the contract still needs review |
| W2. Wallet-heavy cryptography | The wallet also produces proofs (M4) and aggregates, and the chain verifies only succinct objects | Smallest chain footprint long term | Proof systems are the least mature part; on-chain verification of a STARK still needs a fork or a large contract |
| W3. Relayer in between | A service accepts PQ-signed requests, wraps them in deploys and pays phlo | Users need no REV for phlo and no secp256k1 key at all | Adds a party that can censor (but not steal, since the contract checks the PQ signature) |

**Recommendation: W1 now, with W3 as an optional convenience.** Address hygiene is the cheapest protection
there is and can ship in the next wallet release. The `PQVault` contract plus wallet-side key management gives
high-value accounts real quantum security with no chain change. Protocol signatures (§11's agility fork)
still matter for everyone else, and for validators, which no wallet can protect.

### 16.3 What cannot move to the wallet

- **Validator signatures.** Blocks are verified by every node; a validator's protection is a protocol matter.
- **Spending authority** for ordinary vaults. As long as `deployerId` is a secp256k1 key, a forged deploy spends
  the vault whatever the owner's wallet does.
- **Verification itself.** A wallet can make verification cheaper and smaller, but anything the chain relies on
  it must check itself.
