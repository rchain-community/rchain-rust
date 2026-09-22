//! The rgov governance contracts, vendored as genesis content — the **testnet** setup.
//!
//! Upstream deploys this set at runtime (`scripts/bootstrap-rgov.ts`): a funded key deploys the
//! classes in dependency order, records the resulting URIs, feeds them back into the dependents and
//! builds a master directory. The recorded master URI is a product of that deployment, so it shifts
//! on every chain and goes stale silently. Genesis installs the set instead, with **fixed keys**, so
//! every URI is a constant of the chain (`spec/GENESIS.md` is the manifest).
//!
//! What is installed, in order (`governance_deploys`):
//!
//! 1. the eight classes (`RGOV_CORE`), each registering exactly as upstream wrote it;
//! 2. the master directory (`master_directory_template`, upstream's seven slots rendered with the
//!    class constants) — it publishes the deployer's `MasterContractAdmin` capability and mints the
//!    **read cap**;
//! 3. `extra_directory_slots_source` — our own term filling the three slots upstream's template
//!    omits (`Chat`, `Ballot`, `Group`), because the wallet's editor asks the directory for those
//!    names and an unwritten slot answers `Nil`;
//! 4. the `GetMe`/`SendThem` feature, which is what a client's first call resolves.
//!
//! **Two registrations must not be conflated.** A *class* registers with `insertArbitrary`
//! (upstream's shape) and keeps that shape: every consumer destructures the **bare** value
//! (`for (Dir <- lookCh)`), so converting it to `insertSigned` — whose value is `(nonce, value)` —
//! breaks the template silently (a deploy that "processes with success" and produces nothing, which
//! is how this was found). To give clients a hardcodable key anyway, each class *publishes* the URI
//! it was given and genesis copies the entry onto the constant key ([`contract_uri_for`]). Everything
//! this module changes is asserted against the vendored text, so a drift upstream fails the build.
//!
//! **Testnet scope, and what mainnet needs instead:** installing a master directory and a `GetMe`
//! feature for one fixed key means that key holds `@[*deployerId, "MasterContractAdmin"]` for the
//! chain — fine for a testnet (and it is what makes the client-side URIs constant), unacceptable on
//! a public network, where each deployer must run their own template + feature deploy from their own
//! key and the node must install none of steps 2–4. `spec/GENESIS.md` carries the full list.

use rchain_crypto::hash::blake2b256::hash as blake2b256;
use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::signatures::secp256k1::Secp256k1;
use rchain_crypto::signatures::signatures_alg::SignaturesAlg;
use rchain_crypto::signatures::signed::Signed;
use rchain_models::casper::protocol::casper_message::{DeployData, SignedDeployData};
use rchain_rholang::registry::build_uri;
use rchain_shared::base16;

const KUDOS_RHO: &str = include_str!("resources/rgov/Kudos.rho");
const INBOX_RHO: &str = include_str!("resources/rgov/Inbox.rho");
const DIRECTORY_RHO: &str = include_str!("resources/rgov/Directory.rho");
const MEMBER_ID_GOV_REV_RHO: &str = include_str!("resources/rgov/memberIdGovRev.rho");
const ISSUE_RHO: &str = include_str!("resources/rgov/Issue.rho");
const BALLOT_RHO: &str = include_str!("resources/rgov/Ballot.rho");
const CHAT_RHO: &str = include_str!("resources/rgov/Chat.rho");
const GROUP_RHO: &str = include_str!("resources/rgov/Group.rho");
/// The `GetMe`/`SendThem` feature: what a client's first governance call (`newInbox`) looks up.
const MEMBER_DIRECTORY_RHO: &str = include_str!("resources/rgov/MemberDirectory.rho");
const MASTER_DIRECTORY_RHO: &str =
    include_str!("resources/rgov/create-master-contract-directory-testnet.rho");

/// `accounting.MAX_VALUE` — the nonce every system contract registers with (the element consumers
/// destructure and ignore: `for (@(_, X) <- ch)`).
const PHLO_LIMIT: i64 = i32::MAX as i64;

/// The upstream deployment order (`bootstrap-rgov.ts`'s `STEPS`, from the original `Jakefile.js`):
/// `memberIdGovRev` imports `directory.rho` and `inbox.rho`, so those must be installed first.
///
/// `ballot`, `chat` and `group` are not in upstream's `STEPS`, but the master directory has slots
/// for them here: the wallet's editor asks the directory for those three class names, and a slot
/// that was never filled answers `Nil` — which a client cannot tell from "broken". Installing them
/// is the testnet scope decision (`spec/GENESIS.md`).
pub const RGOV_CORE: &[&str] = &[
    "kudos",
    "inbox",
    "directory",
    "roll",
    "issue",
    "ballot",
    "chat",
    "group",
];

/// The fixed private key for a vendored contract, *derived* rather than pasted: a hardcoded hex
/// constant would be unauditable (nobody can check it was not chosen adversarially), whereas a named
/// hash can be recomputed by anyone reading this file. It is the same value on every chain, which is
/// what makes the resulting `rho:id` a constant.
pub fn contract_key(name: &str) -> String {
    base16::encode(&blake2b256(format!("rnode/genesis/rgov/{name}").as_bytes()))
}

/// The one **dummy key** the three testnet governance terms share: the master directory, the extra
/// slots and the `GetMe` feature.
///
/// They must share it, and that is not a detail: the template publishes its
/// `@[*deployerId, "MasterContractAdmin"]` capability *for its own deployer*, and the feature's
/// registration is gated on reading that capability back. With three different keys the feature's
/// gate never opens, so it registers nothing, the directory answers `Nil` for `GetMe`, and a client
/// calling it gets silence — verified on a node: the handshake reached "directory answered GetMe"
/// and never entered `getMe` at all.
pub fn testnet_governance_key() -> String {
    contract_key("testnet-governance")
}

/// The terms that share [`testnet_governance_key`] (everything that reads or writes the master
/// directory's admin capability).
fn shares_testnet_governance_key(name: &str) -> bool {
    matches!(name, "masterDirectory" | "extraSlots" | "memberDirectory")
}

/// The fixed timestamp for a vendored contract. Genesis deploys carry no real time; a constant makes
/// the deploy (and therefore its terms' ordering) reproducible. `spec/GENESIS.md` records the value.
const RGOV_TIMESTAMP: i64 = 1_700_000_000_000;

/// The `rho:id` each vendored class is **published under** — a key this node chooses, derived from a
/// named string so anyone can recompute it, and constant for every chain of a given shard id.
///
/// It is *not* the key the deploy registers under. Upstream registers with
/// `rho:registry:insertArbitrary`, and that shape is what every consumer destructures
/// (`for (Dir <- lookCh)` — the bare value, *not* the `(nonce, value)` tuple `insertSigned` stores);
/// converting the registration to `insertSigned` breaks the master-directory template silently, which
/// is how this was found (a template deploy that "processed with success" and produced nothing).
///
/// So the registration stays exactly as upstream wrote it, and each class **publishes** the URI it
/// was given ([`URI_PUBLISH_CHANNEL`]); genesis copies the entry onto this key
/// (`seed_rgov_class_aliases`). A client hardcodes *this*, and the copy is what makes the key
/// independent of the deploy order — the registered URI is `blake2b256` of the deploy's RNG state,
/// deterministic here but moved by inserting any deploy before it.
pub fn contract_uri_for(name: &str) -> Result<String, String> {
    Ok(build_uri(&blake2b256(
        format!("rnode/genesis/rgov-class/{name}").as_bytes(),
    )))
}

/// The key the master directory's **read capability** is published under — the value a governance
/// client needs as its `ReadcapURI`, and the one genuinely per-deployer artifact the testnet setup
/// makes constant (`spec/GENESIS.md`).
pub fn readcap_uri() -> Result<String, String> {
    Ok(build_uri(&blake2b256(b"rnode/genesis/rgov-readcap")))
}

/// The channel a vendored class publishes its registered URI on, as `["<name>", <uri>]`.
///
/// The publish datums stay in the genesis state: they are the deterministic record of what was
/// installed, and nothing else reads this channel (it is a fixed, genesis-only name).
pub const URI_PUBLISH_CHANNEL: &str = "rnode:genesis:rgov-uri";

/// The genesis term for one vendored contract, adapted as `spec/GENESIS.md` and the NOTICE describe.
pub fn source(name: &str) -> Result<String, String> {
    match name {
        "kudos" => publish_registration(KUDOS_RHO, name, "deployId!([\"#define $Kudos\", uri])"),
        "issue" => publish_registration(ISSUE_RHO, name, "deployId!(uri)"),
        "directory" => {
            let source = publish_registration(DIRECTORY_RHO, name, "deployId!(uri)")?;
            // The file's tail exercises the contract it just registered.
            cut_from(&source, "} |\n    directory!(Nil, *ret)")
        }
        "inbox" => {
            let source = publish_registration(INBOX_RHO, name, "deployId!(uri)")?;
            // The class registration shares its block with a trailing test program (create an
            // instance, register its send capability, send test messages). The cut keeps the
            // registration and the `for` that reports it; the demo prints that precede it go too.
            let source = cut_from(&source, " |\n    lookup!(uri, *lookupCh) |")?;
            Ok(source.replace(
                "  stdout!(\"hello world\") |\n  stdout!([\"Unforgeable\", bundle+{*Inbox}]) |\n",
                "",
            ))
        }
        "roll" => {
            let source = publish_registration(MEMBER_ID_GOV_REV_RHO, name, "deployId!(uri)")?;
            // The dependencies are imported by URI, and those URIs are now constants.
            substitute_imports(&source)
        }
        "ballot" => {
            let source = publish_registration(BALLOT_RHO, name, "deployId!(uri)")?;
            // The tail drives four demo voters through a tally.
            cut_from(&source, "|\n  trace!(\"testing Ballot\")")
        }
        "chat" => {
            let source = publish_registration(CHAT_RHO, name, "stdout!([\"#define $Chat\", uri])")?;
            // The tail drives a listener through four messages.
            cut_from(&source, "} |\n    result!(\"testing\") |")
        }
        "group" => {
            let source = publish_registration(GROUP_RHO, name, "deployId!(uri)")?;
            // The tail creates two demo groups and looks them up.
            cut_from(&source, "} |\n  new return(`rho:io:stdout`)")
        }
        "memberDirectory" => {
            // The feature. Its registration epilogue ends with a demo `sendThem` to a hardcoded
            // address; the rest of the epilogue is what a client needs (it registers `GetMe` and
            // `SendThem` into the master directory, and publishes the *deployer's* inbox/dictionary).
            let source = cut_statement(
                MEMBER_DIRECTORY_RHO,
                "sendThem!([\"1111NkGJcLb9UdKg27bE1MXhaXwd2Sdhssn3i3EcWnZy11VLyW3zH\"",
            )?;
            Ok(load(&source))
        }
        // Not classes: the directory itself, then the slots upstream's template leaves out.
        "masterDirectory" => master_directory_template(),
        "extraSlots" => extra_directory_slots_source(),
        other => Err(format!("rgov: unknown vendored contract `{other}`")),
    }
}

/// The order the whole governance block installs in, after the class libraries: all eight classes
/// (each publishing its URI), then the master directory that references them, then the three slots
/// upstream's template omits, then the `GetMe` feature that a client's first call resolves.
pub fn governance_deploys(shard_id: &str) -> Result<Vec<(&'static str, SignedDeployData)>, String> {
    let mut out = deploys_named(shard_id)?;
    for name in ["masterDirectory", "extraSlots", "memberDirectory"] {
        out.push((name, deploy(name, shard_id)?));
    }
    Ok(out)
}

/// The publish channel as a `SortedProc`, for the callers that read it.
pub fn publish_channel() -> rchain_models::sorted::SortedProc {
    rchain_models::sorted::SortedProc::new(rchain_models::par_ops::from_expr(
        rchain_models::ast::Expr::GString(URI_PUBLISH_CHANNEL.to_string()),
    ))
}

/// Parse a published `["<name>", <uri>]` datum into its two parts.
pub fn published_uri(par: &rchain_models::ast::Par) -> Option<(String, String)> {
    use rchain_models::rholang::RhoType::{RhoList, RhoString, RhoUri};
    let items = RhoList::unapply(par)?;
    let name = RhoString::unapply(&items[0])?.to_string();
    let uri = RhoUri::unapply(&items[1])
        .or_else(|| RhoString::unapply(&items[1]))?
        .to_string();
    Some((name, uri))
}

/// Append the URI publish to a class registration's report, so the genesis seeder can copy the
/// registered entry onto the constant key. The marker is asserted and must be unique: a registration
/// that reports differently upstream must fail the build rather than silently stop publishing.
fn publish_registration(rho: &str, name: &str, marker: &str) -> Result<String, String> {
    let occurrences = rho.matches(marker).count();
    if occurrences != 1 {
        return Err(format!(
            "rgov: expected exactly one `{marker}` in {name}.rho, found {occurrences}"
        ));
    }
    Ok(load(&rho.replace(
        marker,
        &format!("{marker} | @\"{URI_PUBLISH_CHANNEL}\"!([\"{name}\", uri])"),
    )))
}

/// Remove a single trailing statement (and the `|` that joined it) from the end of the file.
///
/// Distinct from [`cut_from`], which truncates to the end: this drops one statement in the middle of
/// a block, keeping the block's structure. Used for a demo call that upstream runs last.
fn cut_statement(rho: &str, marker: &str) -> Result<String, String> {
    let occurrences = rho.matches(marker).count();
    if occurrences != 1 {
        return Err(format!(
            "rgov: expected exactly one `{marker}` statement, found {occurrences}"
        ));
    }
    let at = rho
        .find(marker)
        .ok_or_else(|| format!("rgov: `{marker}` vanished between the count and the find"))?;
    // Walk back over the `|` (and whitespace) that joined the statement to the previous one.
    let before = rho[..at].trim_end();
    let before = before.strip_suffix('|').unwrap_or(before).trim_end();
    let after = &rho[at..];
    let end = after.find('\n').map(|i| at + i + 1).unwrap_or(rho.len());
    Ok(format!("{before}\n{}", &rho[end..]))
}

/// Drop everything from `marker` to the end of the file, keeping the text before it — and close the
/// blocks the truncation left open.
///
/// Used for the deploy-time self-test programs upstream runs at the end of a file (see the NOTICE):
/// a chain's genesis must not send test traffic or print demo output. The kept prefix is usually
/// mid-block (the test program shares a `new … in { … }` with the registration it exercises), so
/// the brace balance of the prefix is appended rather than hand-counted here: the count is checked
/// by the tests that normalize every rendered source, and a wrong count fails the genesis build.
fn cut_from(source: &str, marker: &str) -> Result<String, String> {
    let at = source
        .find(marker)
        .ok_or_else(|| format!("rgov: marker `{marker}` not found for the self-test cut"))?;
    let kept = &source[..at];
    let open = kept.chars().filter(|c| *c == '{').count();
    let close = kept.chars().filter(|c| *c == '}').count();
    let mut out = kept.to_string();
    for _ in 0..open.saturating_sub(close) {
        out.push_str("\n}");
    }
    Ok(out)
}

/// Substitute the `match ("import", "./X.rho", \`rho:id:...\`)` markers with the installed
/// contracts' constants — this is what `bootstrap-rgov.ts` does at runtime by string replacement,
/// and doing it at build time is what makes the whole set's URIs stable.
fn substitute_imports(source: &str) -> Result<String, String> {
    let mut out = source.to_string();
    for dependency in ["directory", "inbox"] {
        let marker = format!("match (\"import\", \"./{dependency}.rho\", `rho:id:...`)");
        if !out.contains(&marker) {
            return Err(format!(
                "rgov: dependency marker for `{dependency}` not found"
            ));
        }
        let uri = contract_uri_for(dependency)?;
        out = out.replace(
            &marker,
            &format!("match (\"import\", \"./{dependency}.rho\", `{uri}`)"),
        );
    }
    Ok(out)
}

/// Append the loader comment the other blessed sources carry (`CompiledRholangSource.loadSource`).
fn load(source: &str) -> String {
    format!("{source}\n//Loaded from resource file <<rgov>>\n")
}

/// Build + sign the genesis deploy for one vendored contract, exactly as the standard deploys are
/// built (free, unbounded phlo, fixed key, shard-scoped).
pub fn deploy(name: &str, shard_id: &str) -> Result<SignedDeployData, String> {
    let data = DeployData {
        attachments: Vec::new(),
        term: source(name)?,
        timestamp: RGOV_TIMESTAMP,
        phlo_price: 0,
        phlo_limit: PHLO_LIMIT,
        valid_after_block_number: 0,
        shard_id: shard_id.to_string(),
    };
    let sk = PrivateKey::new(base16::unsafe_decode(&key_for(name)));
    let signed = Signed::new(data, &Secp256k1, &sk).map_err(|e| e.to_string())?;
    Ok(SignedDeployData {
        data: signed.data,
        deployer: signed.pk.bytes().to_vec(),
        sig: signed.sig,
        sig_algorithm: signed.sig_algorithm.name().to_string(),
    })
}

/// The private key a given term is deployed with: the shared dummy testnet key for the governance
/// trio, its own for a class.
fn key_for(name: &str) -> String {
    if shares_testnet_governance_key(name) {
        testnet_governance_key()
    } else {
        contract_key(name)
    }
}

/// The vendored set in install order.
pub fn deploys(shard_id: &str) -> Result<Vec<SignedDeployData>, String> {
    Ok(deploys_named(shard_id)?
        .into_iter()
        .map(|(_, deploy)| deploy)
        .collect())
}

/// The vendored set with its manifest names, in install order — so the genesis ordering check can
/// name the contracts it orders (`BLESSED_DEPENDENCIES`).
pub fn deploys_named(shard_id: &str) -> Result<Vec<(&'static str, SignedDeployData)>, String> {
    RGOV_CORE
        .iter()
        .map(|name| deploy(name, shard_id).map(|deploy| (*name, deploy)))
        .collect()
}

/// The `rho:id` of every vendored contract, in install order — the constants a consumer (or the
/// master directory template) hardcodes.
pub fn contract_uris() -> Result<Vec<(&'static str, String)>, String> {
    RGOV_CORE
        .iter()
        .map(|name| contract_uri_for(name).map(|uri| (*name, uri)))
        .collect()
}

/// `build_uri` over the deployer key (the `insertSigned` derivation). Kept because the *class* keys
/// are no longer derived this way — see [`contract_uri_for`] — and a reader comparing the two
/// derivations should be able to see both.
pub fn derive_uri(private_key_hex: &str) -> Result<String, String> {
    let sk = PrivateKey::new(base16::unsafe_decode(private_key_hex));
    let pk = Secp256k1.to_public(&sk).map_err(|e| e.to_string())?;
    Ok(build_uri(&blake2b256(pk.bytes())))
}

/// The master-directory template with the member URIs substituted for the installed constants.
///
/// Upstream's template carries the seven member URIs as literals at the top (as recorded from a
/// *testnet* deployment) — those are exactly the values that go stale, which is what this whole
/// module removes. Substituting here rather than at deploy time means a client can deploy the
/// returned term verbatim and get a master directory whose members are the same on every chain of
/// this port.
///
/// The member order is upstream's: `directory`, `echo`, `inbox`, `issue`, `kudos`, `roll`, `log`.
/// `Echo.rho` and `mq.rho` (the `log` slot) never call `insertArbitrary` — they define classes and
/// publish no URI — so their slots reuse the `Directory` URI, exactly as `bootstrap-rgov.ts` does
/// with its `SLOT_ALIASES`.
pub fn master_directory_template() -> Result<String, String> {
    // The literals upstream shipped, in the template's own order (asserted below).
    let recorded: [&str; 7] = [
        "rho:id:o9b5otixodhpkxgtbsz1ja5sak43gdhei69ukc9swp355qi8dkm3n7",
        "rho:id:yupw4m3mfjn9smxtsja5igqqdxyqpnjzjg16aqzdwhzcbqkhxz8f5n",
        "rho:id:fqfifaqpwp9o4joyybmg9w8iiczfcyq8f66br9zmg8fqompigccgju",
        "rho:id:urj8148w4ufm8mw8kgz6kddb97bccezk98won98aw11coxdnwn6sr1",
        "rho:id:eifmzammsbx8gg5fjghjn34pw6hbi6hqep7gyk4bei96nmra11m4hi",
        "rho:id:kiijxigqydnt7ds3w6w3ijdszswfysr3hpspthuyxz4yn3ksn4ckzf",
        "rho:id:fbjgow69qme33wk9jwhbjd8ofy36w7gjyup6gfc5d3tsfwfq8s4144",
    ];
    let directory = contract_uri_for("directory")?;
    let installed: [String; 7] = [
        directory.clone(),
        directory.clone(),
        contract_uri_for("inbox")?,
        contract_uri_for("issue")?,
        contract_uri_for("kudos")?,
        contract_uri_for("roll")?,
        directory,
    ];
    let mut out = MASTER_DIRECTORY_RHO.to_string();
    for (recorded_uri, installed_uri) in recorded.iter().zip(&installed) {
        if !out.contains(recorded_uri) {
            return Err(format!(
                "rgov: the master-directory template no longer carries {recorded_uri}"
            ));
        }
        out = out.replace(recorded_uri, installed_uri);
    }
    // The read cap this deploy mints is the `ReadcapURI` a governance client hardcodes, and the
    // template announces it only on `stdout` and the deploy-scoped `deployId`. Publish it so genesis
    // can copy it onto `readcap_uri()`.
    let announced =
        "stdout!({ \"ReadcapURI\": *URI})\n               | deployId!({ \"ReadcapURI\": *URI })";
    if !out.contains(announced) {
        return Err("rgov: the template's ReadcapURI announcement moved".into());
    }
    out = out.replace(
        announced,
        &format!("{announced}\n               | @\"{URI_PUBLISH_CHANNEL}\"!([\"readcap\", *URI])"),
    );
    Ok(load(&out))
}

/// The three class slots the wallet's editor asks for that upstream's template does not fill.
///
/// Upstream's template hardcodes seven member slots; the wallet's snippets look `Chat`, `Ballot` and
/// `Group` up in the directory, and a slot that was never written answers `Nil` — which a client
/// cannot tell from "broken" (`spec/GENESIS.md`). Rather than rewrite upstream's seven-slot body,
/// this is **our own** term, authored here so it is auditable as ours: it takes the master
/// directory's write capability that the template published and writes the three classes in.
pub fn extra_directory_slots_source() -> Result<String, String> {
    let (chat, ballot, group) = (
        contract_uri_for("chat")?,
        contract_uri_for("ballot")?,
        contract_uri_for("group")?,
    );
    Ok(load(&format!(
        r#"new
   deployerId(`rho:rchain:deployerId`),
   lookup(`rho:registry:lookup`)
in {{
   for (@{{"write": *MCAwrite, ..._}} <<- @[*deployerId, "MasterContractAdmin"]) {{ Nil
   |  new chatCh, ballotCh, groupCh
      in {{
         lookup!(`{chat}`, *chatCh) |
         lookup!(`{ballot}`, *ballotCh) |
         lookup!(`{group}`, *groupCh) |
         for (C_Chat <- chatCh) {{ MCAwrite!("Chat", *C_Chat) }} |
         for (C_Ballot <- ballotCh) {{ MCAwrite!("Ballot", *C_Ballot) }} |
         for (C_Group <- groupCh) {{ MCAwrite!("Group", *C_Group) }}
      }}
   }}
}}
"#
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The public key (the `deployerId`) a term's deploy carries.
    fn deploy_public_key(name: &str) -> Vec<u8> {
        let sk = PrivateKey::new(base16::unsafe_decode(&key_for(name)));
        Secp256k1
            .to_public(&sk)
            .unwrap_or_else(|e| panic!("a derived key must be valid: {e}"))
            .bytes()
            .to_vec()
    }

    /// Normalizing a blessed term needs the node's 32 MiB stack (these contracts recurse past the
    /// 2 MiB test default), matching `node/src/main.rs` and `node/tests/common/mod.rs`.
    fn normalizes(term: &str) -> bool {
        let term = term.to_string();
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(move || rchain_rholang::normalizer::source_to_adt(&term).is_ok())
            .expect("spawn")
            .join()
            .expect("join")
    }

    /// Every rendered term, classes and the three governance terms alike, parses and normalizes —
    /// the patches edit text and one of the terms is ours, so this is what catches an unbalanced or
    /// malformed adaptation.
    #[test]
    fn every_rendered_contract_parses_and_normalizes() {
        for name in RGOV_CORE
            .iter()
            .chain(&["masterDirectory", "extraSlots", "memberDirectory"])
        {
            let term = source(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                normalizes(&term),
                "{name}: the adapted term must parse and normalize"
            );
        }
    }

    /// The class registration keeps **upstream's shape** — `insertArbitrary`, whose stored value is
    /// the bare class — and every class publishes the URI it was given. This is the regression that
    /// matters: converting the registration to `insertSigned` stores `(nonce, value)`, and the
    /// master-directory template (and `memberIdGovRev`) destructure the bare value, so the template
    /// stalls silently.
    #[test]
    fn the_class_registration_keeps_upstreams_shape() {
        for name in RGOV_CORE {
            let term = source(name).unwrap();
            assert!(
                term.contains("insertArbitrary!("),
                "{name}: the class registration stays upstream's"
            );
            assert!(
                !term.contains("insertSigned!"),
                "{name}: a signed registration would store a `(nonce, value)` tuple, which the \
                 rgov consumers cannot destructure"
            );
            assert!(
                term.contains(&format!("@\"{URI_PUBLISH_CHANNEL}\"!([\"{name}\", uri])")),
                "{name}: the class must publish its URI for genesis to copy onto the constant key"
            );
        }
    }

    /// The three governance terms are signed by **one** key, and the classes are not: the template
    /// publishes its `MasterContractAdmin` capability for its own deployer and the feature's
    /// registration is gated on reading it back, so a mismatch there leaves `GetMe` unregistered and
    /// answering `Nil` — a stall with no error anywhere (verified on a node).
    #[test]
    fn the_governance_terms_share_one_key() {
        let shared: Vec<String> = ["masterDirectory", "extraSlots", "memberDirectory"]
            .iter()
            .map(|name| base16::encode(&deploy_public_key(name)))
            .collect();
        assert_eq!(shared[0], shared[1], "template and extra slots");
        assert_eq!(shared[1], shared[2], "extra slots and feature");
        let kudos = base16::encode(&deploy_public_key("kudos"));
        assert_ne!(kudos, shared[0], "a class keeps its own key");
    }

    /// The keys genesis publishes, pinned. A change here is a genesis change and moves what a client
    /// hardcodes, so it must be reflected in `spec/GENESIS.md`.
    #[test]
    fn the_published_keys_are_constants() {
        let expected: &[(&str, &str)] = &[
            ("kudos", "PIN"),
            ("inbox", "PIN"),
            ("directory", "PIN"),
            ("roll", "PIN"),
            ("issue", "PIN"),
            ("ballot", "PIN"),
            ("chat", "PIN"),
            ("group", "PIN"),
        ];
        let uris = contract_uris().unwrap();
        assert_eq!(uris.len(), expected.len());
        for ((name, uri), (expected_name, expected_uri)) in uris.iter().zip(expected) {
            assert_eq!(name, expected_name);
            if *expected_uri == "PIN" {
                println!("{name} -> {uri}");
            } else {
                assert_eq!(uri, expected_uri, "{name}");
            }
            // Derived from a named string, so a reader can recompute it.
            assert!(uri.starts_with("rho:id:"), "{uri}");
        }
        let readcap = readcap_uri().unwrap();
        println!("readcap -> {readcap}");
        assert!(readcap.starts_with("rho:id:"));
    }

    /// `memberIdGovRev` imports its dependencies by URI; the install order guarantees those exist,
    /// and the substitution is what makes the whole set's URIs stable across chains.
    #[test]
    fn the_member_directory_imports_the_installed_contracts() {
        let term = source("roll").unwrap();
        for dependency in ["directory", "inbox"] {
            let uri = contract_uri_for(dependency).unwrap();
            assert!(
                term.contains(&format!(
                    "match (\"import\", \"./{dependency}.rho\", `{uri}`)"
                )),
                "roll must import {dependency} by its installed URI"
            );
        }
        assert!(
            !term.contains("rho:id:..."),
            "no unresolved import placeholder may remain"
        );
    }

    /// The master-directory template ships with the installed URIs, and none of the recorded
    /// testnet URIs survive — those are the stale values the vendoring exists to remove.
    #[test]
    fn the_master_directory_template_carries_the_installed_uris() {
        let term = master_directory_template().unwrap();
        for name in ["directory", "inbox", "issue", "kudos", "roll"] {
            let uri = contract_uri_for(name).unwrap();
            assert!(term.contains(&uri), "the template must name the {name} URI");
        }
        assert!(
            !term.contains("o9b5otixodhpkxgtbsz1ja5sak43gdhei69ukc9swp355qi8dkm3n7"),
            "no recorded testnet URI may survive"
        );
        assert!(
            normalizes(&term),
            "the substituted template must parse and normalize"
        );
    }

    /// The self-test traffic upstream runs at deploy time is gone: genesis must not send test
    /// messages or print demo output.
    #[test]
    fn the_deploy_time_self_tests_are_removed() {
        let inbox = source("inbox").unwrap();
        assert!(!inbox.contains("hello world"));
        assert!(!inbox.contains("\"values\",\"test\""));
        assert!(!source("directory").unwrap().contains("got capabilities"));
        assert!(!source("ballot").unwrap().contains("testing Ballot"));
        assert!(!source("group").unwrap().contains("got em"));
        // The feature's epilogue keeps its registration and drops its demo send.
        let feature = source("memberDirectory").unwrap();
        assert!(feature.contains("MCAwrite!(\"GetMe\""));
        assert!(
            !feature.contains("sendThem!([\"1111NkGJcLb9UdKg27bE1MXhaXwd2Sdhssn3i3EcWnZy11VLyW3zH")
        );
    }

    /// The three slots upstream's template omits are filled by our own term — the wallet's editor
    /// asks the directory for these names, and an unwritten slot answers `Nil`.
    #[test]
    fn the_extra_slots_term_writes_the_names_the_wallet_asks_for() {
        let term = source("extraSlots").unwrap();
        for (name, key) in [("Chat", "chat"), ("Ballot", "ballot"), ("Group", "group")] {
            assert!(
                term.contains(&format!("MCAwrite!(\"{name}\"")),
                "the directory needs a {name} slot"
            );
            assert!(
                term.contains(&contract_uri_for(key).unwrap()),
                "{name} must be looked up by its published key"
            );
        }
        // It writes through the capability the master directory published, not a hardcoded one.
        assert!(term.contains("@[*deployerId, \"MasterContractAdmin\"]"));
    }
}
