//! The object sets of the points-to solve.
//!
//! Objects are dense indices assigned in object-id order, so walking a set in
//! index order walks it in object-id order. A small set is a sorted vector; a
//! set above [`DENSE_ABOVE`] members becomes a bitset over the indices, which
//! keeps membership tests constant and a large set at one bit per object.

/// Members above which a set is stored as a bitset. Most variables hold a
/// handful of objects; the few large ones are what bitsets are for.
const DENSE_ABOVE: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Repr {
    Sparse(Vec<u32>),
    Dense(Vec<u64>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObjectSet {
    repr: Repr,
    len: usize,
}

impl Default for ObjectSet {
    fn default() -> Self {
        Self {
            repr: Repr::Sparse(Vec::new()),
            len: 0,
        }
    }
}

impl ObjectSet {
    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(super) fn contains(&self, object: u32) -> bool {
        match &self.repr {
            Repr::Sparse(members) => members.binary_search(&object).is_ok(),
            Repr::Dense(words) => words
                .get(object as usize / 64)
                .is_some_and(|word| word & (1 << (object % 64)) != 0),
        }
    }

    /// The members in ascending order.
    pub(super) fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        let (sparse, dense) = match &self.repr {
            Repr::Sparse(members) => (Some(members.iter().copied()), None),
            Repr::Dense(words) => (None, Some(dense_members(words))),
        };
        sparse
            .into_iter()
            .flatten()
            .chain(dense.into_iter().flatten())
    }

    /// Appends the members of `objects` the set lacks to `missing`.
    pub(super) fn missing(&self, objects: &[u32], missing: &mut Vec<u32>) {
        missing.extend(
            objects
                .iter()
                .copied()
                .filter(|object| !self.contains(*object)),
        );
    }

    /// Adds `new`: ascending, without repeats, and none of them members.
    pub(super) fn add_new(&mut self, new: &[u32]) {
        if new.is_empty() {
            return;
        }
        self.len += new.len();
        if let Repr::Sparse(members) = &mut self.repr {
            if self.len <= DENSE_ABOVE {
                let mut merged = Vec::with_capacity(self.len);
                let (mut left, mut right) = (members.iter().peekable(), new.iter().peekable());
                while let (Some(a), Some(b)) = (left.peek(), right.peek()) {
                    let next = if a < b { left.next() } else { right.next() };
                    merged.extend(next.copied());
                }
                merged.extend(left);
                merged.extend(right);
                *members = merged;
                return;
            }
            let mut words = Vec::new();
            for object in members.iter() {
                set_bit(&mut words, *object);
            }
            self.repr = Repr::Dense(words);
        }
        if let Repr::Dense(words) = &mut self.repr {
            for object in new {
                set_bit(words, *object);
            }
        }
    }
}

fn set_bit(words: &mut Vec<u64>, object: u32) {
    let word = object as usize / 64;
    if words.len() <= word {
        words.resize(word + 1, 0);
    }
    words[word] |= 1 << (object % 64);
}

fn dense_members(words: &[u64]) -> impl Iterator<Item = u32> + '_ {
    words.iter().enumerate().flat_map(|(index, word)| {
        let base = index as u32 * 64;
        let mut bits = *word;
        std::iter::from_fn(move || {
            if bits == 0 {
                return None;
            }
            let bit = bits.trailing_zeros();
            bits &= bits - 1;
            Some(base + bit)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absorb(set: &mut ObjectSet, objects: &[u32]) -> Vec<u32> {
        let mut new = Vec::new();
        set.missing(objects, &mut new);
        set.add_new(&new);
        new
    }

    #[test]
    fn absorb_returns_only_the_members_the_set_lacked() {
        let mut set = ObjectSet::default();
        assert_eq!(absorb(&mut set, &[3, 7]), vec![3, 7]);
        assert_eq!(absorb(&mut set, &[1, 3, 9]), vec![1, 9]);
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![1, 3, 7, 9]);
        assert_eq!(set.len(), 4);
        assert!(set.contains(7) && !set.contains(8));
    }

    #[test]
    fn a_large_set_becomes_a_bitset_with_the_same_members() {
        let mut set = ObjectSet::default();
        let evens = (0..200).map(|object| object * 2).collect::<Vec<_>>();
        let odds = (0..200).map(|object| object * 2 + 1).collect::<Vec<_>>();
        assert_eq!(absorb(&mut set, &evens[..50]), evens[..50]);
        assert!(matches!(set.repr, Repr::Sparse(_)));
        assert_eq!(absorb(&mut set, &evens), evens[50..]);
        assert!(matches!(set.repr, Repr::Dense(_)));
        assert_eq!(absorb(&mut set, &odds), odds);
        assert_eq!(set.iter().collect::<Vec<_>>(), (0..400).collect::<Vec<_>>());
        assert_eq!(set.len(), 400);
        assert!(set.contains(399) && !set.contains(400) && !set.contains(100_000));
    }
}
