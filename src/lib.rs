use lru::{LruCache, DefaultHasher};
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

struct Entry<D> {
    data: D,
    expiration: Instant,
}

impl<D> Entry<D> {
    fn new(data: D, expiration: Instant) -> Self {
        Entry {
            data,
            expiration,
        }
    }

    fn is_valid(&self) -> bool {
        self.expiration > Instant::now()
    }
}

pub struct Cache<K: Eq + Hash, D> {
    lru_cache: LruCache<K, Entry<D>>,
    positive_ttl: Duration, // seconds
}

impl<K: Eq + Hash, D: Eq> Cache<K, D> {
    pub fn new(
        size: usize,
        ttl: Duration,
    ) -> Self {
        let hash_builder = DefaultHasher::default();
        Cache {
            lru_cache: LruCache::with_hasher(
                NonZeroUsize::new(size).unwrap(),
                hash_builder,
            ),
            positive_ttl: ttl,
        }
    }

    pub fn insert(&mut self, key: K, data: D) {
        let expiration = Instant::now() + self.positive_ttl;
        let data = Entry::new(data, expiration);
        self.lru_cache.put(key, data);
    }

    pub fn get(&mut self, key: &K) -> Option<&D> {
        // Check if the entry exists and is valid
        let is_valid = if let Some(entry) = self.lru_cache.peek(key) {
            entry.is_valid()
        } else {
            false
        };

        // If the entry is valid, return it
        if is_valid {
            self.lru_cache.get(key).map(|entry| &entry.data)
        } else {
            // If the entry is expired, remove it
            self.lru_cache.pop(key);
            None
        }
    }

    pub fn len(&self) -> usize {
        self.lru_cache.len()
    }
}

// ===============================================================
// =============================TESTS=============================
// ===============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    #[fixture]
    fn basic() -> Cache<i32, i32> {
        Cache::new(
            3,
            Duration::from_secs(1),
        )
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
        assert_eq!(basic.get(&key1), None); // lru is removed
    }

    #[rstest]
    fn get_lru_change(mut basic: Cache<i32, i32>) {
        // Arrange
        let key1 = 1;
        let key2 = 2;
        let key3 = 3;
        let key4 = 4;
        let value = 2;

        // Act
        basic.insert(key1, value);
        basic.insert(key2, value);
        basic.get(&key1); // key2 is now the lru
        basic.insert(key3, value);
        basic.insert(key4, value);

        // Assert
        assert_eq!(basic.lru_cache.len(), 3);
        assert_eq!(basic.get(&key2), None); // lru is removed
    }

    #[fixture]
    fn ttl() -> Cache<i32, i32> {
        Cache::new(
            3,
            Duration::from_millis(200),
        )
    }

    #[rstest]
    fn ttl_expired(mut ttl: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        ttl.insert(key, value);
        std::thread::sleep(std::time::Duration::from_millis(250));

        // Assert
        assert_eq!(ttl.get(&key), None);
    }

}
