//! The legacy `.rho` corpus: the first contact between the Rust parser/reducer and real programs.
//!
//! Everything the Rust side has been tested against so far — inline strings, the `qucalc`/`examples`
//! set, the `rholang/tests` fixtures — was written *for* this port. The 165 `.rho` files under
//! `legacy/` are the programs the Scala node actually ran: its tutorials, its linking-package
//! examples, its integration-test fixtures and the resources shipped in `casper`. They exercise
//! syntax nobody wrote a Rust test for, and a parse or reduce failure here is a real port bug.
//!
//! The contract:
//!
//! 1. **Every file is accounted for.** `parsed + skipped == 165`, where the 165 is also checked
//!    against the walk itself, so a corpus file that disappears fails the test rather than quietly
//!    shrinking the denominator.
//! 2. **A skip is a decision with a reason**, from a closed enum, keyed by path — not a filter
//!    regex. Every skip entry must name a file that exists *and* was found by the walk, so a stale
//!    entry (a file that was renamed or deleted) fails instead of silently allowing nothing.
//! 3. **Each program gets its own runtime.** Reusing one would let program N match a message
//!    program N-1 left behind, so a "pass" could be an artefact of the corpus order — and the
//!    order is the filesystem's.
//! 4. **The reduction is bounded.** `set_max_reduce_steps` caps each program, and a wall-clock
//!    timeout backstops it, so a program that never terminates is reported by name rather than
//!    hanging the suite.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_crypto::public_key::PublicKey;
use rchain_models::normalizer_env::NormalizerEnv;

mod common;

/// The corpus size, asserted literally: the register records it, and a change here is a change to
/// the corpus (a file added or removed in `legacy/`), which should be a deliberate diff.
const CORPUS: usize = 165;

/// Reduction-step budget per program. The reducer's own default is `DEFAULT_MAX_REDUCE_STEPS`
/// (100 000); this test names its own bound so that "the corpus needs more fuel than the default"
/// is a visible decision rather than a silent one.
const FUEL: i64 = 100_000;

/// Wall-clock backstop per program, so a non-terminating reduction is a named failure.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Why a corpus file is not reduced. Closed on purpose: a new reason is a code change, and the
/// counts are printed per reason so a growing bucket cannot hide behind a single total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkipReason {
    /// A different language. The file is written in a dialect the current grammar does not have:
    /// the `export`/`import` linking packages (`examples/linking/`, a superseded prototype), the
    /// pre-0.12 `examples/old/` syntax (`print(x)` with no `!`, `#` comments, `for (rtn; …)`), or
    /// the K-framework fixtures under `src/main/k/rholang/tests/` (written for the K semantics,
    /// where `match` takes a process *pattern*, and not for a node). The Scala node rejects these
    /// too — `legacy/rholang/src/main/bnfc/rholang_mercury.cf` has none of these productions — so
    /// this is a classification, not port debt.
    SupersededSyntax,
    /// A template, not a program: a placeholder (`$$name$$` — Scala's `s"…"` escape — `%NAME`, or
    /// `@name@`) is substituted by a Scala-side string operation before the term reaches a node.
    MacroTemplate,
    /// Needs a native that only the Scala *test harness* provides: `rho:io:stdlog`,
    /// `rho:rchain:deployId`, the `rho:test:*` assertions, `rho:registry:insertArbitrary2`. The
    /// node's own system processes are installed by the harness of both implementations; these are
    /// not among them, so the `BugFoundError` is the expected outcome rather than a missing native.
    ScalaOnlyNative,
    /// The file's purpose is to fail, or it needs a context this harness does not build (a live
    /// registry, an already-deployed contract, a real REV address). Its rejection is the expected
    /// outcome, so "it did not reduce" carries no signal.
    ExpectedRuntimeError,
    /// A deliberate stress or benchmark workload whose reduction exceeds the test's fuel bound.
    ExceedsTestBudget,
}

impl SkipReason {
    fn as_str(self) -> &'static str {
        match self {
            SkipReason::SupersededSyntax => "superseded syntax",
            SkipReason::MacroTemplate => "macro template",
            SkipReason::ScalaOnlyNative => "scala-only native",
            SkipReason::ExpectedRuntimeError => "expected runtime error",
            SkipReason::ExceedsTestBudget => "exceeds test budget",
        }
    }

    /// Every variant, so a new one cannot be added without the per-reason report gaining a line.
    const ALL: &'static [SkipReason] = &[
        SkipReason::SupersededSyntax,
        SkipReason::MacroTemplate,
        SkipReason::ScalaOnlyNative,
        SkipReason::ExpectedRuntimeError,
        SkipReason::ExceedsTestBudget,
    ];
}

/// The skipped files, keyed by path relative to the repository root, with the reason each is not
/// reduced. Written *from what the reducer reported* (90 files out of 165), not from what looked
/// hard: 56 files are in a dialect the grammar does not have, 17 need a test-harness native, 8 are
/// templates, 4 are meant to fail or need a live context, and 5 exceed the fuel bound.
///
/// The remaining **75 reduce cleanly**. Getting there fixed four real defects in the port — a
/// parenthesised expression was parsed as a one-element tuple (`Registry.rho` could not be reduced
/// at all), the two logical connectives were swapped at the lexer and disjunction was unparseable,
/// `++` was missing its Map/Set arms, and `+`/`-` were missing theirs. See `spec/AUDIT.md` §16.
const SKIPS: &[(&str, SkipReason)] = &[
    // --- a dialect the grammar does not have (57) ---
    ("legacy/rholang/examples/linking/v0.1/LinkedArrayAndMapExample.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/packages/BalanceMap.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/packages/IArrayApi.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/packages/IMapApi.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/packages/LinkedListApi.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/packages/MakeBrandPair.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/packages/MakeMint.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/packages/NonNegativeNumber.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/packages/TestSet.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/packages/X.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/packages/Y.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/tests/BalanceMap_test.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/tests/IArray_test.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/tests/IMap_test.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/tests/LeftBrace_test.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/tests/LinkedList_test.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/tests/MakeMint_test.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/tests/NonNegativeNumber_test.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/tests/RightBrace_test.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/tests/SealUnsealTest.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.1/tests/XY_test.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/packages/LinkedList.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/packages/MakeBrandPair.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/packages/MakeMint.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/packages/NamedFields.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/packages/NonNegativeNumber.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/packages/RhoClass.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/packages/TestSet.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/tests/LinkedListTest.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/tests/MakeBrandPairTest.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/tests/MakeMintTest.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/tests/NamedFieldsTest.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/linking/v0.2/tests/NonNegativeNumberTest.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/HelloWorld.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/ListProcTest.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/MethodTest.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/auction_end.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/log_time.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/regular/Cell1.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/regular/Cell2.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/regular/Cell3.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/regular/token.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/sugar/Cell1.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/sugar/Cell2.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/examples/old/sugar/Cell3.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Free-Variables/Finding-free-vars-with-logical-and-between processes.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Free-Variables/Finding-free-vars-with-logical-and-between-channels.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Free-Variables/Finding-free-vars-with-logical-or-between processes.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Free-Variables/Finding-free-vars-with-logical-or-between-channels.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Global-Program-Structure/Free-variable-in-program.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Global-Program-Structure/Other-instances-of-HigherProc.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Sending-Receiving/infinite-sends-and-receives.rho", SkipReason::ExceedsTestBudget),
    ("legacy/rholang/src/main/k/rholang/tests/Sending-Receiving/logical-and-within-patterns.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Std-Pattern-Matching/match-within-a-match.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Wildcards-vs-Variables/Wildcard-matching-2.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Wildcards-vs-Variables/Wildcard-matching.rho", SkipReason::SupersededSyntax),
    ("legacy/rholang/src/main/k/rholang/tests/Wildcards-vs-Variables/Wildcard-placement.rho", SkipReason::SupersededSyntax),
    // --- a native only the Scala test harness provides (17) ---
    ("legacy/casper/src/main/resources/RegistryRealLifeTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/AuthKeyTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/BlockDataContractTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/EitherTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/FailingResultCollectorTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/MakeMintTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/MultiSigRevVaultTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/PosTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/RegistryTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/RevAddressTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/RevVaultTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/RhoSpecContract.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/RhoSpecContractTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/TreeHashMapTest.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/UpdateAuthKey/UpdateAuthKey.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/UpdatePos/UpdatePos.rho", SkipReason::ScalaOnlyNative),
    ("legacy/casper/src/test/resources/UpdateRegistry/UpdateRegistry.rho", SkipReason::ScalaOnlyNative),
    // --- a template, substituted before deployment (8) ---
    ("legacy/casper/src/test/resources/MultiSigVault/PosMultiSigConfirm.rho", SkipReason::MacroTemplate),
    ("legacy/casper/src/test/resources/MultiSigVault/PosMultiSigTransfer.rho", SkipReason::MacroTemplate),
    ("legacy/casper/src/test/resources/MultiSigVault/TransferToPosMultiSig.rho", SkipReason::MacroTemplate),
    ("legacy/integration-tests/resources/storage/read-data.rho", SkipReason::MacroTemplate),
    ("legacy/integration-tests/resources/wallets/bond.rho", SkipReason::MacroTemplate),
    ("legacy/integration-tests/resources/wallets/transfer_funds.rho", SkipReason::MacroTemplate),
    ("legacy/integration-tests/resources/wallets/transfer_from_pos_vault.rho", SkipReason::MacroTemplate),
    ("legacy/rholang/examples/vault_demo/1.know_ones_revaddress.rho", SkipReason::MacroTemplate),
    // --- meant to fail, or needs a live context (4) ---
    ("legacy/integration-tests/resources/invalid.rho", SkipReason::ExpectedRuntimeError),
    ("legacy/rholang/examples/tut-strings-methods.rho", SkipReason::ExpectedRuntimeError),
    ("legacy/rholang/src/main/k/rholang/tests/Sending-Receiving/logical-or-within-patterns.rho", SkipReason::ExpectedRuntimeError),
    ("legacy/rholang/src/test/resources/failure_tests/unbound_name_test.rho", SkipReason::ExpectedRuntimeError),
    // --- a deliberate stress workload, past the fuel bound (3) ---
    ("legacy/rholang/examples/longslow.rho", SkipReason::ExceedsTestBudget),
    ("legacy/rholang/examples/performance/loop_recursive.rho", SkipReason::ExceedsTestBudget),
    ("legacy/rholang/examples/shortslow.rho", SkipReason::ExceedsTestBudget),
    ("legacy/rspace-bench/src/test/resources/rholang/wide-setup.rho", SkipReason::ExceedsTestBudget),
];

/// The repository root (`rholang/..`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate has a parent")
        .to_path_buf()
}

/// Every `.rho` file under `legacy/`, sorted, as paths relative to the repo root.
fn corpus() -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "rho") {
                out.push(
                    path.strip_prefix(root)
                        .expect("a walked path is under the root")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    let root = repo_root();
    let mut out = Vec::new();
    walk(&root.join("legacy"), &root, &mut out);
    out.sort();
    out
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn fixed_rand() -> Blake2b512Random {
    Blake2b512Random::from_init(&[0u8; 32])
}

/// A file's normalizer environment: the deployer binding only when the source actually mentions
/// `rho:rchain:deployerId`. This replaces the hand-maintained `NEEDS_DEPLOYER` list in
/// `rho_examples.rs` — a list that has to be edited whenever a file's *content* changes, and which
/// fails by *reducing without the binding* rather than by name.
fn env_for(source: &str) -> BTreeMap<String, rchain_models::ast::Par> {
    if source.contains("rho:rchain:deployerId") {
        NormalizerEnv::with_deployer_id(&PublicKey::new(vec![7u8; 65]))
            .to_env()
            .clone()
    } else {
        BTreeMap::new()
    }
}

/// The walk must find exactly the corpus the register records — a file added or deleted under
/// `legacy/` changes a number this test asserts, so it is a deliberate diff rather than a silent
/// change in what "the corpus" means.
#[test]
fn the_corpus_is_the_size_the_register_records() {
    let found = corpus();
    assert_eq!(
        found.len(),
        CORPUS,
        "found {} .rho files under legacy/; the register records {CORPUS}. First/last: {:?}/{:?}",
        found.len(),
        found.first(),
        found.last()
    );
}

/// Every skip names a file that exists **and** was found by the walk, and no file is both skipped
/// and expected to parse its way into a different bucket. A stale entry fails here.
#[test]
fn every_skip_entry_names_a_real_corpus_file() {
    let found = corpus();
    for (path, reason) in SKIPS {
        assert!(
            repo_root().join(path).is_file(),
            "skip entry {path} ({}) names a file that does not exist",
            reason.as_str()
        );
        assert!(
            found.contains(&path.to_string()),
            "skip entry {path} ({}) is not a .rho file found by the walk",
            reason.as_str()
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    for (path, _) in SKIPS {
        assert!(seen.insert(path), "duplicate skip entry: {path}");
    }
}

/// The stack the corpus runs on. The parser and reducer recurse over the term, and the guard in
/// `rholang/src/parser.rs` bounds *nesting* (`MAX_PARSE_DEPTH = 128`), not stack: each level enters
/// ~16 nested parse functions, so the guard's limit costs ~3 MiB of stack in a debug build (and
/// under 2 MiB in release — measured, see `spec/AUDIT.md` §16). `MakeMint.rho`, one of the node's
/// own genesis contracts, reaches parse depth 64. An explicit stack here beats a `RUST_MIN_STACK`
/// invocation nobody will remember, and it documents the requirement where it is needed.
const STACK: usize = 8 << 20;

/// The corpus itself: each unskipped file must parse, normalize and reduce to completion.
///
/// `attempted + skipped == CORPUS` is asserted from both sides: the skip list must cover exactly
/// the files that are not reduced, so a file that starts failing cannot be waved through by adding
/// it to `SKIPS` without also updating this count.
#[test]
fn legacy_contracts_parse_and_reduce() {
    // Both stacks matter: `block_on` drives the future on the *calling* thread, and each program's
    // reduction runs on a runtime worker. A default-sized stack on either one aborts the process.
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .thread_stack_size(STACK)
                .enable_all()
                .build()
                .expect("a tokio runtime")
                .block_on(run_corpus())
        })
        .expect("spawn the corpus thread")
        .join()
        .expect("the corpus thread panicked");
}

async fn run_corpus() {
    let all = corpus();
    let skipped: Vec<&str> = SKIPS.iter().map(|(p, _)| *p).collect();
    let mut failures: Vec<String> = Vec::new();
    let mut attempted = 0usize;

    for rel in &all {
        if skipped.contains(&rel.as_str()) {
            continue;
        }
        attempted += 1;
        let source = read(rel);
        // A fresh runtime per program: a shared one would carry channels across programs, so a
        // match in program N could come from program N-1's leftovers.
        let rt = common::build_runtime(true).await;
        rt.set_max_reduce_steps(FUEL);
        let env = env_for(&source);
        let rand = fixed_rand();

        // Unbuffered and off by default: a program that aborts the process (a stack overflow is not
        // a catchable error) leaves its name as the last line written, which is the only way to
        // find it. Set `LEGACY_PROGRESS=1` when that happens; otherwise it is noise.
        if std::env::var_os("LEGACY_PROGRESS").is_some() {
            eprintln!("EVAL {rel}");
        }
        match tokio::time::timeout(TIMEOUT, rt.evaluate_with_env(&source, &env, &rand)).await {
            Err(_elapsed) => failures.push(format!(
                "{rel}: timed out after {TIMEOUT:?} ({FUEL} reduction steps)"
            )),
            Ok(Err(e)) => failures.push(format!("{rel}: parse/normalize error: {e}")),
            Ok(Ok(result)) if !result.errors.is_empty() => {
                failures.push(format!("{rel}: reduce errors: {:?}", result.errors))
            }
            Ok(Ok(_)) => {}
        }
    }

    // Accounting first, and about *attempts* rather than successes: a program that fails is still
    // accounted for, so this assertion cannot fire early and hide why a program failed (it did, in
    // the first run of this test — the count tripped before the failure list was printed).
    assert_eq!(
        attempted + SKIPS.len(),
        CORPUS,
        "{attempted} attempted + {} skipped != {CORPUS}",
        SKIPS.len()
    );

    if !failures.is_empty() {
        // Per-reason counts, so a growing bucket is visible rather than aggregated into one number.
        for reason in SkipReason::ALL {
            let n = SKIPS.iter().filter(|(_, r)| r == reason).count();
            println!("skipped ({}): {n}", reason.as_str());
        }
        for f in &failures {
            println!("FAIL {f}");
        }
        panic!(
            "{} of {attempted} corpus programs failed to reduce ({} skipped)",
            failures.len(),
            SKIPS.len()
        );
    }
}
