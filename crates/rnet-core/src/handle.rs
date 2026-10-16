//! handle contracts shared by runtime and foreign-language boundaries.

use crate::Handle;

#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// Owner-local generational slots; external synchronization is required for shared mutation.
/// Handles encode a 32-bit generation and one-based 32-bit slot, not owner identity.
/// Different tables can produce identical numbers; callers must retain the owner.
/// Generations wrap after enough slot reuse, so handles are not everlasting unique IDs.
#[derive(Debug, Default)]
pub struct HandleTable<T> {
    slots: Vec<Slot<T>>,
    free: Vec<usize>,
}

impl<T> HandleTable<T> {
    /// Creates an empty table without allocating slots.
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    /// Counts live entries by scanning allocated slots; cost includes previously removed slots.
    pub fn len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.value.is_some())
            .count()
    }

    /// Checks for live entries; may scan all allocated slots.
    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(|slot| slot.value.is_none())
    }

    /// Takes ownership of `value` and returns its table-local handle, reusing a free slot if possible.
    /// The embedding owner must bound allocated slots below the 32-bit index-space limit.
    pub fn insert(&mut self, value: T) -> Handle {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                self.slots.push(Slot {
                    generation: 1,
                    value: None,
                });
                self.slots.len() - 1
            }
        };
        let slot = &mut self.slots[index];
        slot.value = Some(value);
        encode_handle(index, slot.generation)
    }

    /// Borrows the current value if `handle` names an occupied slot with matching generation.
    /// Zero, out-of-range and stale handles return `None`; owner identity is not encoded.
    pub fn get(&self, handle: Handle) -> Option<&T> {
        let (index, generation) = decode_handle(handle)?;
        let slot = self.slots.get(index)?;
        (slot.generation == generation)
            .then_some(slot.value.as_ref())
            .flatten()
    }

    /// Mutably borrows a matching occupied slot, or returns `None` for an invalid generation/index.
    /// The caller must use the handle with its original table.
    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        let (index, generation) = decode_handle(handle)?;
        let slot = self.slots.get_mut(index)?;
        (slot.generation == generation)
            .then_some(slot.value.as_mut())
            .flatten()
    }

    /// Moves out a matching value and advances the slot generation; repeated removal returns `None`.
    /// No mutation occurs for invalid/stale handles. Generation wrap is not global uniqueness.
    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        let (index, generation) = decode_handle(handle)?;
        let slot = self.slots.get_mut(index)?;
        if slot.generation != generation {
            return None;
        }
        let value = slot.value.take()?;
        // Zero marks an invalid handle. Reuse invalidates the previous generation,
        // but the finite generation space cannot promise lifetime-global uniqueness.
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(index);
        Some(value)
    }

    /// Iterates live handles in slot order while borrowing the table; not insertion order.
    pub fn handles(&self) -> impl Iterator<Item = Handle> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.value
                .as_ref()
                .map(|_| encode_handle(index, slot.generation))
        })
    }
}

fn encode_handle(index: usize, generation: u32) -> Handle {
    (u64::from(generation) << 32) | (index as u64 + 1)
}

fn decode_handle(handle: Handle) -> Option<(usize, u32)> {
    let encoded_index = (handle & u64::from(u32::MAX)) as u32;
    let generation = (handle >> 32) as u32;
    if encoded_index == 0 || generation == 0 {
        return None;
    }
    Some(((encoded_index - 1) as usize, generation))
}
