use lru::{LruCache, DefaultHasher};
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::ops::DerefMut;
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
    cond_var: Arc<Condvar>
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
            cond_var: Arc::new(Condvar::new())
        }
    }


    fn new(data: D, expiration: Instant, adhoc_code: u8) -> Self {
        Entry {
            data,
            expiration,
            adhoc_code,
            status: EntryStatus::AVAILABLE,
            cond_var: Arc::new(Condvar::new())
        }
    }

    fn is_valid(&self) -> bool {
        self.expiration > Instant::now()
    }

}

type MissHandler<K, D> = fn(&K, &mut D, &mut u8) -> bool;

pub struct Cache<K, D> {
    lru_cache: Arc<RwLock<LruCache<K, Arc<Mutex<Entry<D>>>>>>,
    miss_handler: MissHandler<K, D>,
    positive_ttl: Duration, // seconds
    negative_ttl: Duration, // seconds
}

impl<K: Eq + Hash + Clone, D: Eq + Default + Clone> Cache<K, D> {
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

    pub fn insert(&self, key: &K, data: D) {
        let expiration = Instant::now() + self.positive_ttl;
        let entry = Entry::new(data, expiration, 0);
        let entry_arc = Arc::new(Mutex::new(entry));
        self.lru_cache.write().unwrap().put(key.clone(), entry_arc);        
    }

    pub fn get(&self, key: &K) -> Option<D> {
        if let Some(entry_arc) = self.get_entry(&key) {
            let entry = entry_arc.lock().unwrap();
            return Some(entry.data.clone());
        }
        None
    }

    fn get_entry(&self, key: &K) -> Option<Arc<Mutex<Entry<D>>>> {
        // lock the cache
        let mut cache = self.lru_cache.write().unwrap();
        // check if the entry exists and is valid
        if let Some(entry_arc) = cache.get(key) {
            let entry = entry_arc.lock().unwrap();
            if entry.is_valid() {
                return Some(Arc::clone(&entry_arc));
            }
        }
        // if the entry is not valid or does not exist, remove it
        cache.pop(key);
        None
    }

    pub fn len(&self) -> usize {
        self.lru_cache.read().unwrap().len()
    }

    fn handle_hit(&self, key: &K) -> Option<(D, u8)> {
        // check if the the entry exists and is valid
        if let Some(entry_arc) = self.get_entry(key) {
            let entry_arc_clone = Arc::clone(&entry_arc);
            let entry = entry_arc.lock().unwrap();
            match entry.status {
                EntryStatus::AVAILABLE => {
                    // should not happen
                    eprintln!("Error: entry should not be available at this point");
                    return None;
                }
                EntryStatus::CALCULATING => {
                    println!("CALCULATING");
                    match entry.cond_var.wait_while(
                        entry_arc_clone.lock().unwrap(), 
                        |entry: &mut Entry<D>| entry.status == EntryStatus::CALCULATING)
                        {
                            Ok(_) => {}
                            Err(e) => {
                                eprintln!("Error while waiting: {:?}", e);
                                return None;
                            }
                        }
                }
                _ => {}
            }
            return Some((entry.data.clone(), entry.adhoc_code));
        }
        return None;
    }

    pub fn retrieve_or_compute(&self, key: &K) -> Option<(D, u8)> {
        let miss_handler = self.miss_handler;
        let positive_ttl = self.positive_ttl;
        let negative_ttl = self.negative_ttl;

        if let Some((data, adhoc_code)) = self.handle_hit(&key) {
            return Some((data, adhoc_code));
        }

        // Miss
        let entry_arc = {
            let mut locked_cache = self.lru_cache.write().unwrap();
            let entry_arc = locked_cache.get_or_insert_mut(key.clone(), || Arc::new(Mutex::new(Entry::default())));
            Arc::clone(&entry_arc)
        };

        let mut locked_entry = entry_arc.lock().unwrap();
        let entry_arc_clone = Arc::clone(&entry_arc);
        let entry = locked_entry.deref_mut();

        match entry.status {
            EntryStatus::AVAILABLE => {
                {
                    entry.status = EntryStatus::CALCULATING;                   
                    if miss_handler(&key, &mut entry.data, &mut entry.adhoc_code) {
                        entry.expiration = Instant::now() + positive_ttl;
                        entry.status = EntryStatus::READY;
                    } else {
                        entry.expiration = Instant::now() + negative_ttl;
                        entry.status = EntryStatus::FAILED;
                    }
                }
                entry.cond_var.notify_all();
            }
            EntryStatus::CALCULATING => {
                println!("CALCULATING");
                match entry.cond_var.wait_while(
                    entry_arc_clone.lock().unwrap(), 
                    |entry: &mut Entry<D>| entry.status == EntryStatus::CALCULATING)
                    {
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("Error while waiting: {:?}", e);
                            return None;
                        }
                    }
            }
            EntryStatus::READY | EntryStatus::FAILED => {}
        }
        
        
        Some((entry.data.clone(), entry.adhoc_code))
    }
}

// ===============================================================
// =============================TESTS=============================
// ===============================================================

#[cfg(test)]
mod tests {

    use std::{result, thread};

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
        simple_cache.insert(&key, value);

        // Assert
        assert_eq!(simple_cache.len(), 1);
    }

    #[rstest]
    fn insert_same_key(simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        simple_cache.insert(&key, value);
        simple_cache.insert(&key, value);

        // Assert
        assert_eq!(simple_cache.len(), 1);
    }

    #[rstest]
    fn get_value(simple_cache: Cache<i32, i32>) {
        // Arrange
        let key = 1;
        let value = 2;

        // Act
        simple_cache.insert(&key, value);

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
        simple_cache.insert(&key1, value);
        simple_cache.insert(&key2, value);
        simple_cache.insert(&key3, value);
        simple_cache.insert(&key4, value);

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
        simple_cache.insert(&key1, value);
        simple_cache.insert(&key2, value);
        simple_cache.get(&key1); // key2 is now the lru
        simple_cache.insert(&key3, value);
        simple_cache.insert(&key4, value);

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
        simple_cache.insert(&key, value);
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
        let entry_1 = simple_cache.lru_cache.read().unwrap().peek(&key).unwrap().lock().unwrap().clone();
        std::thread::sleep(std::time::Duration::from_millis(100));
        simple_cache.retrieve_or_compute(&key);
        let entry_2 = simple_cache.lru_cache.read().unwrap().peek(&key).unwrap().lock().unwrap().clone();
        std::thread::sleep(std::time::Duration::from_millis(150));
        simple_cache.retrieve_or_compute(&key);
        let entry_3 = simple_cache.lru_cache.read().unwrap().peek(&key).unwrap().lock().unwrap().clone();
        
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
        let entry_1 = simple_cache.lru_cache.read().unwrap().peek(&key).unwrap().lock().unwrap().clone();
        std::thread::sleep(std::time::Duration::from_millis(105));
        simple_cache.retrieve_or_compute(&key);
        let entry_2 = simple_cache.lru_cache.read().unwrap().peek(&key).unwrap().lock().unwrap().clone();
        
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
            200,
            miss_handler,
            Duration::from_secs(60),          
            Duration::from_secs(60),          
        )
    }

    #[rstest]
    fn test_thread_safe_cache_same_key(time_consuming_mh: Cache<i32, i32>) {        
        // Arrange            
        let cache = Arc::new(time_consuming_mh);
        let n_threads: i32 = 200;
        let start = Instant::now();

        // Act
        let handles: Vec<_> = (0..n_threads).map(|i| {
            let cache_clone = Arc::clone(&cache);
            thread::Builder::new().name(format!("Thread {}", i)).spawn(move || {
            let key = 456;
            cache_clone.retrieve_or_compute(&key)
            })
        }).collect();

        // Assert
        let results = handles.into_iter().map(|handle| handle.unwrap().join());
        for res in results {
            // assert res is ok
            assert!(res.is_ok());
            if let Some((data, adhoc_code)) = res.unwrap() {
                assert_eq!(data, 456 * 2);
                assert_eq!(adhoc_code, 1);
            }
        }        
        let duration = start.elapsed();
        assert!(cache.len() == 1);
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
        assert!(cache.len() == n_threads.try_into().unwrap());
        let duration = start.elapsed();
        assert!(duration.as_secs() < 1);
    }

    #[rstest]
    fn test_thread_safe_cache_maximum_capacity(time_consuming_mh: Cache<i32, i32>) {
        // Arrange
        let cache = Arc::new(time_consuming_mh);
        let n_threads = 202;
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
        assert!(cache.len() == 200);
        assert!(duration.as_secs() < 1);
        assert_eq!(not_in_cache_count, 2);
    }

    #[rstest]
    fn test_thread_safe_heavy_threads(time_consuming_mh: Cache<i32, i32>) {
        let cache = Arc::new(time_consuming_mh);
        for _ in 0..50 {            
            // Arrange
            let n_keys = 5;
            let entries_per_key = 20;
            let results = vec![(0,0); n_keys * entries_per_key];
            let results_arc = Arc::new(RwLock::new(results));
            let mut threads = Vec::<thread::JoinHandle<_>>::with_capacity(n_keys * entries_per_key);

            // Act
            for i in 0..n_keys {
                for j in 0..entries_per_key {
                    let cache_clone = Arc::clone(&cache);
                    let results_clone = Arc::clone(&results_arc);
                    threads.push(thread::spawn(move || {
                        let key = (i + 1) as i32;
                        let (data, adhoc_code) = cache_clone.retrieve_or_compute(&key).unwrap();
                        let mut results = results_clone.write().unwrap();
                        results[i*entries_per_key+j] = (data, adhoc_code);
                    }));
                }
            }
            for handle in threads {
                let res = handle.join();
                // assert res is ok
                assert!(res.is_ok());
                res.unwrap();
            }

            // Assert
            for i in (0..n_keys * entries_per_key).step_by(entries_per_key) {
                let results = results_arc.read().unwrap();
                let res_i = results[i];
                for j in 1..entries_per_key {
                    let res_j = results[i+j];
                    assert_eq!(res_i, res_j);
                }
            }
            assert!(cache.len() == n_keys);
        }
    }

    #[derive(Debug, PartialEq, Eq, Clone, Hash, Copy, Default)]
    struct SimpleStruct {
        value: i32,
    }
    #[derive(Debug, PartialEq, Eq, Clone, Hash, Default)]
    struct ComplexKey {
        id: i32,
        name: String,
        nested: SimpleStruct,
        array: Vec<i32>,

    }

  #[derive(Debug, PartialEq, Eq, Clone, Hash, Default)]
    struct ComplexData {
        value: i32,
        description: String,
        nested: SimpleStruct,
        array: Vec<i32>,
    }

    #[fixture]
    fn complex_key_and_data_cache() -> Cache<ComplexKey, ComplexData> {
        fn miss_handler(key: &ComplexKey, data: &mut ComplexData, adhoc_code: &mut u8) -> bool {
            // wait 500 ms
            std::thread::sleep(std::time::Duration::from_millis(500));
            // FAIL if key.id is -1
            if key.id == -1 {
                return false
            }
            data.value = key.id * 2;
            data.description = key.name.clone();
            data.nested = key.nested.clone();
            data.array = key.array.clone();
            *adhoc_code += 1; // should always be 1
            true
        }
        Cache::new(
            200,
            miss_handler,
            Duration::from_secs(1),          
            Duration::from_secs(1),          
        )
    }

    #[rstest]
    fn complex_key_and_data_cache_insert(complex_key_and_data_cache: Cache<ComplexKey, ComplexData>) {
        // Arrange
        let key = ComplexKey {
            id: 1,
            name: "name".to_string(),
            nested: SimpleStruct { value: 1 },
            array: vec![1, 2, 3],
        };
        let data = ComplexData {
            value: 2,
            description: "name".to_string(),
            nested: SimpleStruct { value: 1 },
            array: vec![1, 2, 3],
        };

        // Act
        complex_key_and_data_cache.insert(&key, data);

        // Assert
        assert_eq!(complex_key_and_data_cache.len(), 1);
    }

    #[rstest]
    fn complex_key_data_retrieve_or_compute(complex_key_and_data_cache: Cache<ComplexKey, ComplexData>) {
        // Arrange
        let key = ComplexKey {
            id: 1,
            name: "name".to_string(),
            nested: SimpleStruct { value: 1 },
            array: vec![1, 2, 3],
        };

        // Act
        let (data, adhoc_code) = complex_key_and_data_cache.retrieve_or_compute(&key).unwrap();

        // Assert
        assert_eq!(data.value, 2);
        assert_eq!(data.description, "name");
        assert_eq!(data.nested, SimpleStruct { value: 1 });
        assert_eq!(data.array, vec![1, 2, 3]);
        assert_eq!(adhoc_code, 1);
        assert_eq!(complex_key_and_data_cache.len(), 1);    
    }

    #[rstest]
    fn complex_key_data_retrieve_or_compute_change_key(complex_key_and_data_cache: Cache<ComplexKey, ComplexData>) {
        // Arrange
        let key = ComplexKey {
            id: 1,
            name: "name".to_string(),
            nested: SimpleStruct { value: 1 },
            array: vec![1, 2, 3],
        };
        let key_clone = key.clone();
        let mut key_change = key.clone();
        key_change.id = 2;

        // Act
        let (data, _) = complex_key_and_data_cache.retrieve_or_compute(&key).unwrap();
        let (data1, _) = complex_key_and_data_cache.retrieve_or_compute(&key_clone).unwrap();
        let (data2, _) = complex_key_and_data_cache.retrieve_or_compute(&key_change).unwrap();

        // Assert
        assert_eq!(data, data1);
        assert_ne!(data, data2);
        assert_eq!(complex_key_and_data_cache.len(), 2);
    }

    #[rstest]
    fn complex_key_data_thread_safe_cache_same_key(complex_key_and_data_cache: Cache<ComplexKey, ComplexData>) {
        // Arrange
        let cache = Arc::new(complex_key_and_data_cache);
        let n_threads: i32 = 200;
        let start = Instant::now();
        let key = ComplexKey {
            id: 1,
            name: "name".to_string(),
            nested: SimpleStruct { value: 1 },
            array: vec![1, 2, 3],
        };

        // Act
        let handles: Vec<_> = (0..n_threads).map(|i| {
            let cache_clone = Arc::clone(&cache);
            thread::Builder::new().name(format!("Thread {}", i)).spawn({
            let value = key.clone();
            move || {
            cache_clone.retrieve_or_compute(&value)
            }
            })
        }).collect();

        // Assert
        let results = handles.into_iter().map(|handle| handle.unwrap().join());
        for res in results {
            // assert res is ok
            assert!(res.is_ok());
            if let Some((data, adhoc_code)) = res.unwrap() {
                assert_eq!(data.value, 2);
                assert_eq!(data.description, "name");
                assert_eq!(data.nested, SimpleStruct { value: 1 });
                assert_eq!(data.array, vec![1, 2, 3]);
                assert_eq!(adhoc_code, 1);
            }
        }        
        let duration = start.elapsed();
        assert!(cache.len() == 1);
        assert!(duration.as_secs() < 1);
        
    }

    #[rstest]
    fn complex_key_data_thread_safe_cache_different_keys(complex_key_and_data_cache: Cache<ComplexKey, ComplexData>) {
        // Arrange
        let cache = Arc::new(complex_key_and_data_cache);
        let n_threads = 200;
        let start = Instant::now();

        // Act
        let handles: Vec<_> = (0..n_threads).map(|i| {
            let cache_clone = Arc::clone(&cache);
            let key: ComplexKey = ComplexKey {
                id: 1 + i,
                name: "name".to_string(),
                nested: SimpleStruct { value: 1 },
                array: vec![1, 2, 3],
            };
            thread::spawn({ move || {
            cache_clone.retrieve_or_compute(&key)
            }
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
            let key = ComplexKey {
                id: 1 + i,
                name: "name".to_string(),
                nested: SimpleStruct { value: 1 },
                array: vec![1, 2, 3],
            };
            let data = cache.get(&key);

            assert_eq!(data, Some(ComplexData {
                value: (1 + i) * 2,
                description: "name".to_string(),
                nested: SimpleStruct { value: 1 },
                array: vec![1, 2, 3],
            }));
        }
        assert!(cache.len() == n_threads.try_into().unwrap());
        let duration = start.elapsed();
        assert!(duration.as_secs() < 1);
    }


}
