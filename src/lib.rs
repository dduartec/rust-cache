use lru::{LruCache, DefaultHasher};
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};


static RATIO: f64 = 1.3;



struct Entry<D> {
    data: D,
    expiration: Instant,
}

pub struct Cache<K: Eq + Hash, D> {
    lru_cache: LruCache<K, Entry<D>>,
    ttl: Duration, // seconds
}

impl<K: Eq + Hash, D: Eq> Cache<K, D> {
    pub fn new(
        size: usize,
        ttl: Duration,
    ) -> Self {
        let hash_builder = DefaultHasher::default();
        Cache {
            lru_cache: LruCache::with_hasher(
                NonZeroUsize::new((size as f64 * RATIO) as usize).unwrap(),
                hash_builder,
            ),
            ttl,
        }
    }

    pub fn insert(&mut self, key: K, data: D) {
        let expiration = Instant::now() + self.ttl;
        let data = Entry {data,expiration};
        self.lru_cache.put(key, data);
    }

    pub fn get(&mut self, key: &K) -> Option<&D> {
        if let Some(entry) = self.lru_cache.get_mut(key) {
            if entry.expiration > Instant::now() {
                return Some(&entry.data);
            }
            self.lru_cache.pop(key);
        }
        None
    }

    pub fn len(&self) -> usize {
        self.lru_cache.len()
    }
}

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
    fn ttl_expired(mut basic: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        basic.insert(key, value);
        std::thread::sleep(std::time::Duration::from_millis(250));

        // Assert
        assert_eq!(basic.get(&key), None);
    }

}
