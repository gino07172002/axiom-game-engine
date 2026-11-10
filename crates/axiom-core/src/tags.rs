//! Per-world tag interning for runtime queries. Authoring keeps tag strings (World IR and
//! `CompiledEntity::tags` are unchanged); a compiled world also interns every distinct tag
//! to a [`TagId`] (first-seen order over the entity list, then over runtime spawns) and keeps
//! one compact [`TagSet`] per entity slot, so "does slot S carry tag T" is a bit test instead
//! of string comparisons over the entity record.
//!
//! Ids are only meaningful inside the [`TagIndex`] that issued them: a template compiled in
//! another world (RTS production) is re-interned by name when it is spawned.
use crate::CompiledEntity;
use std::collections::HashMap;

pub type TagId = u32;

/// Tags of one entity: ids below 64 in one inline word, larger ids in a sorted spill list
/// (empty, and not allocated, in worlds with at most 64 distinct tags).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagSet {
    low: u64,
    high: Box<[TagId]>,
}

impl TagSet {
    pub fn from_ids(ids: impl IntoIterator<Item = TagId>) -> Self {
        let mut low = 0u64;
        let mut high = vec![];
        for id in ids {
            if id < 64 {
                low |= 1 << id;
            } else {
                high.push(id);
            }
        }
        high.sort_unstable();
        high.dedup();
        Self {
            low,
            high: high.into_boxed_slice(),
        }
    }
    pub fn contains(&self, id: TagId) -> bool {
        if id < 64 {
            self.low >> id & 1 == 1
        } else {
            self.high.binary_search(&id).is_ok()
        }
    }
    /// Every tag of `required` is in `self` (an empty `required` always matches).
    pub fn contains_all(&self, required: &TagSet) -> bool {
        self.low & required.low == required.low
            && required
                .high
                .iter()
                .all(|id| self.high.binary_search(id).is_ok())
    }
    pub fn is_empty(&self) -> bool {
        self.low == 0 && self.high.is_empty()
    }
    /// Heap bytes owned by this set (the spill list).
    pub fn heap_bytes(&self) -> usize {
        self.high.len() * std::mem::size_of::<TagId>()
    }
}

/// Interned tags of a compiled world. Invariant: one set per entity slot, in slot order
/// (`sets.len() == CompiledWorld::len()`, kept by compile and `RuntimeState::spawn`).
#[derive(Clone, Debug, Default)]
pub struct TagIndex {
    ids: HashMap<Box<str>, TagId>,
    sets: Vec<TagSet>,
}

impl TagIndex {
    pub fn build(entities: &[CompiledEntity]) -> Self {
        let mut index = Self {
            ids: HashMap::new(),
            sets: Vec::with_capacity(entities.len()),
        };
        for e in entities {
            index.push(&e.tags);
        }
        index
    }
    /// Appends the set of a new entity slot, interning tags this world has not seen yet.
    pub fn push(&mut self, tags: &[String]) {
        let set = TagSet::from_ids(tags.iter().map(|t| self.intern(t)));
        self.sets.push(set);
    }
    fn intern(&mut self, tag: &str) -> TagId {
        if let Some(&id) = self.ids.get(tag) {
            return id;
        }
        let id = self.ids.len() as TagId;
        self.ids.insert(tag.into(), id);
        id
    }
    /// The id of `tag`, `None` when no entity of this world (so far) carries it.
    pub fn id(&self, tag: &str) -> Option<TagId> {
        self.ids.get(tag).copied()
    }
    /// The set of every tag in `tags`; `None` when one of them is not interned (then no
    /// current entity can carry all of them).
    pub fn resolve(&self, tags: &[String]) -> Option<TagSet> {
        tags.iter()
            .map(|t| self.id(t))
            .collect::<Option<Vec<_>>>()
            .map(TagSet::from_ids)
    }
    pub fn set(&self, slot: usize) -> &TagSet {
        &self.sets[slot]
    }
    pub fn has(&self, slot: usize, id: TagId) -> bool {
        self.sets[slot].contains(id)
    }
    /// Distinct tags interned so far.
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    /// Entity slots covered (equals the compiled world's length).
    pub fn slots(&self) -> usize {
        self.sets.len()
    }
    /// Approximate heap bytes: set array, spill lists, interned names and the hash table.
    pub fn heap_bytes(&self) -> usize {
        self.sets.capacity() * std::mem::size_of::<TagSet>()
            + self.sets.iter().map(TagSet::heap_bytes).sum::<usize>()
            + self.ids.keys().map(|k| k.len()).sum::<usize>()
            + self.ids.capacity() * (std::mem::size_of::<(Box<str>, TagId)>() + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(tags: &[&str]) -> CompiledEntity {
        let mut world = crate::World::empty("t");
        let mut e = crate::Entity::new("e");
        e.tags = tags.iter().map(|t| t.to_string()).collect();
        world.entities.push(e);
        world
            .compile(&crate::Registry::core())
            .unwrap()
            .entities
            .remove(0)
    }

    #[test]
    fn interns_in_first_seen_order_and_answers_membership() {
        let index = TagIndex::build(&[entity(&["player", "hero"]), entity(&["enemy", "player"])]);
        assert_eq!(index.len(), 3);
        assert_eq!(index.slots(), 2);
        assert_eq!(index.id("player"), Some(0));
        assert_eq!(index.id("enemy"), Some(2));
        assert_eq!(index.id("nobody"), None);
        assert!(index.has(0, 1) && !index.has(1, 1) && index.has(1, 2));
        let both = index.resolve(&["enemy".into(), "player".into()]).unwrap();
        assert!(!index.set(0).contains_all(&both) && index.set(1).contains_all(&both));
        assert!(index.resolve(&["enemy".into(), "nobody".into()]).is_none());
        assert!(index.set(0).contains_all(&TagSet::default()));
    }

    #[test]
    fn ids_beyond_one_word_spill_to_a_sorted_list() {
        let names: Vec<String> = (0..130).map(|i| format!("t{i}")).collect();
        let mut index = TagIndex::default();
        index.push(&names);
        index.push(&["t129".into(), "t3".into(), "t64".into(), "t129".into()]);
        let s = index.set(1);
        assert!(s.contains(3) && s.contains(64) && s.contains(129) && !s.contains(65));
        assert_eq!(s.heap_bytes(), 2 * 4);
        let req = index.resolve(&["t129".into(), "t64".into()]).unwrap();
        assert!(s.contains_all(&req) && index.set(0).contains_all(&req));
        let other = index.resolve(&["t65".into()]).unwrap();
        assert!(!s.contains_all(&other));
        assert!(!s.is_empty() && TagSet::default().is_empty());
    }
}
