use std::collections::HashMap;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, serde::Serialize, serde::Deserialize,
)]
pub struct StableKeyId(pub u32);

impl StableKeyId {
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Test-only substring check against the process test interner.
    #[doc(hidden)]
    pub fn contains(self, needle: &str) -> bool {
        test_stable_key_interner().resolve(self).contains(needle)
    }
}

/// Interned key texts and their ids.
///
/// A [`StableKeyId`] is the index of its text in `keys`, so ids must be handed
/// out in insertion order and `keys` must stay densely packed: that invariant is
/// what makes `resolve` a vector index instead of a map lookup. Splitting this
/// state across shards would break it, because each shard would number its own
/// keys from zero.
#[derive(Debug, Default)]
struct StableKeyInternerState {
    keys: Vec<Arc<str>>,
    ids: HashMap<Arc<str>, StableKeyId>,
}

impl StableKeyInternerState {
    /// Appends `key`, returning the id that now indexes it: its position
    /// counted from `base`, which is zero except in an overlay.
    fn push_from(&mut self, base: u32, key: Arc<str>) -> StableKeyId {
        let id = StableKeyId(
            u32::try_from(self.keys.len())
                .ok()
                .and_then(|len| len.checked_add(base))
                .unwrap_or_else(|| panic!("stable-key interner exhausted u32 ids")),
        );
        self.keys.push(Arc::clone(&key));
        self.ids.insert(key, id);
        id
    }
}

#[derive(Debug, Clone, Default)]
pub struct StableKeyInterner {
    state: Arc<RwLock<StableKeyInternerState>>,
    /// Set for an overlay ([`StableKeyInterner::overlay`]): the interner that
    /// owns every id below `base`.
    parent: Option<Arc<OverlayParent>>,
}

#[derive(Debug)]
struct OverlayParent {
    interner: StableKeyInterner,
    base: u32,
}

/// Maps the ids an overlay handed out to the ids its parent gave the same keys.
///
/// Built by [`StableKeyInterner::absorb`]. Ids below the overlay's base were the
/// parent's to begin with and map to themselves.
#[derive(Debug, Clone)]
pub(crate) struct StableKeyRemap {
    base: u32,
    absorbed: Vec<StableKeyId>,
}

impl StableKeyRemap {
    pub(crate) fn apply(&self, id: StableKeyId) -> StableKeyId {
        match id.0.checked_sub(self.base) {
            Some(offset) => *self.absorbed.get(offset as usize).unwrap_or_else(|| {
                panic!("stable-key id {} was not interned by the overlay", id.0)
            }),
            None => id,
        }
    }
}

impl StableKeyInterner {
    /// Returns the id for `key`, assigning a new one the first time it is seen.
    ///
    /// Text already interned is answered under a read lock and without
    /// allocating, so callers holding a `&str` pay nothing extra on the common
    /// path; the owned form is only materialized when the key is new.
    pub fn intern(&self, key: impl AsRef<str> + Into<String>) -> StableKeyId {
        if let Some(id) = self.parent_id(key.as_ref()) {
            return id;
        }
        if let Some(id) = self.read().ids.get(key.as_ref()) {
            return *id;
        }

        let key = key.into();
        let mut state = self.write();
        if let Some(id) = state.ids.get(key.as_str()) {
            return *id;
        }
        let base = self.base();
        state.push_from(base, key.into())
    }

    /// A private interner for one task of a parallel stage.
    ///
    /// The overlay answers every key this interner already holds with this
    /// interner's id, and numbers the keys it is the first to see from this
    /// interner's current length upwards, in the order the task interns them.
    /// Interning from several threads into one shared interner would number the
    /// keys in whatever order the threads reach it; giving each task an overlay
    /// and folding the overlays back with [`absorb`](Self::absorb) in task order
    /// numbers them the same way on every run and at every job count.
    ///
    /// The overlay only ever answers with this interner's ids below `base`,
    /// the ones that existed when it was taken. A key this interner gains
    /// later, from another thread, is numbered by the overlay like any new key
    /// and mapped to this interner's id when the overlay is absorbed, so ids the
    /// overlay hands out never collide with ids handed out here meanwhile.
    pub(crate) fn overlay(&self) -> Self {
        assert!(
            self.parent.is_none(),
            "an overlay of an overlay would number keys from two bases"
        );
        let base = u32::try_from(self.read().keys.len())
            .unwrap_or_else(|_| panic!("stable-key interner exhausted u32 ids"));
        Self {
            state: Arc::default(),
            parent: Some(Arc::new(OverlayParent {
                interner: self.clone(),
                base,
            })),
        }
    }

    /// Interns the keys `overlay` was the first to see, in the order it saw
    /// them, and returns the map from its ids to this interner's.
    pub(crate) fn absorb(&self, overlay: &StableKeyInterner) -> StableKeyRemap {
        let parent = overlay
            .parent
            .as_ref()
            .expect("only an overlay can be absorbed");
        assert!(
            Arc::ptr_eq(&parent.interner.state, &self.state),
            "an overlay is absorbed into the interner it was taken from"
        );
        let keys = overlay.read().keys.clone();
        StableKeyRemap {
            base: parent.base,
            absorbed: keys
                .into_iter()
                .map(|key| self.intern(key.as_ref()))
                .collect(),
        }
    }

    fn base(&self) -> u32 {
        self.parent.as_ref().map_or(0, |parent| parent.base)
    }

    /// The parent's id for `key`, when this is an overlay and the parent had
    /// the key when the overlay was taken.
    fn parent_id(&self, key: &str) -> Option<StableKeyId> {
        let parent = self.parent.as_ref()?;
        let id = *parent.interner.read().ids.get(key)?;
        (id.0 < parent.base).then_some(id)
    }

    /// Interns `key` and returns its text in the same lock acquisition.
    ///
    /// Callers that need both — every fact-metadata construction does, because
    /// the stable-key text is hashed into the payload digest — would otherwise
    /// take the lock twice for one key.
    pub(crate) fn intern_and_resolve(&self, key: &str) -> (StableKeyId, Arc<str>) {
        if let Some(parent) = &self.parent
            && let Some((text, id)) = parent
                .interner
                .read()
                .ids
                .get_key_value(key)
                .filter(|(_, id)| id.0 < parent.base)
                .map(|(text, id)| (Arc::clone(text), *id))
        {
            return (id, text);
        }
        let base = self.base();
        let mut state = self.write();
        if let Some((text, id)) = state.ids.get_key_value(key) {
            return (*id, Arc::clone(text));
        }
        let text: Arc<str> = Arc::from(key);
        let id = state.push_from(base, Arc::clone(&text));
        (id, text)
    }

    /// Number of distinct keys interned so far.
    ///
    /// The resource gauge reports this at every stage boundary: interned key
    /// text is retained for the whole run, so its count is the single best
    /// proxy for how much of peak RSS is identity strings rather than facts.
    pub(crate) fn len(&self) -> usize {
        self.base() as usize + self.read().keys.len()
    }

    /// Total bytes of interned key text (excludes per-`Arc` and map overhead).
    pub(crate) fn text_bytes(&self) -> usize {
        self.read().keys.iter().map(|key| key.len()).sum()
    }

    pub fn resolve(&self, id: StableKeyId) -> Arc<str> {
        if let Some(parent) = &self.parent
            && id.0 < parent.base
        {
            return parent.interner.resolve(id);
        }
        let state = self.read();
        Arc::clone(
            state
                .keys
                .get((id.0 - self.base()) as usize)
                .unwrap_or_else(|| panic!("unknown stable-key id {}", id.0)),
        )
    }

    /// A read-only view that resolves many keys under one lock acquisition.
    ///
    /// [`resolve`](Self::resolve) takes the lock and clones an `Arc` per call,
    /// which dominates loops that read millions of keys and contends when
    /// several threads run such loops at once. The view holds the read lock for
    /// its lifetime instead, which constrains where it may live:
    ///
    /// - the thread holding it must not intern a key until it is dropped;
    /// - it must not be held across a parallel section. The lock queues new
    ///   readers behind a waiting writer, and a rayon worker blocked on such a
    ///   read can be the one the section is waiting for, so the view, the
    ///   writer and the section would wait on each other. Take one view per
    ///   parallel task instead (see `Digest::of_rows`).
    pub(crate) fn read_view(&self) -> StableKeyReadView<'_> {
        StableKeyReadView {
            state: self.read(),
            parent: self
                .parent
                .as_ref()
                .map(|parent| (parent.interner.read(), parent.base)),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, StableKeyInternerState> {
        self.state.read().unwrap_or_else(|error| error.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, StableKeyInternerState> {
        self.state
            .write()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub fn detached_clone(&self) -> Self {
        assert!(
            self.parent.is_none(),
            "an overlay is folded back with `absorb`, not detached"
        );
        let state = self.read();
        let keys = state.keys.clone();
        let ids = keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                (
                    Arc::clone(key),
                    StableKeyId(
                        u32::try_from(index)
                            .unwrap_or_else(|_| panic!("stable-key interner exhausted u32 ids")),
                    ),
                )
            })
            .collect();
        Self {
            state: Arc::new(RwLock::new(StableKeyInternerState { keys, ids })),
            parent: None,
        }
    }
}

/// Interned key texts readable without a lock round trip per key.
pub(crate) struct StableKeyReadView<'a> {
    state: RwLockReadGuard<'a, StableKeyInternerState>,
    parent: Option<(RwLockReadGuard<'a, StableKeyInternerState>, u32)>,
}

impl StableKeyReadView<'_> {
    /// The text of `id`, which must have been interned before the view was taken.
    pub(crate) fn text(&self, id: StableKeyId) -> &str {
        let (keys, index) = match &self.parent {
            Some((parent, base)) if id.0 < *base => (&parent.keys, id.0),
            Some((_, base)) => (&self.state.keys, id.0 - base),
            None => (&self.state.keys, id.0),
        };
        keys.get(index as usize)
            .unwrap_or_else(|| panic!("unknown stable-key id {}", id.0))
    }
}

/// Test-only helper: intern into the process-wide test interner.
#[doc(hidden)]
pub fn stable_key_for_test(key: &str) -> StableKeyId {
    test_stable_key_interner().intern(key)
}

/// Test-only process-wide interner for unit tests that lack an `AnalysisDb`.
#[doc(hidden)]
pub fn test_stable_key_interner() -> StableKeyInterner {
    use std::sync::OnceLock;

    static INTERNER: OnceLock<StableKeyInterner> = OnceLock::new();
    INTERNER.get_or_init(StableKeyInterner::default).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_text_reuses_an_id_and_resolves_exactly() {
        let interner = StableKeyInterner::default();

        let first = interner.intern("stable-key".to_string());
        let second = interner.intern("stable-key".to_string());

        assert_eq!(first, second);
        assert_eq!(interner.resolve(first).as_ref(), "stable-key");
    }

    #[test]
    fn ids_are_assigned_in_insertion_order_and_index_their_text() {
        let interner = StableKeyInterner::default();

        let ids = ["first", "second", "third", "second", "first"]
            .map(|key| interner.intern(key))
            .to_vec();

        assert_eq!(
            ids,
            [
                StableKeyId(0),
                StableKeyId(1),
                StableKeyId(2),
                StableKeyId(1),
                StableKeyId(0)
            ]
        );
        for (index, key) in ["first", "second", "third"].iter().enumerate() {
            let id = StableKeyId(u32::try_from(index).expect("small index"));
            assert_eq!(interner.resolve(id).as_ref(), *key);
        }
    }

    #[test]
    fn interning_with_the_text_agrees_with_interning_alone() {
        let interner = StableKeyInterner::default();
        let existing = interner.intern("existing");

        let (reused, reused_text) = interner.intern_and_resolve("existing");
        let (fresh, fresh_text) = interner.intern_and_resolve("fresh");

        assert_eq!(reused, existing);
        assert_eq!(reused_text.as_ref(), "existing");
        assert_eq!(fresh, StableKeyId(1));
        assert_eq!(fresh_text.as_ref(), "fresh");
        assert_eq!(interner.intern("fresh"), fresh);
    }

    #[test]
    fn an_overlay_reads_through_and_numbers_its_own_keys_from_the_base() {
        let shared = StableKeyInterner::default();
        let existing = shared.intern("existing");
        let overlay = shared.overlay();

        assert_eq!(overlay.intern("existing"), existing);
        let first = overlay.intern("first");
        let second = overlay.intern("second");
        assert_eq!((first, second), (StableKeyId(1), StableKeyId(2)));
        assert_eq!(overlay.intern("first"), first);
        assert_eq!(overlay.resolve(existing).as_ref(), "existing");
        assert_eq!(overlay.resolve(second).as_ref(), "second");
        let view = overlay.read_view();
        assert_eq!(view.text(existing), "existing");
        assert_eq!(view.text(first), "first");
        drop(view);
        assert_eq!(shared.len(), 1, "an overlay leaves its parent untouched");
    }

    #[test]
    fn a_key_the_parent_gains_after_the_overlay_was_taken_is_numbered_by_the_overlay() {
        let shared = StableKeyInterner::default();
        shared.intern("existing");
        let overlay = shared.overlay();
        let late = shared.intern("late");

        // The overlay numbers "late" itself, from the same base the parent's
        // late id came from, and only the remap relates the two.
        let seen = overlay.intern("late");
        let own = overlay.intern("own");
        assert_eq!(overlay.resolve(seen).as_ref(), "late");
        assert_eq!(overlay.resolve(own).as_ref(), "own");
        let remap = shared.absorb(&overlay);
        assert_eq!(remap.apply(seen), late);
        assert_eq!(shared.resolve(remap.apply(own)).as_ref(), "own");
    }

    #[test]
    fn absorbing_overlays_in_task_order_numbers_keys_independently_of_timing() {
        // Two tasks that see overlapping keys. Whichever finished first, folding
        // them back in task order must give every key the same id.
        let run = |absorb_second_first_interned: bool| {
            let shared = StableKeyInterner::default();
            shared.intern("existing");
            let first_task = shared.overlay();
            let second_task = shared.overlay();
            let (first_shared, second_shared) = if absorb_second_first_interned {
                second_task.intern("b");
                let second_shared = second_task.intern("shared");
                first_task.intern("a");
                (first_task.intern("shared"), second_shared)
            } else {
                first_task.intern("a");
                let first_shared = first_task.intern("shared");
                second_task.intern("b");
                (first_shared, second_task.intern("shared"))
            };
            let first_remap = shared.absorb(&first_task);
            let second_remap = shared.absorb(&second_task);
            assert_eq!(
                first_remap.apply(first_shared),
                second_remap.apply(second_shared)
            );
            assert_eq!(first_remap.apply(StableKeyId(0)), StableKeyId(0));
            ["a", "shared", "b"].map(|key| shared.intern(key))
        };

        assert_eq!(run(false), run(true));
        assert_eq!(run(false), [StableKeyId(1), StableKeyId(2), StableKeyId(3)]);
    }

    #[test]
    fn detached_clone_does_not_share_future_allocations() {
        let interner = StableKeyInterner::default();
        let original = interner.intern("first".to_string());
        let detached = interner.detached_clone();

        let second = interner.intern("second".to_string());

        assert_eq!(detached.resolve(original).as_ref(), "first");
        assert_eq!(second, StableKeyId(1));
        assert_eq!(detached.intern("detached".to_string()), StableKeyId(1));
    }
}
