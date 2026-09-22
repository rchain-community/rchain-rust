//! Example: load the real census, show the `ways` distribution, fold a closure, and prove the
//! "ways as a coefficient" invariant.
//!
//! Usage: `cargo run -- [path/to/census_inventory.json]`
//!
//! The census path is resolved as: an explicit CLI argument, then the `QUCALC_CENSUS`
//! environment variable, then `census_inventory.json` in the current directory.

use qucalc::{fold, Census};
use std::path::PathBuf;

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("QUCALC_CENSUS").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("census_inventory.json"));

    let census =
        Census::load(&path).unwrap_or_else(|e| panic!("failed to load {}: {e}", path.display()));

    // 1. The distribution of `ways` across every class in the inventory.
    let mut all_ways: Vec<u64> = census
        .closures
        .values()
        .flat_map(|(_, branches)| branches.values().flatten().map(|w| w.ways))
        .collect();
    all_ways.sort_unstable();
    let n = all_ways.len();
    if n == 0 {
        println!("== census `ways` distribution (0 classes) ==");
        println!("   (empty census — nothing to report)");
        return;
    }
    let mean = all_ways.iter().sum::<u64>() as f64 / n as f64;
    println!("== census `ways` distribution ({n} classes) ==");
    println!(
        "   min={}  max={}  median={}  mean={mean:.0}",
        all_ways[0],
        all_ways[n - 1],
        all_ways[n / 2]
    );

    // 2. A concrete closure: `mix-ZX|R=3`, folded most-ways-first.
    let name = "mix-ZX|R=3";
    let sup = census.closure(name).expect("closure exists");
    println!("\n== {name}: superposition, most ways first ==");
    for w in &sup {
        println!(
            "   class {:>2}: signed {:>9}   ways {:>11}",
            w.class, w.signed, w.ways
        );
    }
    let (signed, ways) = fold(&sup);
    println!("   fold -> signed_sum = {signed}, ways_sum = {ways}");

    // 3. Prove the performance invariant: the top class is ONE term, not N terms.
    if let Some(top) = sup.first() {
        println!("\n== ways-as-coefficient invariant ==");
        println!(
            "   class {} has {} ways, held as a SINGLE WeightedClass",
            top.class, top.ways
        );
        println!(
            "   superposition has {} terms, NOT {} terms",
            sup.len(),
            top.ways
        );
    }
}
