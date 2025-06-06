use civicjournal_time::storage::memory::MemoryStorage;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::page::JournalPage;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids, get_test_config};
use chrono::Utc;

use futures::future::join_all;
#[tokio::test]
async fn test_fail_on_store_and_clear() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let page = JournalPage::new(0, None, Utc::now(), cfg);

    storage.set_fail_on_store(0, Some(page.page_id));
    let res = storage.store_page(&page).await;
    assert!(res.is_err(), "store_page should fail when fail_on_store is set");

    storage.clear_fail_on_store();
    storage.store_page(&page).await.unwrap();
}

#[tokio::test]
async fn test_list_finalized_pages_summary_sorted() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let now = Utc::now();

    let mut first = JournalPage::new(0, None, now, cfg);
    first.recalculate_merkle_root_and_page_hash();
    let mut second = JournalPage::new(0, Some(first.page_hash), now, cfg);
    second.recalculate_merkle_root_and_page_hash();

    storage.store_page(&second).await.unwrap();
    storage.store_page(&first).await.unwrap();

    let summaries = storage.list_finalized_pages_summary(0).await.unwrap();
    assert_eq!(summaries.len(), 2);
    assert!(summaries[0].page_id < summaries[1].page_id);
}

#[tokio::test]
async fn test_backup_and_restore_behavior() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let dir = tempfile::tempdir().unwrap();
    let backup_path = dir.path().join("backup.zip");

    storage.backup_journal(&backup_path).await.unwrap();
    let res = storage.restore_journal(&backup_path, dir.path()).await;
    assert!(res.is_err(), "restore_journal should return an error for MemoryStorage");
}

#[tokio::test]
async fn test_load_page_by_hash() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let mut page = JournalPage::new(0, None, Utc::now(), cfg);
    page.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page).await.unwrap();

    let loaded = storage.load_page_by_hash(page.page_hash).await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().page_id, page.page_id);
}

#[tokio::test]
async fn test_load_leaf_by_hash() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let mut page = JournalPage::new(0, None, Utc::now(), cfg);
    let leaf = civicjournal_time::core::leaf::JournalLeaf::new(
        Utc::now(),
        None,
        "c1".into(),
        serde_json::json!({"foo": 1}),
    )
    .unwrap();
    if let civicjournal_time::core::page::PageContent::Leaves(ref mut v) = page.content {
        v.push(leaf.clone());
    }
    page.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page).await.unwrap();

    let found = storage.load_leaf_by_hash(&leaf.leaf_hash).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().leaf_id, leaf.leaf_id);
}

#[tokio::test]
async fn test_page_exists_and_delete() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let page = JournalPage::new(0, None, Utc::now(), cfg);

    storage.store_page(&page).await.unwrap();
    assert!(storage
        .page_exists(page.level, page.page_id)
        .await
        .unwrap());

    storage.delete_page(page.level, page.page_id).await.unwrap();
    assert!(!storage
        .page_exists(page.level, page.page_id)
        .await
        .unwrap());
}

#[tokio::test]
async fn test_is_empty_and_clear() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let page = JournalPage::new(0, None, Utc::now(), cfg);

    assert!(storage.is_empty());
    storage.store_page(&page).await.unwrap();
    assert!(!storage.is_empty());
    storage.clear();
    assert!(storage.is_empty());
}
#[tokio::test]
async fn test_fail_on_store_any_page() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let page = JournalPage::new(0, None, Utc::now(), cfg);

    storage.set_fail_on_store(0, None);
    assert!(storage.store_page(&page).await.is_err());

    storage.clear_fail_on_store();
    storage.store_page(&page).await.unwrap();
}

#[tokio::test]
async fn test_fail_condition_non_matching_page() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let page1 = JournalPage::new(0, None, Utc::now(), cfg);
    let page2 = JournalPage::new_with_id(page1.page_id + 1, 0, None, Utc::now(), cfg);

    storage.set_fail_on_store(0, Some(page2.page_id));

    // Should succeed because the fail condition targets a different page
    storage.store_page(&page1).await.unwrap();

    // Storing the targeted page should fail
    assert!(storage.store_page(&page2).await.is_err());

    storage.clear_fail_on_store();
}
#[tokio::test]
async fn test_concurrent_store_and_load() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let storage = MemoryStorage::new();
    let cfg = get_test_config();
    let mut handles = Vec::new();

    for i in 0..5u64 {
        let storage_clone = storage.clone();
        let cfg_ref = cfg;
        handles.push(tokio::spawn(async move {
            let page = JournalPage::new_with_id(i, 0, None, Utc::now(), cfg_ref);
            storage_clone.store_page(&page).await.unwrap();
            page.page_id
        }));
    }

    let ids: Vec<u64> = join_all(handles).await.into_iter().map(|r| r.unwrap()).collect();

    for id in ids {
        assert!(storage.page_exists(0, id).await.unwrap());
    }
}
