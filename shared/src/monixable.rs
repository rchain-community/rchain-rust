//! Monix effect bridge.
//!
//! Mirrors `shared/src/main/scala/coop/rchain/monix/Monixable.scala`. Monix `Task` has no Rust
//! analogue, so this is a documented, identity-shaped shim: `to_task`/`from_task` collapse to the
//! identity conversion. The concrete effect runtime for the Rust port is tokio (see
//! [`crate::typed_store`]), which has no need for a `Task` bridge.

/// Bridge between an abstract effect and a concrete runtime (port of `Monixable[F]`); identity in
/// Rust because there is no separate `Task` type.
pub trait Monixable<T>: Sized {
    fn to_task(self) -> T;
    fn from_task(task: T) -> Self;
}

/// The identity instance (port of `Monixable.MonixableTask`).
impl<T> Monixable<T> for T {
    fn to_task(self) -> T {
        self
    }
    fn from_task(task: T) -> Self {
        task
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bridge is the **identity**, and that is the whole contract: the Rust port has no separate
    /// `Task` type, so `to_task`/`from_task` must not transform their argument — if they ever did,
    /// every call site that relies on them being no-ops would silently change behaviour. The
    /// round trip is also an inverse pair in both directions.
    #[test]
    fn the_bridge_is_the_identity_in_both_directions() {
        assert_eq!(42u8.to_task(), 42);
        assert_eq!(u8::from_task(42), 42);
        assert_eq!("text".to_string().to_task(), "text");
        assert_eq!(
            <String as Monixable<String>>::from_task("x".to_string()),
            "x"
        );
        assert_eq!(Vec::<u8>::new().to_task(), Vec::<u8>::new());
    }

    /// The blanket impl covers **every** `T`, including a type that is neither `Clone` nor
    /// `PartialEq` — so the shim cannot be relying on the value being copyable or comparable.
    #[test]
    fn the_instance_is_implemented_for_every_type() {
        struct Opaque(u8);
        let opaque: Opaque = Opaque(1).to_task();
        let back: Opaque = Monixable::from_task(opaque);
        assert_eq!(back.0, 1);
    }
}
