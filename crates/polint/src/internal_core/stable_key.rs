//! Interned stable-key identities.
//!
//! A [`StableKeyId`] names a fact's identity by the *canonical bytes* of its
//! stable-key text. Two keys are the same key exactly when those bytes are
//! equal, whatever shape produced them: a raw interned string and a composite
//! built from a family plus parts intern to the same id when they encode the
//! same bytes, so ids stay dense and in insertion order exactly as before.
//!
//! Composite identities are stored as structure, not as expanded text. A
//! composite key routinely embeds a parent key's whole canonical text as one of
//! its parts, so keeping every key's complete text made a repository's
//! identities cost far more than its sources: 2.5 M keys held ~3 GiB of text for
//! 2.6 MB of Excalidraw source, because each level re-expanded every level below
//! it. Here a composite instead holds a short segment list — literal chunks in a
//! shared arena plus references to the child keys it embeds — and its canonical
//! bytes are streamed on demand.

use std::cell::RefCell;
use std::hash::{BuildHasher, Hasher};
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

/// One part value of a composite key.
///
/// [`KeyPart::Key`] is what makes structural sharing possible: it encodes
/// exactly the bytes `KeyPart::Text(&interner.resolve(id))` would, without
/// materializing or re-storing them.
#[derive(Clone, Copy, Debug)]
pub(crate) enum KeyPart<'a> {
    Text(&'a str),
    Key(StableKeyId),
}

/// One piece of a composite key's canonical bytes.
///
/// `Literal` names a range of the interner's shared arena — or, while a key is
/// still being built, of the caller's scratch buffer; `Key` names another
/// interned key whose canonical bytes appear verbatim at this position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Segment {
    Literal { start: u32, len: u32 },
    Key(u32),
}

/// How one key's canonical bytes are stored.
#[derive(Debug)]
enum KeyRepr {
    /// Complete text, kept as-is: what a key interned from a string gets, where
    /// there is no substructure to share.
    Text(Arc<str>),
    /// Canonical bytes are the concatenation of the segments' canonical bytes.
    Composite(Box<[Segment]>),
}

#[derive(Debug)]
struct KeyNode {
    repr: KeyRepr,
    /// Canonical byte length, kept on the node so a parent can write this key's
    /// length prefix without expanding it.
    len: u32,
    /// Composable hash of the canonical bytes (see [`CanonicalHasher`]).
    hash: u64,
    /// `base^len`, the term that lets a parent fold this key's hash in without
    /// re-reading its bytes.
    pow: u64,
    /// Whether the canonical bytes contain a backslash. Part values are written
    /// with `\` folded to `/`, so only a backslash-free key can be embedded by
    /// reference instead of by folded copy.
    has_backslash: bool,
}

/// Traversal frames for canonical streaming, inline until an identity nests
/// deeper than any real key does.
///
/// Streaming runs on the hot path of every `resolve`, so a heap allocation per
/// traversal would show up as allocator traffic proportional to fact count.
#[derive(Debug)]
struct FrameStack {
    inline: [(u32, u32); Self::INLINE],
    len: usize,
    spill: Vec<(u32, u32)>,
}

impl FrameStack {
    const INLINE: usize = 24;

    fn new(root: u32) -> Self {
        let mut stack = Self {
            inline: [(0, 0); Self::INLINE],
            len: 0,
            spill: Vec::new(),
        };
        stack.push((root, 0));
        stack
    }

    fn push(&mut self, frame: (u32, u32)) {
        if self.len < Self::INLINE {
            self.inline[self.len] = frame;
            self.len += 1;
        } else {
            self.spill.push(frame);
        }
    }

    fn pop(&mut self) -> Option<(u32, u32)> {
        if let Some(frame) = self.spill.pop() {
            return Some(frame);
        }
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        Some(self.inline[self.len])
    }
}

/// Composable hash of canonical bytes, modulo the Mersenne prime 2^61 - 1.
///
/// A composite's bytes are its children's bytes, so hashing them by traversal
/// would make interning cost the *expanded* length that structural sharing
/// exists to avoid — the whole point is to touch a key's own bytes only. This
/// polynomial hash concatenates: `H(a·b) = H(a)·base^|b| + H(b)`, so a parent
/// folds each child in with one multiply from the child's stored `(hash, pow)`.
///
/// `base` is drawn per interner from the process hash seed, so bucket
/// distribution is not a fixed function of the input, and every candidate a
/// probe accepts is still confirmed by comparing canonical bytes: a collision
/// costs one comparison, never a wrong identity.
#[derive(Debug, Clone, Copy)]
struct CanonicalHasher {
    base: u64,
}

const HASH_MODULUS: u64 = (1 << 61) - 1;

impl CanonicalHasher {
    fn new() -> Self {
        let seed = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        // Any base in [2, modulus - 2]; the extremes make degenerate polynomials.
        Self {
            base: 2 + seed % (HASH_MODULUS - 3),
        }
    }

    fn reduce(value: u128) -> u64 {
        let folded = ((value >> 61) + (value & u128::from(HASH_MODULUS))) as u64;
        if folded >= HASH_MODULUS {
            folded - HASH_MODULUS
        } else {
            folded
        }
    }

    fn multiply(left: u64, right: u64) -> u64 {
        Self::reduce(u128::from(left) * u128::from(right))
    }

    fn add(left: u64, right: u64) -> u64 {
        let sum = left + right;
        if sum >= HASH_MODULUS {
            sum - HASH_MODULUS
        } else {
            sum
        }
    }

    /// `(hash, base^len)` for a byte run.
    fn bytes(self, bytes: &[u8]) -> (u64, u64) {
        let mut hash = 0_u64;
        let mut pow = 1_u64;
        for byte in bytes {
            hash = Self::add(Self::multiply(hash, self.base), u64::from(*byte) + 1);
            pow = Self::multiply(pow, self.base);
        }
        (hash, pow)
    }

    /// `(hash, pow)` of `left` followed by `right`.
    fn concat(left: (u64, u64), right: (u64, u64)) -> (u64, u64) {
        (
            Self::add(Self::multiply(left.0, right.1), right.0),
            Self::multiply(left.1, right.1),
        )
    }

    /// Spreads a polynomial value across all 64 bits for probe indexing.
    fn bucket(hash: u64) -> u64 {
        let mut value = hash ^ 0x9e37_79b9_7f4a_7c15;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

/// Open-addressed canonical-bytes index: `slots[i]` holds `id + 1`, or 0 when
/// empty.
///
/// A `HashMap<Arc<str>, StableKeyId>` would have to own a complete text per key,
/// which is the cost this module exists to remove. Probing on each node's stored
/// canonical hash keeps the index at four bytes per slot.
#[derive(Debug, Default)]
struct CanonicalIndex {
    slots: Vec<u32>,
    filled: usize,
}

impl CanonicalIndex {
    /// Returns the id whose canonical bytes satisfy `matches`, if any.
    ///
    /// `matches` is consulted only for candidates whose stored hash and length
    /// already agree, so a full byte comparison runs once per genuine collision
    /// rather than once per probe.
    fn find(
        &self,
        hash: u64,
        nodes: &[KeyNode],
        len: usize,
        mut matches: impl FnMut(u32) -> bool,
    ) -> Option<u32> {
        if self.slots.is_empty() {
            return None;
        }
        let mask = self.slots.len() - 1;
        let mut index = (CanonicalHasher::bucket(hash) as usize) & mask;
        loop {
            let slot = self.slots[index];
            if slot == 0 {
                return None;
            }
            let candidate = slot - 1;
            let node = &nodes[candidate as usize];
            if node.hash == hash && node.len as usize == len && matches(candidate) {
                return Some(candidate);
            }
            index = (index + 1) & mask;
        }
    }

    /// Records `id`. `nodes` must already contain it, so a growth rehash sees a
    /// consistent table.
    fn insert(&mut self, id: u32, hash: u64, nodes: &[KeyNode]) {
        if self.slots.is_empty() || (self.filled + 1) * 4 >= self.slots.len() * 3 {
            self.grow(nodes);
        }
        let mask = self.slots.len() - 1;
        let mut index = (CanonicalHasher::bucket(hash) as usize) & mask;
        while self.slots[index] != 0 {
            index = (index + 1) & mask;
        }
        self.slots[index] = id + 1;
        self.filled += 1;
    }

    fn grow(&mut self, nodes: &[KeyNode]) {
        let capacity = if self.slots.is_empty() {
            1024
        } else {
            self.slots.len() * 2
        };
        let mut slots = vec![0_u32; capacity];
        let mask = capacity - 1;
        for slot in &self.slots {
            if *slot == 0 {
                continue;
            }
            let hash = nodes[(*slot - 1) as usize].hash;
            let mut index = (CanonicalHasher::bucket(hash) as usize) & mask;
            while slots[index] != 0 {
                index = (index + 1) & mask;
            }
            slots[index] = *slot;
        }
        self.slots = slots;
    }
}

/// Interned key structure and its ids.
///
/// A [`StableKeyId`] is the index of its node in `nodes`, so ids must be handed
/// out in insertion order and `nodes` must stay densely packed: that invariant is
/// what makes `resolve` a vector index instead of a map lookup. Splitting this
/// state across shards would break it, because each shard would number its own
/// keys from zero.
#[derive(Debug)]
struct StableKeyInternerState {
    nodes: Vec<KeyNode>,
    index: CanonicalIndex,
    /// Literal bytes of every composite key, concatenated. Segment ranges are
    /// `u32`, so a composite is stored as complete text once the arena would
    /// overflow that; identity is unaffected either way.
    arena: String,
    hasher: CanonicalHasher,
    /// Bytes actually retained for key text: the arena plus complete-text keys
    /// plus segment lists.
    retained_bytes: usize,
    /// Canonical bytes summed over every key — what a text-per-key
    /// representation would have retained.
    canonical_bytes: u64,
}

impl Default for StableKeyInternerState {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            index: CanonicalIndex::default(),
            arena: String::new(),
            hasher: CanonicalHasher::new(),
            retained_bytes: 0,
            canonical_bytes: 0,
        }
    }
}

impl StableKeyInternerState {
    /// Visits the canonical bytes of `id` in order, one chunk at a time.
    ///
    /// Traversal is iterative: a deeply nested identity must not be able to
    /// consume the call stack.
    fn stream(&self, id: u32, visit: &mut dyn FnMut(&[u8])) {
        let mut stack = FrameStack::new(id);
        while let Some((node_id, from)) = stack.pop() {
            match &self.nodes[node_id as usize].repr {
                KeyRepr::Text(text) => visit(text.as_bytes()),
                KeyRepr::Composite(segments) => {
                    let mut cursor = from as usize;
                    while cursor < segments.len() {
                        match segments[cursor] {
                            Segment::Literal { start, len } => {
                                let start = start as usize;
                                visit(&self.arena.as_bytes()[start..start + len as usize]);
                                cursor += 1;
                            }
                            Segment::Key(child) => {
                                stack.push((node_id, (cursor + 1) as u32));
                                stack.push((child, 0));
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Visits the canonical bytes a not-yet-interned composite would have.
    fn stream_pending(&self, scratch: &str, segments: &[Segment], visit: &mut dyn FnMut(&[u8])) {
        for segment in segments {
            match *segment {
                Segment::Literal { start, len } => {
                    let start = start as usize;
                    visit(&scratch.as_bytes()[start..start + len as usize]);
                }
                Segment::Key(child) => self.stream(child, visit),
            }
        }
    }

    /// `(hash, base^len)` for a run of literal bytes.
    fn hash_bytes(&self, bytes: &[u8]) -> (u64, u64) {
        self.hasher.bytes(bytes)
    }

    /// `(hash, base^len)` for a composite still held in scratch.
    ///
    /// Only the composite's own literal bytes are read; each embedded child
    /// folds in from the `(hash, pow)` its node already carries, so interning
    /// costs the key's own size rather than its expansion.
    fn hash_pending(&self, scratch: &str, segments: &[Segment]) -> (u64, u64) {
        let mut accumulated = (0_u64, 1_u64);
        for segment in segments {
            let piece = match *segment {
                Segment::Literal { start, len } => {
                    let start = start as usize;
                    self.hash_bytes(&scratch.as_bytes()[start..start + len as usize])
                }
                Segment::Key(child) => {
                    let node = &self.nodes[child as usize];
                    (node.hash, node.pow)
                }
            };
            accumulated = CanonicalHasher::concat(accumulated, piece);
        }
        accumulated
    }

    fn write_canonical(&self, id: u32, out: &mut String) {
        let mut bytes = Vec::with_capacity(self.nodes[id as usize].len as usize);
        self.stream(id, &mut |chunk| bytes.extend_from_slice(chunk));
        out.push_str(
            std::str::from_utf8(&bytes).expect("canonical key bytes concatenate UTF-8 pieces"),
        );
    }

    fn resolve(&self, id: u32) -> Arc<str> {
        let node = self
            .nodes
            .get(id as usize)
            .unwrap_or_else(|| panic!("unknown stable-key id {id}"));
        match &node.repr {
            KeyRepr::Text(text) => Arc::clone(text),
            KeyRepr::Composite(_) => {
                let mut text = String::with_capacity(node.len as usize);
                self.write_canonical(id, &mut text);
                Arc::from(text)
            }
        }
    }

    /// Id whose canonical bytes equal `text`, if one is interned.
    fn find_text(&self, text: &str, hash: u64) -> Option<u32> {
        self.index.find(hash, &self.nodes, text.len(), |candidate| {
            let mut cursor = CanonicalCursor::new(self, candidate);
            cursor.consume(text.as_bytes()) && cursor.at_end()
        })
    }

    /// Id whose canonical bytes equal the pending composite's, if one is interned.
    fn find_pending(
        &self,
        scratch: &str,
        segments: &[Segment],
        hash: u64,
        len: usize,
    ) -> Option<u32> {
        self.index.find(hash, &self.nodes, len, |candidate| {
            let mut cursor = CanonicalCursor::new(self, candidate);
            let mut equal = true;
            self.stream_pending(scratch, segments, &mut |chunk| {
                if equal && !cursor.consume(chunk) {
                    equal = false;
                }
            });
            equal && cursor.at_end()
        })
    }

    fn next_id(&self) -> u32 {
        u32::try_from(self.nodes.len())
            .unwrap_or_else(|_| panic!("stable-key interner exhausted u32 ids"))
    }

    fn push_text(&mut self, text: Arc<str>, hash: (u64, u64)) -> StableKeyId {
        let id = self.next_id();
        let len = u32::try_from(text.len()).expect("a stable key fits in u32 bytes");
        let has_backslash = text.as_bytes().contains(&b'\\');
        self.retained_bytes += text.len();
        self.canonical_bytes += u64::from(len);
        self.nodes.push(KeyNode {
            repr: KeyRepr::Text(text),
            len,
            hash: hash.0,
            pow: hash.1,
            has_backslash,
        });
        self.index.insert(id, hash.0, &self.nodes);
        StableKeyId(id)
    }

    fn materialize_pending(&self, scratch: &str, segments: &[Segment], len: usize) -> String {
        let mut text = String::with_capacity(len);
        let mut bytes = Vec::with_capacity(len);
        self.stream_pending(scratch, segments, &mut |chunk| {
            bytes.extend_from_slice(chunk)
        });
        text.push_str(std::str::from_utf8(&bytes).expect("canonical pieces are UTF-8"));
        text
    }

    fn push_composite(
        &mut self,
        scratch: &str,
        segments: &[Segment],
        hash: (u64, u64),
        len: usize,
    ) -> StableKeyId {
        // Segment ranges are u32 offsets into the arena. Past that the key keeps
        // its identity but stores complete text instead of structure.
        if self.arena.len() + scratch.len() > u32::MAX as usize {
            let text = self.materialize_pending(scratch, segments, len);
            return self.push_text(Arc::from(text), hash);
        }
        let base = self.arena.len() as u32;
        self.arena.push_str(scratch);
        let placed = segments
            .iter()
            .map(|segment| match *segment {
                Segment::Literal { start, len } => Segment::Literal {
                    start: base + start,
                    len,
                },
                Segment::Key(child) => Segment::Key(child),
            })
            .collect::<Box<[Segment]>>();
        let id = self.next_id();
        let len = u32::try_from(len).expect("a stable key fits in u32 bytes");
        self.retained_bytes += scratch.len() + placed.len() * size_of::<Segment>();
        self.canonical_bytes += u64::from(len);
        self.nodes.push(KeyNode {
            repr: KeyRepr::Composite(placed),
            len,
            hash: hash.0,
            pow: hash.1,
            // A composite writes every value with `\` folded to `/` and embeds
            // only backslash-free children, so its bytes never hold one.
            has_backslash: false,
        });
        self.index.insert(id, hash.0, &self.nodes);
        StableKeyId(id)
    }
}

/// Pull-side reader over one interned key's canonical bytes.
///
/// Equality streams both sides instead of materializing either, so comparing a
/// deeply shared identity costs no allocation.
struct CanonicalCursor<'a> {
    state: &'a StableKeyInternerState,
    stack: FrameStack,
    current: &'a [u8],
}

impl<'a> CanonicalCursor<'a> {
    fn new(state: &'a StableKeyInternerState, id: u32) -> Self {
        Self {
            state,
            stack: FrameStack::new(id),
            current: &[],
        }
    }

    /// Loads the next non-empty chunk into `current`; false once exhausted.
    fn advance(&mut self) -> bool {
        while self.current.is_empty() {
            let Some((node_id, from)) = self.stack.pop() else {
                return false;
            };
            match &self.state.nodes[node_id as usize].repr {
                KeyRepr::Text(text) => self.current = text.as_bytes(),
                KeyRepr::Composite(segments) => {
                    let mut cursor = from as usize;
                    while cursor < segments.len() {
                        match segments[cursor] {
                            Segment::Literal { start, len } => {
                                cursor += 1;
                                if len > 0 {
                                    let start = start as usize;
                                    self.current =
                                        &self.state.arena.as_bytes()[start..start + len as usize];
                                    if cursor < segments.len() {
                                        self.stack.push((node_id, cursor as u32));
                                    }
                                    break;
                                }
                            }
                            Segment::Key(child) => {
                                if cursor + 1 < segments.len() {
                                    self.stack.push((node_id, (cursor + 1) as u32));
                                }
                                self.stack.push((child, 0));
                                break;
                            }
                        }
                    }
                }
            }
        }
        true
    }

    /// Consumes `bytes` when they are the cursor's next bytes.
    fn consume(&mut self, mut bytes: &[u8]) -> bool {
        while !bytes.is_empty() {
            if !self.advance() {
                return false;
            }
            let take = bytes.len().min(self.current.len());
            if self.current[..take] != bytes[..take] {
                return false;
            }
            self.current = &self.current[take..];
            bytes = &bytes[take..];
        }
        true
    }

    fn at_end(&mut self) -> bool {
        !self.advance()
    }
}

/// Per-thread encoding buffers for [`StableKeyInterner::intern_key_parts`].
#[derive(Debug, Default)]
struct Pending {
    scratch: String,
    segments: Vec<Segment>,
}

thread_local! {
    static PENDING_KEY: RefCell<Pending> = RefCell::new(Pending::default());
}

#[derive(Debug, Clone)]
pub struct StableKeyInterner {
    state: Arc<RwLock<StableKeyInternerState>>,
}

impl Default for StableKeyInterner {
    fn default() -> Self {
        Self {
            state: Arc::new(RwLock::new(StableKeyInternerState::default())),
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
        let hash = {
            let state = self.read();
            let hash = state.hash_bytes(key.as_ref().as_bytes());
            if let Some(id) = state.find_text(key.as_ref(), hash.0) {
                return StableKeyId(id);
            }
            hash
        };
        let key = key.into();
        let mut state = self.write();
        if let Some(id) = state.find_text(&key, hash.0) {
            return StableKeyId(id);
        }
        state.push_text(Arc::from(key), hash)
    }

    /// Interns `key` and returns its text in the same lock acquisition.
    ///
    /// Callers that need both — every fact-metadata construction does, because
    /// the stable-key text is hashed into the payload digest — would otherwise
    /// take the lock twice for one key.
    pub(crate) fn intern_and_resolve(&self, key: &str) -> (StableKeyId, Arc<str>) {
        let mut state = self.write();
        let hash = state.hash_bytes(key.as_bytes());
        if let Some(id) = state.find_text(key, hash.0) {
            return (StableKeyId(id), state.resolve(id));
        }
        let text: Arc<str> = Arc::from(key);
        let id = state.push_text(Arc::clone(&text), hash);
        (id, text)
    }

    /// Interns the canonical key for `family` and `parts`, sharing rather than
    /// re-expanding whatever child identities the parts name.
    ///
    /// `parts` must already be in canonical (label-sorted) order. The bytes are
    /// exactly the ones the equivalent all-text key would have, so this returns
    /// the same id that path would.
    pub(crate) fn intern_key_parts(
        &self,
        family: &str,
        parts: &[(&str, KeyPart<'_>)],
    ) -> StableKeyId {
        // One key is built per fact, so the encoding buffers are borrowed from
        // the thread rather than allocated per call.
        PENDING_KEY.with(|pending| {
            let mut pending = pending.borrow_mut();
            let Pending { scratch, segments } = &mut *pending;
            scratch.clear();
            segments.clear();
            let (hash, len) = {
                let state = self.read();
                build_pending(&state, family, parts, scratch, segments);
                let len = pending_len(&state, segments);
                let hash = state.hash_pending(scratch, segments);
                if let Some(id) = state.find_pending(scratch, segments, hash.0, len) {
                    return StableKeyId(id);
                }
                (hash, len)
            };
            // Nodes are append-only, so the child ids and lengths this encoding
            // captured under the read lock are still valid; only the lookup can
            // have raced with another thread interning the same identity.
            let mut state = self.write();
            if let Some(id) = state.find_pending(scratch, segments, hash.0, len) {
                return StableKeyId(id);
            }
            state.push_composite(scratch, segments, hash, len)
        })
    }

    /// Feeds the canonical bytes of `id` to `visit` in order, without
    /// materializing them.
    pub(crate) fn stream_canonical(&self, id: StableKeyId, mut visit: impl FnMut(&[u8])) {
        let state = self.read();
        state.stream(id.0, &mut visit);
    }

    /// Orders two keys by their canonical bytes, materializing neither.
    ///
    /// Stores sort their rows by stable-key text; doing that through `resolve`
    /// expands both sides of every comparison, which is `O(n log n)`
    /// materializations of keys that are only being *compared*. Streaming both
    /// sides stops at the first differing byte and allocates nothing, while
    /// producing exactly the ordering `resolve(left).cmp(&resolve(right))` does.
    pub(crate) fn compare_canonical(
        &self,
        left: StableKeyId,
        right: StableKeyId,
    ) -> std::cmp::Ordering {
        if left == right {
            return std::cmp::Ordering::Equal;
        }
        let state = self.read();
        let mut left = CanonicalCursor::new(&state, left.0);
        let mut right = CanonicalCursor::new(&state, right.0);
        loop {
            let (left_more, right_more) = (left.advance(), right.advance());
            if !left_more || !right_more {
                // A key that ran out first is a prefix of the other, so it sorts first.
                return left_more.cmp(&right_more);
            }
            let shared = left.current.len().min(right.current.len());
            match left.current[..shared].cmp(&right.current[..shared]) {
                std::cmp::Ordering::Equal => {
                    left.current = &left.current[shared..];
                    right.current = &right.current[shared..];
                }
                ordering => return ordering,
            }
        }
    }

    /// Canonical byte length of `id`.
    pub(crate) fn canonical_len(&self, id: StableKeyId) -> usize {
        self.read().nodes[id.0 as usize].len as usize
    }

    /// Number of distinct keys interned so far.
    ///
    /// The resource gauge reports this at every stage boundary: interned key
    /// identity is retained for the whole run, so its count is the single best
    /// proxy for how much of peak RSS is identity rather than facts.
    pub(crate) fn len(&self) -> usize {
        self.read().nodes.len()
    }

    /// Bytes of key text actually retained: the shared arena, complete-text
    /// keys, and segment lists.
    ///
    /// This is storage, not canonical size — a composite's canonical bytes are
    /// streamed from shared structure, so a shared piece is counted once rather
    /// than once per key that embeds it. [`Self::canonical_bytes`] reports the
    /// expanded total.
    pub(crate) fn text_bytes(&self) -> usize {
        self.read().retained_bytes
    }

    /// Canonical bytes summed over every key: what a text-per-key
    /// representation would have retained.
    pub(crate) fn canonical_bytes(&self) -> u64 {
        self.read().canonical_bytes
    }

    /// Canonical text of `id`.
    ///
    /// A composite is expanded here rather than stored expanded. Caching the
    /// result behind a `Weak` was measured and rejected: `Arc<str>` keeps its
    /// text in the same allocation as its counts, so a surviving weak reference
    /// pins the bytes and the cache retains every key it ever handed out
    /// (jelly peak RSS 2.25 -> 2.83 GiB). Callers that keep the text should keep
    /// the [`StableKeyId`] instead and resolve at the boundary that needs bytes.
    pub fn resolve(&self, id: StableKeyId) -> Arc<str> {
        self.read().resolve(id.0)
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
        let state = self.read();
        let nodes = state
            .nodes
            .iter()
            .map(|node| KeyNode {
                repr: match &node.repr {
                    KeyRepr::Text(text) => KeyRepr::Text(Arc::clone(text)),
                    KeyRepr::Composite(segments) => KeyRepr::Composite(segments.clone()),
                },
                len: node.len,
                hash: node.hash,
                pow: node.pow,
                has_backslash: node.has_backslash,
            })
            .collect::<Vec<_>>();
        Self {
            state: Arc::new(RwLock::new(StableKeyInternerState {
                nodes,
                index: CanonicalIndex {
                    slots: state.index.slots.clone(),
                    filled: state.index.filled,
                },
                arena: state.arena.clone(),
                hasher: state.hasher,
                retained_bytes: state.retained_bytes,
                canonical_bytes: state.canonical_bytes,
            })),
        }
    }
}

/// Encodes `family` and `parts` into `scratch`/`segments`, referencing rather
/// than copying whatever child key can be embedded as-is.
fn build_pending(
    state: &StableKeyInternerState,
    family: &str,
    parts: &[(&str, KeyPart<'_>)],
    scratch: &mut String,
    segments: &mut Vec<Segment>,
) {
    let mut flushed = 0_usize;
    push_length_prefixed(scratch, family);
    for (label, value) in parts {
        scratch.push('|');
        push_length_prefixed(scratch, label);
        scratch.push('=');
        match value {
            KeyPart::Text(text) => push_length_prefixed_path(scratch, text),
            KeyPart::Key(id) => {
                let node = &state.nodes[id.0 as usize];
                if node.has_backslash {
                    // Folding changes these bytes, so this child cannot be
                    // shared by reference; write the folded copy instead.
                    let mut text = String::with_capacity(node.len as usize);
                    state.write_canonical(id.0, &mut text);
                    push_length_prefixed_path(scratch, &text);
                } else {
                    push_decimal(scratch, node.len as usize);
                    scratch.push(':');
                    flush_literal(segments, &mut flushed, scratch.len());
                    segments.push(Segment::Key(id.0));
                }
            }
        }
    }
    flush_literal(segments, &mut flushed, scratch.len());
}

fn flush_literal(segments: &mut Vec<Segment>, flushed: &mut usize, end: usize) {
    if end > *flushed {
        segments.push(Segment::Literal {
            start: u32::try_from(*flushed).expect("scratch offsets fit in u32"),
            len: u32::try_from(end - *flushed).expect("scratch offsets fit in u32"),
        });
        *flushed = end;
    }
}

fn pending_len(state: &StableKeyInternerState, segments: &[Segment]) -> usize {
    segments
        .iter()
        .map(|segment| match *segment {
            Segment::Literal { len, .. } => len as usize,
            Segment::Key(child) => state.nodes[child as usize].len as usize,
        })
        .sum()
}

/// Writes `value` as `<byte length>:<value>`.
pub(crate) fn push_length_prefixed(buffer: &mut String, value: &str) {
    push_decimal(buffer, value.len());
    buffer.push(':');
    buffer.push_str(value);
}

/// Like [`push_length_prefixed`], folding `\` to `/`. The fold is byte-for-byte,
/// so the length prefix is the same either way.
pub(crate) fn push_length_prefixed_path(buffer: &mut String, value: &str) {
    push_decimal(buffer, value.len());
    buffer.push(':');
    let mut rest = value;
    while let Some(index) = rest.find('\\') {
        buffer.push_str(&rest[..index]);
        buffer.push('/');
        rest = &rest[index + 1..];
    }
    buffer.push_str(rest);
}

pub(crate) fn push_decimal(buffer: &mut String, value: usize) {
    let mut digits = [0_u8; 20];
    let mut index = digits.len();
    let mut value = value;
    loop {
        index -= 1;
        digits[index] = b'0' + u8::try_from(value % 10).expect("a decimal digit fits in a byte");
        value /= 10;
        if value == 0 {
            break;
        }
    }
    buffer.push_str(std::str::from_utf8(&digits[index..]).expect("decimal digits are ASCII"));
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

    /// Canonical text for `family`/`parts` built the way the pre-sharing
    /// implementation did: every part expanded to complete text.
    fn expanded_text(
        interner: &StableKeyInterner,
        family: &str,
        parts: &[(&str, KeyPart<'_>)],
    ) -> String {
        let mut text = String::new();
        push_length_prefixed(&mut text, family);
        for (label, value) in parts {
            text.push('|');
            push_length_prefixed(&mut text, label);
            text.push('=');
            match value {
                KeyPart::Text(raw) => push_length_prefixed_path(&mut text, raw),
                KeyPart::Key(id) => {
                    push_length_prefixed_path(&mut text, interner.resolve(*id).as_ref())
                }
            }
        }
        text
    }

    fn canonical_bytes_of(interner: &StableKeyInterner, id: StableKeyId) -> Vec<u8> {
        let mut bytes = Vec::new();
        interner.stream_canonical(id, |chunk| bytes.extend_from_slice(chunk));
        bytes
    }

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
    fn detached_clone_does_not_share_future_allocations() {
        let interner = StableKeyInterner::default();
        let original = interner.intern("first".to_string());
        let detached = interner.detached_clone();

        let second = interner.intern("second".to_string());

        assert_eq!(detached.resolve(original).as_ref(), "first");
        assert_eq!(second, StableKeyId(1));
        assert_eq!(detached.intern("detached".to_string()), StableKeyId(1));
    }

    #[test]
    fn detached_clone_keeps_composite_structure_resolvable() {
        let interner = StableKeyInterner::default();
        let child = interner.intern("6:Import|4:path=3:a/b");
        let parent = interner.intern_key_parts("Type", &[("place", KeyPart::Key(child))]);

        let detached = interner.detached_clone();

        assert_eq!(detached.resolve(parent), interner.resolve(parent));
        assert_eq!(
            detached.intern(interner.resolve(parent).as_ref().to_string()),
            parent
        );
    }

    /// The compatibility boundary: a composite and the complete text it encodes
    /// are the same key, in either interning order.
    #[test]
    fn composite_and_raw_text_intern_to_one_id_in_both_orders() {
        let composite_first = StableKeyInterner::default();
        let child = composite_first.intern("8:Function|4:name=3:foo");
        let composite = composite_first.intern_key_parts(
            "Type",
            &[
                ("callable", KeyPart::Key(child)),
                ("phase", KeyPart::Text("late")),
            ],
        );
        let text = composite_first.resolve(composite).to_string();
        assert_eq!(composite_first.intern(text.clone()), composite);
        assert_eq!(composite_first.len(), 2);

        let text_first = StableKeyInterner::default();
        let child = text_first.intern("8:Function|4:name=3:foo");
        let raw = text_first.intern(text);
        let composite = text_first.intern_key_parts(
            "Type",
            &[
                ("callable", KeyPart::Key(child)),
                ("phase", KeyPart::Text("late")),
            ],
        );
        assert_eq!(composite, raw);
        assert_eq!(text_first.len(), 2);
    }

    #[test]
    fn composite_bytes_equal_the_expanded_encoding_for_every_shape() {
        let interner = StableKeyInterner::default();
        let leaf = interner.intern("10:SourceFile|4:path=11:src/main.go");
        let backslashed = interner.intern("4:Weird|3:raw=5:a\\b\\c");
        let unicode = interner.intern("4:Uni|4:name=7:naïve");
        let empty = interner.intern("");
        let deep = interner.intern_key_parts("Body", &[("file", KeyPart::Key(leaf))]);

        let cases: Vec<(&str, Vec<(&str, KeyPart<'_>)>)> = vec![
            ("Type", vec![("place", KeyPart::Key(leaf))]),
            ("Type", vec![("place", KeyPart::Key(backslashed))]),
            ("Type", vec![("place", KeyPart::Key(unicode))]),
            ("Type", vec![("place", KeyPart::Key(empty))]),
            ("Type", vec![("place", KeyPart::Key(deep))]),
            (
                "AccessPath",
                vec![
                    ("base", KeyPart::Key(deep)),
                    ("proj", KeyPart::Text("field\\x")),
                    ("tail", KeyPart::Key(leaf)),
                ],
            ),
            ("Empty", vec![]),
            (
                "Both",
                vec![("a", KeyPart::Text("")), ("b", KeyPart::Key(empty))],
            ),
        ];

        for (family, parts) in cases {
            let want = expanded_text(&interner, family, &parts);
            let id = interner.intern_key_parts(family, &parts);
            assert_eq!(
                String::from_utf8(canonical_bytes_of(&interner, id)).unwrap(),
                want,
                "streamed bytes for {family}"
            );
            assert_eq!(
                interner.resolve(id).as_ref(),
                want,
                "resolved text for {family}"
            );
            assert_eq!(interner.canonical_len(id), want.len());
            assert_eq!(
                interner.intern(want.clone()),
                id,
                "raw text of {family} must reuse the composite id"
            );
        }
    }

    /// Backslash folding is what makes an embedded child's bytes differ from its
    /// own, so a child holding one must be copied folded rather than shared.
    #[test]
    fn embedded_child_backslashes_are_folded_exactly_once() {
        let interner = StableKeyInterner::default();
        let child = interner.intern("a\\b");
        let parent = interner.intern_key_parts("F", &[("k", KeyPart::Key(child))]);

        assert_eq!(interner.resolve(parent).as_ref(), "1:F|1:k=3:a/b");
        // The folded parent is a distinct identity from the unfolded child.
        assert_ne!(parent, child);
        assert_eq!(interner.resolve(child).as_ref(), "a\\b");
    }

    #[test]
    fn deeply_nested_identities_stream_without_recursing() {
        let interner = StableKeyInterner::default();
        let mut id = interner.intern("0:");
        for _ in 0..20_000 {
            id = interner.intern_key_parts("N", &[("c", KeyPart::Key(id))]);
        }

        let bytes = canonical_bytes_of(&interner, id);
        assert_eq!(bytes.len(), interner.canonical_len(id));
        assert!(bytes.starts_with(b"1:N|1:c="));
        // Streaming shares every level, so retained bytes stay far below the
        // canonical expansion.
        assert!(
            (interner.text_bytes() as u64) < interner.canonical_bytes() / 100,
            "retained {} vs canonical {}",
            interner.text_bytes(),
            interner.canonical_bytes()
        );
    }

    #[test]
    fn distinct_chunk_boundaries_reach_the_same_identity() {
        let interner = StableKeyInterner::default();
        let whole = interner.intern("2:ab");
        let by_parts = interner.intern_key_parts("x", &[("k", KeyPart::Text("v"))]);
        let text = interner.resolve(by_parts).to_string();

        assert_eq!(interner.intern(text.as_str().to_string()), by_parts);
        assert_ne!(whole, by_parts);
        // A composite whose only segment is a child equals that child's bytes
        // only when the encoding says so; never by structural accident.
        let wrapped = interner.intern_key_parts("x", &[("k", KeyPart::Key(whole))]);
        assert_ne!(wrapped, whole);
    }

    #[test]
    fn maximum_length_prefixes_and_control_bytes_round_trip() {
        let interner = StableKeyInterner::default();
        let odd = interner.intern("\u{0}|=:\\\u{7f}");
        let parts = vec![("z", KeyPart::Key(odd)), ("a", KeyPart::Text("=|:"))];
        let want = expanded_text(&interner, "Odd", &parts);
        let id = interner.intern_key_parts("Odd", &parts);

        assert_eq!(interner.resolve(id).as_ref(), want);
        assert_eq!(interner.intern(want), id);
    }

    #[test]
    fn decimal_prefixes_match_byte_lengths_not_char_counts() {
        let mut buffer = String::new();
        push_length_prefixed_path(&mut buffer, "naïve\\x");

        assert_eq!(buffer, "8:naïve/x");
        assert_eq!("naïve\\x".len(), 8);
    }

    #[test]
    fn push_decimal_covers_zero_and_the_widest_value() {
        let mut buffer = String::new();
        push_decimal(&mut buffer, 0);
        buffer.push(' ');
        push_decimal(&mut buffer, usize::MAX);

        assert_eq!(buffer, format!("0 {}", usize::MAX));
    }

    #[test]
    fn canonical_comparison_matches_resolved_text_order() {
        let interner = StableKeyInterner::default();
        let leaf = interner.intern("10:SourceFile|4:path=1:a");
        let other_leaf = interner.intern("10:SourceFile|4:path=1:b");
        let mut ids = vec![leaf, other_leaf, interner.intern(""), interner.intern("z")];
        for base in [leaf, other_leaf] {
            for label in ["a", "b", "aa"] {
                ids.push(interner.intern_key_parts("Type", &[(label, KeyPart::Key(base))]));
                ids.push(interner.intern_key_parts(
                    "Type",
                    &[(label, KeyPart::Key(base)), ("z", KeyPart::Text("x"))],
                ));
            }
        }

        for left in &ids {
            for right in &ids {
                assert_eq!(
                    interner.compare_canonical(*left, *right),
                    interner.resolve(*left).cmp(&interner.resolve(*right)),
                    "{:?} vs {:?}",
                    interner.resolve(*left),
                    interner.resolve(*right)
                );
            }
        }

        let mut by_stream = ids.clone();
        by_stream.sort_by(|left, right| interner.compare_canonical(*left, *right));
        let mut by_text = ids.clone();
        by_text.sort_by_key(|id| interner.resolve(*id));
        assert_eq!(
            by_stream
                .iter()
                .map(|id| interner.resolve(*id))
                .collect::<Vec<_>>(),
            by_text
                .iter()
                .map(|id| interner.resolve(*id))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn concurrent_interning_of_one_identity_yields_one_id() {
        let interner = StableKeyInterner::default();
        let child = interner.intern("4:Base|1:x=1:y");

        let ids = std::thread::scope(|scope| {
            // Spawn every thread before joining any, so the interner really does
            // see the same identity arrive from several threads at once.
            let mut handles = Vec::new();
            for _ in 0..8 {
                let interner = interner.clone();
                handles.push(scope.spawn(move || {
                    (0..64)
                        .map(|_| {
                            interner.intern_key_parts("Type", &[("place", KeyPart::Key(child))])
                        })
                        .collect::<Vec<_>>()
                }));
            }
            let mut ids = Vec::new();
            for handle in handles {
                ids.extend(handle.join().expect("thread"));
            }
            ids
        });

        let first = ids[0];
        assert!(ids.iter().all(|id| *id == first));
        assert_eq!(interner.len(), 2);
    }
}
