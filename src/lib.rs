use lru::{LruCache, DefaultHasher};
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};
use std::sync::{Arc, Condvar, Mutex, RwLock};

#[derive(Debug, PartialEq, Clone, Copy)]
enum EntryStatus {
    AVAILABLE,
    CALCULATING,
    READY,
    FAILED,
}

#[derive(Debug, Clone)]
struct Entry<D> {
    data: D,
    adhoc_code: u8,
    expiration: Instant,
    status: EntryStatus,
}

impl<D: PartialEq> PartialEq for Entry<D> {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data &&
        self.adhoc_code == other.adhoc_code &&
        self.expiration == other.expiration &&
        self.status == other.status
    }
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
            let cache = self.lru_cache.read().unwrap();
            let cache_entry = cache.peek(&key).unwrap();
            return Some(unsafe { *(&cache_entry.data as *const D) });
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

    fn handle_hit(&self, key: &K) -> Option<Entry<D>> {
        let is_in_cache = self.is_in_cache(key);
        
        let cache = self.lru_cache.read().unwrap();
        // First, check if the entry exists and is valid        
        if is_in_cache {           
            // Hit
            let cache_entry = cache.peek(&key).unwrap();
            match cache_entry.status {
                EntryStatus::CALCULATING => {
                    println!("CALCULATING");
                    // cache_entry.wait();
                    return Some(cache_entry.clone());
                }
                _ => {}
            }
            return Some(cache_entry.clone());
        };
        return None;

    }

    pub fn retrieve_or_compute(&self, key: &K) -> Option<(D, u8)> {
        let miss_handler = self.miss_handler;
        let positive_ttl = self.positive_ttl;
        let negative_ttl = self.negative_ttl;

        if let Some(cache_entry) = self.handle_hit(key) {
            return Some((cache_entry.data, cache_entry.adhoc_code));
        }
      
        println!("MISS");
        // Miss
        let status = {
            let mut binding = self.lru_cache.write().unwrap();
            let cache_entry = binding.get_or_insert_mut(*key, || Entry::default());
            if cache_entry.status == EntryStatus::AVAILABLE {
                cache_entry.status = EntryStatus::CALCULATING;
                EntryStatus::AVAILABLE
            } else {
                cache_entry.status
            }
        };

        match status {
            EntryStatus::CALCULATING => {
                let cache = self.lru_cache.read().unwrap();
                println!("CALCULATING 2");
                //wait for the entry to change status                
                let cache_entry = cache.peek(&key).unwrap();
                // cache_entry.wait();
                return Some((cache_entry.data, cache_entry.adhoc_code));
            }
            EntryStatus::READY | EntryStatus::FAILED => {
                let cache = self.lru_cache.read().unwrap();
                let cache_entry = cache.peek(&key).unwrap();
                return Some((cache_entry.data, cache_entry.adhoc_code));
            }
            _ => {}
        }

        

        let entry = Entry::default();
        let mut entry = Entry::new(entry.data, Instant::now(), entry.adhoc_code);
        if miss_handler(&key, &mut entry.data, &mut entry.adhoc_code) {
            entry.expiration = Instant::now() + positive_ttl;
            entry.status = EntryStatus::READY;
        } else {
            entry.expiration = Instant::now() + negative_ttl;
            entry.status = EntryStatus::FAILED;
        }


        {
            let mut cache = self.lru_cache.write().unwrap();
            let cache_entry = cache.peek_mut(key).unwrap();

            cache_entry.data = entry.data;
            cache_entry.adhoc_code = entry.adhoc_code;
            cache_entry.expiration = entry.expiration;
            cache_entry.status = entry.status;
            // cache_entry.notify();
        }

        let binding = self.lru_cache.read().unwrap();
        let cache_entry = binding.peek(&key).unwrap();
        
        
        Some((cache_entry.data, cache_entry.adhoc_code))
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
        let (data, adhoc_code) = simple_cache.retrieve_or_compute(&key).unwrap();

        // Assert
        assert_eq!(data, 2);
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
        let (data, adhoc_code) = simple_cache.retrieve_or_compute(&key).unwrap();

        // Assert
        assert_eq!(data, 2);
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

    #[fixture]
    fn time_consuming_mh() -> Cache<i32, i32> {
        fn miss_handler(key: &i32, data: &mut i32, adhoc_code: &mut u8) -> bool {

            std::thread::sleep(std::time::Duration::from_millis(500));

            *data = key * 2;
            *adhoc_code += 1; // should always be 1
            true
        }
        Cache::new(
            20,
            miss_handler,
            Duration::from_secs(60),          
            Duration::from_secs(60),          
        )
    }

    #[rstest]
    fn test_thread_safe_cache_same_key(time_consuming_mh: Cache<i32, i32>) {
        // Arrange
        let cache = Arc::new(time_consuming_mh);
        let n_threads: i32 = 2;
        let start = Instant::now();

        // Act
        let handles: Vec<_> = (0..n_threads).map(|_| {
            let cache_clone = Arc::clone(&cache);
            thread::spawn(move || {
            let key = 456;
            cache_clone.retrieve_or_compute(&key)
            })
        }).collect();

        // Assert
        for handle in handles {
            let res = handle.join();
            // assert res is ok
            assert!(res.is_ok());
            if let Some((data, adhoc_code)) = res.unwrap() {
                assert_eq!(data, 456 * 2);
                assert_eq!(adhoc_code, 1);
            }
        }        
        let duration = start.elapsed();
        assert!(duration.as_secs() < 1);
    }

    #[rstest]
    fn test_thread_safe_cache_different_keys(time_consuming_mh: Cache<i32, i32>) {
        // Arrange
        let cache = Arc::new(time_consuming_mh);
        let n_threads = 20;
        let start = Instant::now();

        // Act
        let handles: Vec<_> = (0..n_threads).map(|i| {
            let cache_clone = Arc::clone(&cache);
            thread::spawn(move || {
                let key = 456 + i;
                cache_clone.retrieve_or_compute(&key);
            })
        }).collect();

        for handle in handles {
            let res = handle.join();
            // assert res is ok
            assert!(res.is_ok());
            res.unwrap();
        }

        // Assert        
        for i in 0..n_threads {
            let key = 456 + i;
            let data = cache.get(&key);
            assert_eq!(data, Some(key * 2));
        }
        let duration = start.elapsed();
        assert!(duration.as_secs() < 1);
    }

    #[rstest]
    fn test_thread_safe_cache_maximum_capacity(time_consuming_mh: Cache<i32, i32>) {
        // Arrange
        let cache = Arc::new(time_consuming_mh);
        let n_threads = 22;
        let start = Instant::now();

        // Act
        let handles: Vec<_> = (0..n_threads).map(|i| {
            let cache_clone = Arc::clone(&cache);
            thread::spawn(move || {
                let key = 456 + i;
                cache_clone.retrieve_or_compute(&key);
            })
        }).collect();

        for handle in handles {
            let res = handle.join();
            // assert res is ok
            assert!(res.is_ok());
            res.unwrap();
        }

        // Assert
        let mut not_in_cache_count = 0;      
        for i in 0..n_threads {
            let key = 456 + i;
            let data = cache.get(&key);
            if data == None {
                not_in_cache_count += 1;
            } else {
                let key = 456 + i;
                let data = cache.get(&key);
                assert_eq!(data, Some(key * 2));
            }
        }
        let duration = start.elapsed();
        assert!(duration.as_secs() < 1);
        assert_eq!(not_in_cache_count, 2);
    }

}
