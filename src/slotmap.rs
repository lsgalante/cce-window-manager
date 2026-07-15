// SPDX-FileCopyrightText: © 2025 Isaac Freund
// SPDX-License-Identifier: 0BSD

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub generation: u32,
    pub index: u32,
}

enum SlotData<T> {
    Value(T),
    NextFree(u32),
}

struct Slot<T> {
    generation: u32,
    data: SlotData<T>,
}

pub struct SlotMap<T> {
    slots: Vec<Slot<T>>,
    count: u32,
    first_free: u32,
}

impl<T> SlotMap<T> {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            count: 0,
            first_free: 1,
        }
    }
}

impl<T> Default for SlotMap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SlotMap<T> {

    pub fn put(&mut self, value: T) -> Key {
        let index = self.first_free as usize;
        if index < self.slots.len() {
            let slot = &mut self.slots[index];
            if let SlotData::NextFree(next) = slot.data {
                self.first_free = next;
            } else {
                panic!("SlotMap state corrupted: expected NextFree slot");
            }
            slot.data = SlotData::Value(value);
            self.count += 1;
            Key {
                generation: slot.generation,
                index: index as u32,
            }
        } else {
            self.slots.push(Slot {
                generation: 0,
                data: SlotData::Value(value),
            });
            let new_index = self.slots.len() - 1;
            self.count += 1;
            self.first_free += 1;
            Key {
                generation: 0,
                index: new_index as u32,
            }
        }
    }

    pub fn get(&self, key: Key) -> Option<&T> {
        let idx = key.index as usize;
        if idx < self.slots.len() {
            let slot = &self.slots[idx];
            if slot.generation == key.generation {
                if let SlotData::Value(ref val) = slot.data {
                    return Some(val);
                }
            }
        }
        None
    }

    pub fn get_mut(&mut self, key: Key) -> Option<&mut T> {
        let idx = key.index as usize;
        if idx < self.slots.len() {
            let slot = &mut self.slots[idx];
            if slot.generation == key.generation {
                if let SlotData::Value(ref mut val) = slot.data {
                    return Some(val);
                }
            }
        }
        None
    }

    pub fn remove(&mut self, key: Key) -> Option<T> {
        let idx = key.index as usize;
        if idx < self.slots.len() {
            let slot = &mut self.slots[idx];
            if slot.generation == key.generation {
                if let SlotData::Value(_) = slot.data {
                    let old_data = std::mem::replace(&mut slot.data, SlotData::NextFree(self.first_free));
                    slot.generation = slot.generation.wrapping_add(1);
                    self.count -= 1;
                    self.first_free = key.index;
                    if let SlotData::Value(val) = old_data {
                        return Some(val);
                    }
                }
            }
        }
        None
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            slots: &self.slots,
            index: 0,
        }
    }
}

pub struct Iter<'a, T> {
    slots: &'a [Slot<T>],
    index: usize,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.slots.len() {
            let slot = &self.slots[self.index];
            self.index += 1;
            if let SlotData::Value(ref val) = slot.data {
                return Some(val);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        let mut map = SlotMap::new();

        let key5 = map.put(5);
        assert_eq!(map.get(key5), Some(&5));

        assert_eq!(map.remove(key5), Some(5));
        assert_eq!(map.get(key5), None);

        let key6 = map.put(6);
        assert_eq!(map.get(key6), Some(&6));
        assert_eq!(map.get(key5), None);

        let key7 = map.put(7);
        let key8 = map.put(8);
        let key9 = map.put(9);

        assert_eq!(map.get(key6), Some(&6));
        assert_eq!(map.get(key7), Some(&7));
        assert_eq!(map.get(key8), Some(&8));
        assert_eq!(map.get(key9), Some(&9));

        map.remove(key8);
        assert_eq!(map.get(key8), None);
        assert_eq!(map.get(key9), Some(&9));
    }

    #[test]
    fn test_iteration() {
        let mut map = SlotMap::new();
        map.put(5);
        map.put(6);
        map.put(7);
        map.put(8);
        map.put(9);

        let vals: Vec<&i32> = map.iter().collect();
        assert_eq!(vals, vec![&5, &6, &7, &8, &9]);
    }
}
