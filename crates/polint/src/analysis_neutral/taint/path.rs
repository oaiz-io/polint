//! Access paths: a slot and the steps below it. A fact on a path means the path
//! and everything below it carries taint, so a fact on a shorter path covers a
//! longer one.

use std::hash::{Hash, Hasher};

use crate::analysis_neutral::taint::ir::{Slot, Step};

/// The most steps a path keeps, dereferences included.
pub(crate) const MAX_STEPS: usize = 6;

/// The steps below a slot, at most [`MAX_STEPS`] long. Only the first `len`
/// steps are part of the path: equality, ordering and hashing ignore the rest.
#[derive(Clone, Copy)]
pub(crate) struct Path {
    len: u8,
    steps: [Step; MAX_STEPS],
}

impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        self.steps() == other.steps()
    }
}

impl Eq for Path {}

impl PartialOrd for Path {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Path {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.steps().cmp(other.steps())
    }
}

impl Hash for Path {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u8(self.len);
        for step in self.steps() {
            step.hash(state);
        }
    }
}

impl Default for Path {
    fn default() -> Self {
        Path::EMPTY
    }
}

impl std::fmt::Debug for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.steps()).finish()
    }
}

impl Path {
    pub(crate) const EMPTY: Path = Path {
        len: 0,
        steps: [Step::Deref; MAX_STEPS],
    };

    pub(crate) fn steps(&self) -> &[Step] {
        &self.steps[..self.len as usize]
    }

    pub(crate) fn len(&self) -> usize {
        self.len as usize
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The path of `steps`, cut to the field limit `k` (dereferences do not count)
    /// and to [`MAX_STEPS`]. Cutting widens the fact: it then covers everything
    /// below the cut.
    pub(crate) fn limited(steps: impl IntoIterator<Item = Step>, k: usize) -> Path {
        let mut path = Path::EMPTY;
        let mut fields = 0;
        for step in steps {
            if !matches!(step, Step::Deref) {
                fields += 1;
                if fields > k {
                    break;
                }
            }
            if path.len as usize == MAX_STEPS {
                break;
            }
            path.steps[path.len as usize] = step;
            path.len += 1;
        }
        path
    }

    /// `prefix` followed by `rest`, limited to `k` fields.
    pub(crate) fn join(prefix: &[Step], rest: &[Step], k: usize) -> Path {
        Path::limited(prefix.iter().chain(rest.iter()).copied(), k)
    }

    /// The steps after `prefix`, when `prefix` is a prefix of this path.
    pub(crate) fn strip<'a>(&'a self, prefix: &[Step]) -> Option<&'a [Step]> {
        self.steps().strip_prefix(prefix)
    }

    /// Whether this path is a prefix of `steps` (so a fact on it covers them).
    pub(crate) fn covers(&self, steps: &[Step]) -> bool {
        steps.starts_with(self.steps())
    }

    /// This path's first `len` steps.
    pub(crate) fn prefix(&self, len: usize) -> Path {
        let mut prefix = Path::EMPTY;
        let len = len.min(self.len as usize);
        prefix.steps[..len].copy_from_slice(&self.steps[..len]);
        prefix.len = len as u8;
        prefix
    }

    pub(crate) fn has_memory_step(&self) -> bool {
        self.steps().iter().any(|step| step.is_memory())
    }
}

/// A fact: the slot and the path below it that carries taint.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct Fact {
    pub(crate) slot: Slot,
    pub(crate) path: Path,
}

impl Fact {
    pub(crate) fn whole(slot: Slot) -> Fact {
        Fact {
            slot,
            path: Path::EMPTY,
        }
    }

    pub(crate) fn new(slot: Slot, path: Path) -> Fact {
        Fact { slot, path }
    }
}
