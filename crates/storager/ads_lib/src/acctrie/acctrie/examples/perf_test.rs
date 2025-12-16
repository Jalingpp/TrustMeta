use std::time::Instant;
use acctrie::AccTrie;

// Simple performance harness for CRUD operations on AccTrie.
// Configure number of keys with the environment variable `PERF_N` (default 2000).
fn main() {
    let n: usize = std::env::var("PERF_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);

    println!("Performance test: N = {}", n);

    let mut t = AccTrie::new();

    // Bulk insert
    let start = Instant::now();
    for i in 0..n {
        let key = format!("k{:08}", i);
        t.insert(&key, i as i64);
    }
    let dur = start.elapsed();
    println!("insert {} keys: {:?}", n, dur);

    // Sampling queries (1000 or less)
    let q = std::cmp::min(1000, n);
    let step = if q > 0 { n / q } else { 1 };
    let start = Instant::now();
    let mut found = 0usize;
    let mut i = 0usize;
    while i < n && found < q {
        let key = format!("k{:08}", i);
        if let Some((_vals, _acc, _p, _n)) = t.query(&key) {
            found += 1;
        }
        i += step.max(1);
    }
    let dur = start.elapsed();
    println!("{} queries sampled: {:?}", found, dur);

    // Bulk updates: replace each value with value+1
    let start = Instant::now();
    for i in 0..n {
        let key = format!("k{:08}", i);
        let _ = t.update(&key, Some(&[i as i64 + 1]), None);
    }
    let dur = start.elapsed();
    println!("update {} keys: {:?}", n, dur);

    // Bulk deletes: delete every key
    let start = Instant::now();
    for i in 0..n {
        let key = format!("k{:08}", i);
        let _ = t.delete(&key, None);
    }
    let dur = start.elapsed();
    println!("delete {} keys: {:?}", n, dur);
}
