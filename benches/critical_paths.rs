use criterion::{criterion_group, criterion_main, Criterion, black_box};
use civicjournal_time::core::time_manager::TimeHierarchyManager;
use civicjournal_time::storage::memory::MemoryStorage;
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::query::QueryEngine;
use civicjournal_time::test_utils::{reset_global_ids, get_test_config};
use chrono::{Utc, Duration};
use serde_json::json;
use std::sync::Arc;
use tokio::runtime::Runtime;

fn bench_add_leaf(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let config = get_test_config().clone();
    let storage = Arc::new(MemoryStorage::new());
    let manager = Arc::new(TimeHierarchyManager::new(Arc::new(config), storage.clone()));
    c.bench_function("add_leaf", |b| {
        b.to_async(&rt).iter(|| async {
            let ts = Utc::now();
            let leaf = JournalLeaf::new(ts, None, "bench".to_string(), json!({"v": 1})).unwrap();
            let _ = manager.add_leaf(black_box(&leaf), black_box(ts)).await.unwrap();
        });
    });
}

fn bench_reconstruct_state(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    reset_global_ids();
    let config = get_test_config().clone();
    let storage = Arc::new(MemoryStorage::new());
    let manager = Arc::new(TimeHierarchyManager::new(Arc::new(config.clone()), storage.clone()));
    let engine = QueryEngine::new(storage.clone(), manager.clone(), Arc::new(config));

    // Prepopulate with 100 leaves
    rt.block_on(async {
        let base = Utc::now();
        for i in 0..100u32 {
            let ts = base + Duration::seconds(i as i64);
            let leaf = JournalLeaf::new(ts, None, "c1".to_string(), json!({"v": i})).unwrap();
            manager.add_leaf(&leaf, ts).await.unwrap();
        }
    });

    let at_time = Utc::now();
    c.bench_function("reconstruct_container_state", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = engine.reconstruct_container_state(black_box("c1"), black_box(at_time)).await.unwrap();
        });
    });
}

criterion_group!(benches, bench_add_leaf, bench_reconstruct_state);
criterion_main!(benches);
