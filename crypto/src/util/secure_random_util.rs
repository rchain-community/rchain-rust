//! Non-blocking CSPRNG.
//!
//! Mirrors `crypto/src/main/scala/coop/rchain/crypto/util/SecureRandomUtil.scala`. The Scala tries
//! `NativePRNGNonBlocking` / `Windows-PRNG` / `SHA1PRNG`; the Rust port uses the OS CSPRNG directly.
//!
//! rand 0.10 renamed `OsRng` to [`SysRng`](rand::rngs::SysRng) (the same getrandom-backed OS
//! entropy source, now fallible: `try_fill_bytes` rather than `fill_bytes`). The alias keeps the
//! port's public name for `SecureRandomUtil`.

pub use rand::rngs::SysRng as OsRng;
