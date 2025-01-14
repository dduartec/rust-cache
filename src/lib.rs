use lru::{LruCache, DefaultHasher};
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};
use std::sync::{Arc, RwLock};

#[derive(Debug, PartialEq, Clone)]
enum EntryStatus {
    AVAILABLE,
    CALCULATING,
    READY,
    FAILED,
}

#[derive(Debug, Clone, PartialEq)]
struct Entry<D> {
    data: D,
    adhoc_code: u8,
    expiration: Instant,
    status: EntryStatus,
}

impl<D: Default> Entry<D> {

    fn default() -> Self {
        Entry {
            data: Default::default(),
            expiration: Instant::now(),
            adhoc_code: 0,
            status: EntryStatus::AVAILABLE,
        }
    }


    fn new(data: D, expiration: Instant, adhoc_code: u8) -> Self {
        Entry {
            data,
            expiration,
            adhoc_code,
            status: EntryStatus::AVAILABLE,
        }
    }

    fn is_valid(&self) -> bool {
        self.expiration > Instant::now()
    }
}

type MissHandler<K, D> = fn(&K, &mut D, &mut u8) -> bool;

pub struct Cache<K, D> {
    lru_cache: Arc<RwLock<LruCache<K, Entry<D>>>>,
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
            lru_cache: Arc::new(RwLock::new(LruCache::with_hasher(
                NonZeroUsize::new(size).unwrap(),
                hash_builder,
            ))),
            miss_handler,
            positive_ttl,
            negative_ttl,
        }
    }

    pub fn insert(&self, key: &K, data: &D) {
        let expiration = Instant::now() + self.positive_ttl;
        let entry = Entry::new(*data, expiration, 0);
        self.lru_cache.write().unwrap().put(*key, entry);        
    }

    pub fn get(&self, key: &K) -> Option<D> {
            if self.is_in_cache(key) {
                return self.lru_cache.write().unwrap().get(key).map(|entry| entry.data.clone());
            }
            None
        }

    fn is_in_cache(&self, key: &K) -> bool {
        // First, check if the entry exists and is valid
        let is_in_cache = {
            let mut cache = self.lru_cache.write().unwrap();
            if let Some(entry) = cache.get(key) {
                entry.is_valid()
            } else {
                false
            }            
        };

        if is_in_cache {
            return true;
        }

        // If the entry is expired, remove it
        self.lru_cache.write().unwrap().pop(key);
        false
    }

    pub fn len(&self) -> usize {
        self.lru_cache.read().unwrap().len()
    }

    pub fn retrieve_or_compute(&self, key: &K) -> (&D, u8) {
        let miss_handler = self.miss_handler;
        let positive_ttl = self.positive_ttl;
        let negative_ttl = self.negative_ttl;
        
        if self.is_in_cache(key) {
            // Hit
            let cache = self.lru_cache.read().unwrap();
            let cache_entry = cache.peek(&key).unwrap();
            match cache_entry.status {
                EntryStatus::READY => {
                    return (unsafe { &*(&cache_entry.data as *const D) }, cache_entry.adhoc_code);
                }
                EntryStatus::FAILED => {
                    return (unsafe { &*(&cache_entry.data as *const D) }, cache_entry.adhoc_code);
                }
                EntryStatus::CALCULATING => {
                    //wait for the entry to change status
                    while cache_entry.status == EntryStatus::CALCULATING {
                        std::thread::sleep(std::time::Duration::from_millis(10)); // TODO: replace with a condition variable
                    }
                    return (unsafe { &*(&cache_entry.data as *const D) }, cache_entry.adhoc_code);
                }
                _ => {}
            }
            return (unsafe { &*(&cache_entry.data as *const D) }, cache_entry.adhoc_code);
        }      
    
        // Miss
        let mut entry: Entry<D> = Entry::default();
        entry.status = EntryStatus::CALCULATING;
        if miss_handler(&key, &mut entry.data, &mut entry.adhoc_code) {
            entry.expiration = Instant::now() + positive_ttl;
            entry.status = EntryStatus::READY;
        } else {
            entry.expiration = Instant::now() + negative_ttl;
            entry.status = EntryStatus::FAILED;
        }
    
        // Insert new entry
        let mut binding = self.lru_cache.write().unwrap();
        let cache_entry = binding.get_or_insert_mut(*key, || entry);
        (unsafe { &*(&cache_entry.data as *const D) }, cache_entry.adhoc_code)

    }
}

// ===============================================================
// =============================TESTS=============================
// ===============================================================

#[cfg(test)]
mod tests {

    use std::thread;

    use super::*;
    use rstest::*;

    #[fixture]
    fn simple_cache() -> Cache<i32, i32> {
        fn miss_handler(key: &i32, data: &mut i32, adhoc_code: &mut u8) -> bool {
            // FAIL if key is -1
            if *key == -1 {
                return false
            }
            // take computing time if key is 456:
            if *key == 456 {
                std::thread::sleep(std::time::Duration::from_millis(1000));
            }

            *data = key * 2;
            *adhoc_code += 1; // should always be 1
            true
        }
        Cache::new(
            3,
            miss_handler,
            Duration::from_millis(200),          
            Duration::from_millis(100),          
        )
    }

    #[rstest]
    fn insert_value(simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        simple_cache.insert(&key, &value);

        // Assert
        assert_eq!(simple_cache.len(), 1);
    }

    #[rstest]
    fn insert_same_key(simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        simple_cache.insert(&key, &value);
        simple_cache.insert(&key, &value);

        // Assert
        assert_eq!(simple_cache.len(), 1);
    }

    #[rstest]
    fn get_value(simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        simple_cache.insert(&key, &value);

        // Assert
        assert_eq!(simple_cache.get(&key), Some(value));
    }

    #[rstest]
    fn get_value_not_found(simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;

        // Assert
        assert_eq!(simple_cache.get(&key), None);
    }

    #[rstest]
    fn insert_max_capacity(simple_cache: Cache<i32, i32>) {
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
        assert_eq!(simple_cache.len(), 3);
        assert_eq!(simple_cache.get(&key1), None); // lru is removed
    }

    #[rstest]
    fn get_lru_change(simple_cache: Cache<i32, i32>) {
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
        assert_eq!(simple_cache.len(), 3);
        assert_eq!(simple_cache.get(&key2), None); // lru is removed
    }

    #[rstest]
    fn ttl_expired(simple_cache: Cache<i32, i32>) {
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
    fn retrieve_or_compute_not_in_cache(simple_cache: Cache<i32, i32>){
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
    fn retrieve_or_compute_already_in_cache(simple_cache: Cache<i32, i32>){
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
    fn retrieve_or_compute_ttl_expired(simple_cache: Cache<i32, i32>){
        // Arrange
        let key = 1;

        // Act
        simple_cache.retrieve_or_compute(&key);
        let entry_1 = simple_cache.lru_cache.read().unwrap().peek(&key).unwrap().clone();
        std::thread::sleep(std::time::Duration::from_millis(100));
        simple_cache.retrieve_or_compute(&key);
        let entry_2 = simple_cache.lru_cache.read().unwrap().peek(&key).unwrap().clone();
        std::thread::sleep(std::time::Duration::from_millis(150));
        simple_cache.retrieve_or_compute(&key);
        let entry_3 = simple_cache.lru_cache.read().unwrap().peek(&key).unwrap().clone();
        
        // Assert
        assert_eq!(entry_1.status, EntryStatus::READY);
        assert_eq!(entry_1, entry_2); // not expired
        assert_ne!(entry_1, entry_3); // expired 
    }

    #[rstest]
    fn retrieve_or_compute_negative_ttl(simple_cache: Cache<i32, i32>){
        // Arrange
        let key = -1;

        // Act
        simple_cache.retrieve_or_compute(&key);
        let entry_1 = simple_cache.lru_cache.read().unwrap().peek(&key).unwrap().clone();
        std::thread::sleep(std::time::Duration::from_millis(105));
        simple_cache.retrieve_or_compute(&key);
        let entry_2 = simple_cache.lru_cache.read().unwrap().peek(&key).unwrap().clone();
        
        // Assert
        assert_ne!(entry_1, entry_2); // expired because negative ttl is lower
        assert_eq!(entry_1.status, EntryStatus::FAILED);
    }

    #[rstest]
    fn test_thread_safe_cache(simple_cache: Cache<i32, i32>) {
        // Arrange
        let cache = Arc::new(simple_cache);
        // Act
        let handles: Vec<_> = (0..10).map(|_| {
            let cache_clone = Arc::clone(&cache);
            thread::spawn(move || {
                let key = 456;
                cache_clone.retrieve_or_compute(&key);
            })
        }).collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Assert
        let key = 456;
        let (data, code) = cache.retrieve_or_compute(&key);
        assert_eq!(*data, key * 2);
        assert_eq!(code, 1);
    }

}
