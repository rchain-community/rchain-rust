//! The rgov governance class contracts, vendored as genesis content.
//!
//! Upstream (`rchain-community/rgov`, see `resources/rgov/NOTICE` for the commit and the licence
//! position) deploys these at *runtime* with `scripts/bootstrap-rgov.ts`: a funded key deploys the
//! set in dependency order and records the resulting URIs, which then have to be fed back into the
//! dependents and into the master directory. The recorded master URI is a product of that
//! deployment, so it shifts on every chain and goes stale silently — the problem this module
//! removes for the class contracts by installing them at genesis with **fixed keys**, so their URIs
//! are constants (see `spec/GENESIS.md`).
//!
//! Two classes of registration live in these files and only one of them belongs in genesis:
//!
//! - the **class** registration (Kudos, Inbox, Directory, the member directory, Issue) — installed
//!   here, converted to `rho:registry:insertSigned:secp256k1` so the URI derives from the fixed
//!   deployer key instead of a random seed;
//! - **runtime** registrations (a member directory's own write capability, an inbox instance's send
//!   capability) — left as `insertArbitrary`, because they are per-deployer state, not classes.
//!
//! Everything this module changes is asserted against the vendored text: a drift upstream fails the
//! genesis build rather than shipping a half-adapted contract.

use rchain_crypto::hash::blake2b256::hash as blake2b256;
use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::signatures::secp256k1::Secp256k1;
use rchain_crypto::signatures::signatures_alg::SignaturesAlg;
use rchain_crypto::signatures::signed::Signed;
use rchain_models::casper::protocol::casper_message::{DeployData, SignedDeployData};
use rchain_rholang::registry::build_uri;
use rchain_rholang::system_processes::SYSTEM_CONTRACT_NONCE;
use rchain_shared::base16;

use crate::genesis::standard_deploys::contract_uri;

const KUDOS_RHO: &str = include_str!("resources/rgov/Kudos.rho");
const INBOX_RHO: &str = include_str!("resources/rgov/Inbox.rho");
const DIRECTORY_RHO: &str = include_str!("resources/rgov/Directory.rho");
const MEMBER_ID_GOV_REV_RHO: &str = include_str!("resources/rgov/memberIdGovRev.rho");
const ISSUE_RHO: &str = include_str!("resources/rgov/Issue.rho");
const MASTER_DIRECTORY_RHO: &str =
    include_str!("resources/rgov/create-master-contract-directory-testnet.rho");

/// `accounting.MAX_VALUE` — the nonce every system contract registers with (the element consumers
/// destructure and ignore: `for (@(_, X) <- ch)`).
const PHLO_LIMIT: i64 = i32::MAX as i64;

/// The upstream deployment order (`bootstrap-rgov.ts`'s `STEPS`, from the original `Jakefile.js`):
/// `memberIdGovRev` imports `directory.rho` and `inbox.rho`, so those must be installed first.
pub const RGOV_CORE: &[&str] = &["kudos", "inbox", "directory", "roll", "issue"];

/// The fixed private key for a vendored contract, *derived* rather than pasted: a hardcoded hex
/// constant would be unauditable (nobody can check it was not chosen adversarially), whereas a named
/// hash can be recomputed by anyone reading this file. It is the same value on every chain, which is
/// what makes the resulting `rho:id` a constant.
pub fn contract_key(name: &str) -> String {
    base16::encode(&blake2b256(format!("rnode/genesis/rgov/{name}").as_bytes()))
}

/// The fixed timestamp for a vendored contract. Genesis deploys carry no real time; a constant makes
/// the deploy (and therefore its terms' ordering) reproducible. `spec/GENESIS.md` records the value.
const RGOV_TIMESTAMP: i64 = 1_700_000_000_000;

/// The `rho:id` each vendored contract registers itself under — a constant, because
/// `insertSigned:secp256k1` derives it from the deployer key, and that key is [`contract_key`].
pub fn contract_uri_for(name: &str) -> Result<String, String> {
    contract_uri(&contract_key(name))
}

/// Convert a **class** self-registration from `insertArbitrary` (whose URI is a hash of a random
/// seed) to `insertSigned` (whose URI is a hash of the deployer key). `value` is the registered
/// term as it appears in the vendored source (`*Kudos`, `bundle+{*Issue}`, …) and `channel` the
/// reply channel; whitespace around the argument is tolerated, but a missing call or a different
/// channel is an error — that is the drift alarm.
fn sign_registration(
    source: &str,
    registered: &str,
    inner: &str,
    channel: &str,
) -> Result<String, String> {
    let call = format!("insertArbitrary!({registered},");
    let start = source
        .find(&call)
        .ok_or_else(|| format!("rgov: no `{call}` registration to convert"))?;
    let after = &source[start + call.len()..];
    let trimmed = after.trim_start();
    let expected = format!("*{channel})");
    if !trimmed.starts_with(&expected) {
        return Err(format!(
            "rgov: the registration for `{registered}` does not pass `{expected}`"
        ));
    }
    let end = source.len() - trimmed.len() + expected.len();
    // `inner` is the contract name *without* its bundle: upstream registers either the bare name or
    // `bundle+{*name}`, and genesis always stores the bundle (`(nonce, bundle+{dispatcher})`), so
    // passing the inner form keeps the two call styles from double-wrapping.
    let replacement = format!(
        "insertSigned!(({SYSTEM_CONTRACT_NONCE}, bundle+{{{inner}}}), *deployerId, *{channel})"
    );
    Ok(format!(
        "{}{}{}",
        &source[..start],
        replacement,
        &source[end..]
    ))
}

/// Add the `insertSigned` binding beside the `insertArbitrary` one (a contract that keeps runtime
/// registrations needs both) and bind `deployerId` when the file does not already.
fn add_signed_binding(source: &str, needs_deployer_id: bool) -> Result<String, String> {
    let from = "insertArbitrary(`rho:registry:insertArbitrary`)";
    let to = "insertArbitrary(`rho:registry:insertArbitrary`), insertSigned(`rho:registry:insertSigned:secp256k1`)";
    if !source.contains(from) {
        return Err("rgov: no `insertArbitrary` binding to extend".to_string());
    }
    let mut out = source.replace(from, to);
    if needs_deployer_id && !out.contains("deployerId(`rho:rchain:deployerId`)") {
        out = out.replacen(
            to,
            "insertSigned(`rho:registry:insertSigned:secp256k1`), deployerId(`rho:rchain:deployerId`)",
            1,
        );
        out = out.replacen(from, "insertArbitrary(`rho:registry:insertArbitrary`), ", 1);
    }
    Ok(out)
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

/// The genesis term for one vendored contract, adapted as `spec/GENESIS.md` and the NOTICE describe.
pub fn source(name: &str) -> Result<String, String> {
    match name {
        "kudos" => {
            let source = add_signed_binding(KUDOS_RHO, true)?;
            let source = sign_registration(&source, "*Kudos", "*Kudos", "regCh")?;
            Ok(load(&source))
        }
        "issue" => {
            let source = add_signed_binding(ISSUE_RHO, true)?;
            let source = sign_registration(&source, "bundle+{*Issue}", "*Issue", "uriCh")?;
            Ok(load(&source))
        }
        "directory" => {
            let source = add_signed_binding(DIRECTORY_RHO, true)?;
            let source = sign_registration(&source, "bundle+{*directory}", "*directory", "uriCh")?;
            // The file's tail exercises the contract it just registered.
            let source = cut_from(&source, "} |\n    directory!(Nil, *ret)")?;
            Ok(load(&source))
        }
        "inbox" => {
            let source = add_signed_binding(INBOX_RHO, true)?;
            let source = sign_registration(&source, "bundle+{*Inbox}", "*Inbox", "creationCh")?;
            // The class registration shares its block with a trailing test program (create an
            // instance, register its send capability, send test messages). The cut keeps the
            // registration and the `for` that reports it, and drops the exercise — the two
            // positions differ by a handful of lines, so the marker is the *last* line of the
            // registration path.
            // The demo prints that precede the registration (the "hello world" and
            // "Unforgeable" banners) go too: they are the head of the same test program.
            let source = cut_from(&source, " |\n    lookup!(uri, *lookupCh) |")?;
            let source = source.replace(
                "  stdout!(\"hello world\") |\n  stdout!([\"Unforgeable\", bundle+{*Inbox}]) |\n",
                "",
            );
            Ok(load(&source))
        }
        "roll" => {
            let source = add_signed_binding(MEMBER_ID_GOV_REV_RHO, false)?;
            let source =
                sign_registration(&source, "*MemberDirectory", "*MemberDirectory", "regCh")?;
            // The dependencies are imported by URI, and those URIs are now constants.
            let source = substitute_imports(&source)?;
            Ok(load(&source))
        }
        other => Err(format!("rgov: unknown vendored contract `{other}`")),
    }
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
    let sk = PrivateKey::new(base16::unsafe_decode(&contract_key(name)));
    let signed = Signed::new(data, &Secp256k1, &sk).map_err(|e| e.to_string())?;
    Ok(SignedDeployData {
        data: signed.data,
        deployer: signed.pk.bytes().to_vec(),
        sig: signed.sig,
        sig_algorithm: signed.sig_algorithm.name().to_string(),
    })
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

/// `build_uri` over the deployer key, re-exported so a test can assert the derivation is the one
/// `insertSigned` performs (rather than a second implementation of it drifting).
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
    Ok(load(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Every vendored contract, as installed, parses and normalizes — the patch edits text, so this
    /// is what catches an adaptation that leaves the source unbalanced.
    #[test]
    fn every_rendered_contract_parses_and_normalizes() {
        for name in RGOV_CORE {
            let term = source(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(
                normalizes(&term),
                "{name}: the adapted term must parse and normalize"
            );
        }
    }

    /// The **class** registration is the signed one — a random-seed URI in a class is what would
    /// make the chain's registry non-reproducible — while a contract's *runtime* registrations
    /// (a member directory's own write capability) legitimately stay `insertArbitrary`: those are
    /// per-deployer state, and their URIs are supposed to be products of the deployment.
    #[test]
    fn the_class_registration_is_the_signed_one() {
        let classes: &[(&str, &str, &str)] = &[
            ("kudos", "*Kudos", "regCh"),
            ("inbox", "*Inbox", "creationCh"),
            ("directory", "*directory", "uriCh"),
            ("roll", "*MemberDirectory", "regCh"),
            ("issue", "*Issue", "uriCh"),
        ];
        for (name, value, channel) in classes {
            let term = source(name).unwrap();
            assert!(
                term.contains(&format!(
                    "insertSigned!(({SYSTEM_CONTRACT_NONCE}, bundle+{{{value}}}), *deployerId, *{channel})"
                )),
                "{name}: the class registration must be the signed one"
            );
            assert!(
                !term.contains(&format!("insertArbitrary!({value},")),
                "{name}: the class must not be registered with insertArbitrary"
            );
        }
        // `roll` keeps its runtime registrations, which is the reason the conversion above is not a
        // blanket replacement of the binding.
        assert!(source("roll").unwrap().contains("insertArbitrary!("));
    }

    /// The URIs are constants of fixed keys — the property that makes them hardcodable, and the
    /// reason these are installed at genesis instead of deployed per chain. Pinned: a change here is
    /// a genesis change, and `spec/GENESIS.md` publishes these values.
    #[test]
    fn the_uris_are_constants() {
        let expected: &[(&str, &str)] = &[
            (
                "kudos",
                "rho:id:c35xabt84irokn3kp7qh9gs31f58rzje8ntifucmg19s1q6gek8y",
            ),
            (
                "inbox",
                "rho:id:8qbr8guigfush1n8y64ubkjnakwrh9m3suty68pkoebgbjwuiuio",
            ),
            (
                "directory",
                "rho:id:xp7ih4n3kghz89kkc3smgighxs1ou8wx6hrt54z9smr55tfd1i3o",
            ),
            (
                "roll",
                "rho:id:j9wmz843xzfporr7qnghzsj46doxctpxg4msw9tjtrqex1smngjy",
            ),
            (
                "issue",
                "rho:id:jne1e6mptyjp96zrak8reb4s8hmw3fonrkkkaxbiok18xxs1oj1o",
            ),
        ];
        let uris = contract_uris().unwrap();
        assert_eq!(uris.len(), expected.len());
        for ((name, uri), (expected_name, expected_uri)) in uris.iter().zip(expected) {
            assert_eq!(name, expected_name);
            assert_eq!(uri, expected_uri, "{name}",);
            // The URI is the deployer-key derivation `insertSigned` performs, not a second
            // implementation of it that could drift.
            assert_eq!(
                &derive_uri(&contract_key(name)).unwrap(),
                uri,
                "{name}: the URI must be the deployer-key derivation"
            );
        }
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
    }
}
