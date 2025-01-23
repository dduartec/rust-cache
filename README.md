# Rust Cache

This project is a simple caching library written in Rust. It provides an easy-to-use interface for storing and retrieving data with high performance. The library supports various caching strategies and is designed to be lightweight and efficient.

## Features

- In-memory caching
- Configurable cache size
- Expiration policies
- Thread-safe operations

## Installation

Add the following to your `Cargo.toml`:

```toml
[dependencies]
rust-cache = "0.1.0"
```

## Usage

Here's a basic example of how to use the Rust Cache library:

```rust
use gs_rust_cache::Cache;

fn miss_handler(key: &i32, data: &mut i32, adhoc_code: &mut u8, _: &[&dyn Any]) -> bool {
    // Your Code Here
    *data = 123;
    *adhoc_code = 200;
    true
}

fn main() {
    let mut cache = Cache<i32, i32>::new(
        size: 3,
        miss_handler,
        positive_ttl: Duration::from_millis(200),          
        negative_ttl: Duration::from_millis(100),          
    );

    let key =  456;
    let value = cache.retrieve_or_compute(&key); // first one is calculated
    let value_1 = cache.retrieve_or_compute(&key); // afterwards it is retrieved

    assert_eq!(value, value_1);    
}
```

## License

This project is licensed under the MIT License.