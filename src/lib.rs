use lru::{LruCache, DefaultHasher};
use std::hash::Hash;
use std::num::NonZeroUsize;


static RATIO: f64 = 1.3;

pub struct Cache<K: Eq + Hash, D> {
    lru_cache: LruCache<K, D>,
}

impl<K: Eq + Hash, D: Eq> Cache<K, D> {
    pub fn new(
        size: usize,
    ) -> Self {
        let hash_builder = DefaultHasher::default();
        Cache {
            lru_cache: LruCache::with_hasher(
                NonZeroUsize::new((size as f64 * RATIO) as usize).unwrap(),
                hash_builder,
            ),
        }
    }

    pub fn insert(&mut self, key: K, data: D) {
        self.lru_cache.put(key, data);
    }

    pub fn get(&mut self, key: &K) -> Option<&D> {
            self.lru_cache.get(key)
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    #[fixture]
    fn basic() -> Cache<i32, i32> {
        Cache::new(3)
    }

    #[rstest]
    fn insert_value(mut basic: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        basic.insert(key, value);

        // Assert
        assert_eq!(basic.lru_cache.len(), 1);
    }

    #[rstest]
    fn insert_same_key(mut basic: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        basic.insert(key, value);
        basic.insert(key, value);

        // Assert
        assert_eq!(basic.lru_cache.len(), 1);
    }

    #[rstest]
    fn get_value(mut basic: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        basic.insert(key, value);

        // Assert
        assert_eq!(basic.get(&key), Some(&value));
    }

    #[rstest]
    fn get_value_not_found(mut basic: Cache<i32, i32>) {
        // Arrange
        let key = 1;

        // Assert
        assert_eq!(basic.get(&key), None);
    }

    #[rstest]
    fn insert_max_capacity(mut basic: Cache<i32, i32>) {
        // Arrange
        let key1 = 1;
        let key2 = 2;
        let key3 = 3;
        let key4 = 4;
        let value = 2;

        // Act
        basic.insert(key1, value);
        basic.insert(key2, value);
        basic.insert(key3, value);
        basic.insert(key4, value);

        // Assert
        assert_eq!(basic.lru_cache.len(), 3);
        assert_eq!(basic.get(&key1), None);
    }

}
