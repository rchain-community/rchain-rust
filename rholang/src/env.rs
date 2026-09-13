//! The de Bruijn-level environment. Mirrors `rholang/.../interpreter/Env.scala`.

use std::collections::BTreeMap;

/// A de Bruijn environment mapping levels to terms (the Scala `Env[A]`).
///
/// `level` is the current binding level, `shift` offsets the level lookup. `put` binds a value at
/// the current level; `get(k)` resolves de Bruijn index `k` via `(level + shift) - k - 1`.
#[derive(Clone, Debug)]
pub struct Env<A> {
    env_map: BTreeMap<i32, A>,
    level: i32,
    shift: i32,
}

impl<A: Clone> Env<A> {
    pub fn new() -> Self {
        Env {
            env_map: BTreeMap::new(),
            level: 0,
            shift: 0,
        }
    }

    /// Bind `a` at the current level and advance the level (the Scala `put`).
    pub fn put(&self, a: A) -> Self {
        let mut env_map = self.env_map.clone();
        env_map.insert(self.level, a);
        Env {
            env_map,
            level: self.level + 1,
            shift: self.shift,
        }
    }

    /// Resolve de Bruijn index `k` (the Scala `get`).
    pub fn get(&self, k: i32) -> Option<A> {
        self.env_map
            .get(&((self.level + self.shift) - k - 1))
            .cloned()
    }

    /// Offset the level lookup by `j` (the Scala `shift`).
    pub fn shift(&self, j: i32) -> Self {
        Env {
            env_map: self.env_map.clone(),
            level: self.level,
            shift: self.shift + j,
        }
    }

    /// The current shift offset (the Scala `env.shift`).
    pub fn shift_amount(&self) -> i32 {
        self.shift
    }

    /// Build an environment from a sequence of values (the Scala `makeEnv`).
    pub fn make_env(values: impl IntoIterator<Item = A>) -> Self {
        let mut env = Env::new();
        for v in values {
            env = env.put(v);
        }
        env
    }
}

impl<A: Clone> Default for Env<A> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Index 0 is the **last** value bound, not the first: `get(k)` resolves
    /// `(level + shift) - k - 1`, so the environment counts downwards from the most recent
    /// binding. This is the De Bruijn convention the reducer relies on, and the one place where
    /// "obviously the first element" is wrong.
    #[test]
    fn index_zero_is_the_most_recent_binding() {
        let env = Env::make_env(["first", "second", "third"]);
        assert_eq!(env.get(0), Some("third"));
        assert_eq!(env.get(1), Some("second"));
        assert_eq!(env.get(2), Some("first"));
    }

    /// An unbound index is `None`, never a panic or a wrap-around to a valid level — a malformed
    /// de Bruijn index in a term must surface as an unbound variable, not as another variable's
    /// value.
    #[test]
    fn an_unbound_index_resolves_to_none() {
        let env = Env::make_env(["only"]);
        assert_eq!(env.get(1), None);
        assert_eq!(env.get(1000), None);
        assert_eq!(Env::<i32>::new().get(0), None);
        // Negative indices are not a panic path (`level + shift - k - 1` is just a key lookup).
        assert_eq!(env.get(-1), None);
    }

    /// `put` is a persistent update: the environment it was called on must not be modified, which
    /// is what lets a branch of the reducer bind a value without affecting its sibling.
    #[test]
    fn put_does_not_mutate_the_environment_it_was_called_on() {
        let base: Env<&str> = Env::new();
        let extended = base.put("v");
        assert_eq!(
            base.get(0),
            None,
            "the original must not have been extended"
        );
        assert_eq!(extended.get(0), Some("v"));
        assert_eq!(extended.shift_amount(), 0);
    }

    /// `put` binds at `level` and **ignores `shift`** — so in a shifted environment the newest
    /// binding is not the one index 0 resolves to. The reducer does not shift across a `put`
    /// today, but the asymmetry is easy to get wrong and cheap to pin.
    #[test]
    fn put_binds_at_the_unshifted_level() {
        let extended = Env::new().put(1).shift(-1).put(2);
        // level 2, shift -1: index 0 resolves level 0, which is the *first* binding.
        assert_eq!(extended.get(0), Some(1));
        assert_eq!(extended.get(1), None);
        // The value just bound sits at level 1, reachable only by shifting the lookup back.
        assert_eq!(extended.shift(1).get(0), Some(2));
    }

    /// A level can only be written once per lineage: `put` always increments `level`, so two
    /// branches from the same base bind at the same level in *separate* maps and neither sees the
    /// other. This is what makes the environment safe to share across a reducer's branches.
    #[test]
    fn a_level_is_never_rebound() {
        let base: Env<i32> = Env::new();
        let left = base.put(1);
        let right = base.put(2);
        assert_eq!(left.get(0), Some(1));
        assert_eq!(right.get(0), Some(2));
        assert_eq!(base.get(0), None, "the base is unchanged by either branch");
    }

    /// `shift(j)` moves the *lookup*, not the bindings: a positive shift makes an older binding
    /// visible at index 0, and a shift far enough back falls off the end into `None`.
    #[test]
    fn shift_moves_the_lookup_without_moving_the_bindings() {
        let env = Env::make_env(["a", "b"]); // a at level 0, b at level 1
        let shifted = env.shift(-1);
        assert_eq!(shifted.shift_amount(), -1);
        // lookup level = (2 + (-1)) - 0 - 1 = 0 -> the older binding.
        assert_eq!(shifted.get(0), Some("a"));
        assert_eq!(shifted.get(1), None, "past the oldest binding");

        // Shifting forward looks beyond the bindings, so nothing resolves.
        assert_eq!(env.shift(1).get(0), None);
    }

    /// `make_env` folds with `put`, so it must agree with an explicit chain — the property the
    /// dispatch tests rely on when they claim "the last datum is index 0".
    #[test]
    fn make_env_agrees_with_an_explicit_put_chain() {
        let built = Env::make_env(["a", "b", "c"]);
        let chained = Env::new().put("a").put("b").put("c");
        for k in 0..4 {
            assert_eq!(built.get(k), chained.get(k), "index {k}");
        }
        // An empty fold is the empty environment.
        assert_eq!(Env::<&str>::make_env(Vec::new()).get(0), None);
    }
}
