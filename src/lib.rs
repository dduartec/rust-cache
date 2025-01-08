use lru::{LruCache, DefaultHasher};
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Entry<D> {
    data: D,
    adhoc_code: u8,
    expiration: Instant,
}

impl<D: Default> Entry<D> {

    fn default() -> Self {
        Entry {
            data: Default::default(),
            expiration: Instant::now(),
            adhoc_code: 0,
        }
    }


    fn new(data: D, expiration: Instant, adhoc_code: u8) -> Self {
        Entry {
            data,
            expiration,
            adhoc_code,
        }
    }

    fn is_valid(&self) -> bool {
        self.expiration > Instant::now()
    }
}

type MissHandler<K, D> = fn(&K, &mut D, &mut u8) -> bool;

pub struct Cache<K, D> {
    lru_cache: LruCache<K, Entry<D>>,
    miss_handler: MissHandler<K, D>,
    positive_ttl: Duration, // seconds
    negative_ttl: Duration, // seconds
}

impl<K: Eq + Hash + Copy, D: Eq + Default + Copy> Cache<K, D> {
    pub fn new(
        size: usize,
        miss_handler: MissHandler<K, D>,
        positive_ttl: Duration,
        negative_ttl: Duration,
    ) -> Self {
        let hash_builder = DefaultHasher::default();
        Cache {
            lru_cache: LruCache::with_hasher(
                NonZeroUsize::new(size).unwrap(),
                hash_builder,
            ),
            miss_handler,
            positive_ttl,
            negative_ttl,
        }
    }

    pub fn insert(&mut self, key: &K, data: &D) {
        let expiration = Instant::now() + self.positive_ttl;
        let entry = Entry::new(*data, expiration, 0);
        self.lru_cache.put(*key, entry);        
    }

    pub fn get(&mut self, key: &K) -> Option<&D> {
        self.get_entry(key).map(|entry| &entry.data)
    }

    fn get_entry(&mut self, key: &K) -> Option<&Entry<D>> {
        // First, check if the entry exists and is valid
        if let Some(entry) = self.lru_cache.get(key) {
            if entry.is_valid() {
                return self.lru_cache.get(key);
            }
        }
        // If the entry is expired, remove it
        self.lru_cache.pop(key);
        None
    }

    pub fn len(&self) -> usize {
        self.lru_cache.len()
    }

    pub fn retrieve_or_compute(&mut self, key: &K) -> (&D, u8) {
        let miss_handler = self.miss_handler;
        let positive_ttl = self.positive_ttl;
        let negative_ttl = self.negative_ttl;
        
        if let Some(_) = self.get_entry(key) {
            // Hit
            let cache_entry = self.lru_cache.peek(&key).unwrap();
            return (&cache_entry.data, cache_entry.adhoc_code);
        }                
    
        // Step 2: Handle cache miss
        let mut entry: Entry<D> = Entry::default();

        if miss_handler(&key, &mut entry.data, &mut entry.adhoc_code) {
            entry.expiration = Instant::now() + positive_ttl;
        } else {
            entry.expiration = Instant::now() + negative_ttl;
        }
    
        // Step 3: Insert the new entry
        let cache_entry = self.lru_cache.get_or_insert_mut(*key, || entry);
        (&cache_entry.data, cache_entry.adhoc_code)

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
    fn simple_cache() -> Cache<i32, i32> {
        fn miss_handler(key: &i32, data: &mut i32, adhoc_code: &mut u8) -> bool {
            if *key == -1 {
                return false
            }
            *data = 2;
            *adhoc_code+=1;
            true
        }
        Cache::new(
            3,
            miss_handler,
            Duration::from_millis(200),          
            Duration::from_millis(200),          
        )
    }

    #[rstest]
    fn insert_value(mut simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        simple_cache.insert(&key, &value);

        // Assert
        assert_eq!(simple_cache.lru_cache.len(), 1);
    }

    #[rstest]
    fn insert_same_key(mut simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        simple_cache.insert(&key, &value);
        simple_cache.insert(&key, &value);

        // Assert
        assert_eq!(simple_cache.lru_cache.len(), 1);
    }

    #[rstest]
    fn get_value(mut simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        simple_cache.insert(&key, &value);

        // Assert
        assert_eq!(simple_cache.get(&key), Some(&value));
    }

    #[rstest]
    fn get_value_not_found(mut simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;

        // Assert
        assert_eq!(simple_cache.get(&key), None);
    }

    #[rstest]
    fn insert_max_capacity(mut simple_cache: Cache<i32, i32>) {
        // Arrange
        let key1 = 1;
        let key2 = 2;
        let key3 = 3;
        let key4 = 4;
        let value = 2;

        // Act
        simple_cache.insert(&key1, &value);
        simple_cache.insert(&key2, &value);
        simple_cache.insert(&key3, &value);
        simple_cache.insert(&key4, &value);

        // Assert
        assert_eq!(simple_cache.lru_cache.len(), 3);
        assert_eq!(simple_cache.get(&key1), None); // lru is removed
    }

    #[rstest]
    fn get_lru_change(mut simple_cache: Cache<i32, i32>) {
        // Arrange
        let key1 = 1;
        let key2 = 2;
        let key3 = 3;
        let key4 = 4;
        let value = 2;

        // Act
        simple_cache.insert(&key1, &value);
        simple_cache.insert(&key2, &value);
        simple_cache.get(&key1); // key2 is now the lru
        simple_cache.insert(&key3, &value);
        simple_cache.insert(&key4, &value);

        // Assert
        assert_eq!(simple_cache.lru_cache.len(), 3);
        assert_eq!(simple_cache.get(&key2), None); // lru is removed
    }

    #[rstest]
    fn ttl_expired(mut simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        simple_cache.insert(&key, &value);
        std::thread::sleep(std::time::Duration::from_millis(250));

        // Assert
        assert_eq!(simple_cache.get(&key), None);
    }

    #[rstest]
    fn retrieve_or_compute_not_in_cache(mut simple_cache: Cache<i32, i32>){
        // Arrange
        let key = 1;

        // Act
        let (data, adhoc_code) = simple_cache.retrieve_or_compute(&key);

        // Assert
        assert_eq!(*data, 2);
        assert_eq!(adhoc_code, 1);
        assert_eq!(simple_cache.len(), 1);
    }

    #[rstest]
    fn retrieve_or_compute_already_in_cache(mut simple_cache: Cache<i32, i32>){
        // Arrange
        let key = 1;

        // Act
        simple_cache.retrieve_or_compute(&key);
        simple_cache.retrieve_or_compute(&key);
        simple_cache.retrieve_or_compute(&key);
        simple_cache.retrieve_or_compute(&key);
        let (data, adhoc_code) = simple_cache.retrieve_or_compute(&key);

        // Assert
        assert_eq!(*data, 2);
        assert_eq!(adhoc_code, 1);
        assert_eq!(simple_cache.len(), 1);
    }

    #[rstest]
    fn retrieve_or_compute_ttl_expired(mut simple_cache: Cache<i32, i32>){
        // Arrange
        let key = 1;

        // Act
        simple_cache.retrieve_or_compute(&key);
        let expiration_1 = simple_cache.lru_cache.peek(&key).unwrap().expiration;
        std::thread::sleep(std::time::Duration::from_millis(250));
        simple_cache.retrieve_or_compute(&key);
        let expiration_2 = simple_cache.lru_cache.peek(&key).unwrap().expiration;
        

        assert_ne!(expiration_1, expiration_2);      
    }

}
