use hashbrown::HashMap;
use std::hash::{BuildHasherDefault, Hash};
use std::collections::hash_map::DefaultHasher;

pub struct Cache<K: Eq + Hash, D> {
    data: HashMap<K, D, BuildHasherDefault<DefaultHasher>>,
}

impl<K: Eq + Hash, D: Eq> Cache<K, D> {
    pub fn new() -> Self {
        Cache {
            data: HashMap::default(),
        }
    }

    pub fn insert(&mut self, key: K, value: D) {
        self.data.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_cache() {
        // Arrange
        let cache = Cache::<i32, i32>::new();

        // Assert
        assert_eq!(cache.data.len(), 0);
    }

    #[test]
    fn insert_value() {
        // Arrange
        let mut cache = Cache::new();
        let key = 1;
        let value = 2;

        // Act
        cache.insert(key, value);

        // Assert
        assert_eq!(cache.data.get(&key), Some(&value));
    }
}
