use civicjournal_time::storage::file::FileStorage;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::core::page::{JournalPage, PageContent};
use civicjournal_time::CompressionAlgorithm;
use civicjournal_time::config::{CompressionConfig, Config};
use civicjournal_time::error::CJError;
use civicjournal_time::test_utils::{reset_global_ids, SHARED_TEST_ID_MUTEX};
use chrono::Utc;
use serde_json::json;
use tempfile::tempdir;
use std::path::Path;
use std::io::Read;

// Helper to create config with desired compression
fn cfg(algo: CompressionAlgorithm, enabled: bool) -> Config {
    Config {
        compression: CompressionConfig { enabled, algorithm: algo, level: 3 },
        ..Default::default()
    }
}

// Helper to make a simple L0 page with one leaf
fn make_page(id: u64, cfg: &Config) -> JournalPage {
    let ts = Utc::now();
    let mut page = JournalPage::new_with_id(id, 0, None, ts, cfg);
    let leaf = JournalLeaf::new(ts, None, "c".into(), json!({"k":"v"})).unwrap();
    if let PageContent::Leaves(ref mut v) = page.content { v.push(leaf); }
    page.recalculate_merkle_root_and_page_hash();
    page
}

#[tokio::test]
async fn test_new_creates_marker() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
    assert!(dir.path().join(".civicjournal-time").exists());
}

#[tokio::test]
async fn test_store_load_no_compression_magic() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
    let page = make_page(0, &cfg);
    storage.store_page(&page).await.unwrap();
    let file_path = dir.path().join("journal/level_0/page_0.cjt");
    let bytes = std::fs::read(&file_path).unwrap();
    assert_eq!(&bytes[0..4], b"CJTP");
    let loaded = storage.load_page(0, 0).await.unwrap().unwrap();
    assert_eq!(page, loaded);
}

#[tokio::test]
async fn test_store_load_compression_algos() {
    reset_global_ids();
    for algo in [CompressionAlgorithm::Zstd, CompressionAlgorithm::Lz4, CompressionAlgorithm::Snappy] {
        let dir = tempdir().unwrap();
        let mut cfg = cfg(algo, true);
        if algo == CompressionAlgorithm::Snappy { cfg.compression.level = 0; }
        let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
        let page = make_page(0, &cfg);
        storage.store_page(&page).await.unwrap();
        let loaded = storage.load_page(0, 0).await.unwrap().unwrap();
        assert_eq!(page, loaded);
    }
}

#[tokio::test]
async fn test_corrupt_header() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
    let page = make_page(0, &cfg);
    storage.store_page(&page).await.unwrap();
    let file_path = dir.path().join("journal/level_0/page_0.cjt");
    {
        let mut data = std::fs::read(&file_path).unwrap();
        data[0] = b'X';
        std::fs::write(&file_path, &data).unwrap();
    }
    let err = storage.load_page(0, 0).await.unwrap_err();
    matches!(err, CJError::InvalidFileFormat(_));
}

#[tokio::test]
async fn test_load_page_too_short() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();

    let level_dir = dir.path().join("journal/level_0");
    std::fs::create_dir_all(&level_dir).unwrap();
    std::fs::write(level_dir.join("page_0.cjt"), b"CJTP").unwrap(); // less than 6 bytes

    let err = storage.load_page(0, 0).await.unwrap_err();
    assert!(matches!(err, CJError::InvalidFileFormat(_)));
}

#[tokio::test]
async fn test_page_exists_and_delete() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
    let page = make_page(0, &cfg);
    storage.store_page(&page).await.unwrap();
    assert!(storage.page_exists(0,0).await.unwrap());
    storage.delete_page(0,0).await.unwrap();
    assert!(!storage.page_exists(0,0).await.unwrap());
    storage.delete_page(0,0).await.unwrap();
}

#[tokio::test]
async fn test_list_finalized_pages_summary() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
    for i in 0..3 { let p = make_page(i, &cfg); storage.store_page(&p).await.unwrap(); }
    let mut p1 = make_page(3, &cfg); p1.level = 1; p1.recalculate_merkle_root_and_page_hash(); storage.store_page(&p1).await.unwrap();
    let l0 = storage.list_finalized_pages_summary(0).await.unwrap();
    assert_eq!(l0.len(),3);
    let l1 = storage.list_finalized_pages_summary(1).await.unwrap();
    assert_eq!(l1.len(),1);
    let l2 = storage.list_finalized_pages_summary(2).await.unwrap();
    assert!(l2.is_empty());
}

#[tokio::test]
async fn test_list_finalized_pages_summary_empty_dir() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();

    let summaries = storage.list_finalized_pages_summary(0).await.unwrap();
    assert!(summaries.is_empty());
    assert!(!dir.path().join("journal/level_0").exists());
}

#[tokio::test]
async fn test_load_page_by_hash() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
    let p0 = make_page(0, &cfg); storage.store_page(&p0).await.unwrap();
    let p1 = { let mut p = make_page(1,&cfg); p.level=1; p.recalculate_merkle_root_and_page_hash(); p };
    storage.store_page(&p1).await.unwrap();
    let found = storage.load_page_by_hash(p1.page_hash).await.unwrap().unwrap();
    assert_eq!(found.page_hash, p1.page_hash);
}

#[tokio::test]
async fn test_load_leaf_by_hash() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
    let p = make_page(0, &cfg); let leaf_hash = match &p.content { PageContent::Leaves(v) => v[0].leaf_hash, _=>[0u8;32]};
    storage.store_page(&p).await.unwrap();
    let found = storage.load_leaf_by_hash(&leaf_hash).await.unwrap();
    assert!(found.is_some());
    std::fs::remove_dir_all(dir.path().join("journal/level_0")).unwrap();
    let not_found = storage.load_leaf_by_hash(&leaf_hash).await.unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn test_load_leaf_by_hash_skips_non_page_files() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();

    let page = make_page(0, &cfg);
    let leaf_hash = match &page.content { PageContent::Leaves(v) => v[0].leaf_hash, _ => [0u8;32] };
    // Manually store the page under a non-standard name
    let level_dir = dir.path().join("journal/level_0");
    std::fs::create_dir_all(&level_dir).unwrap();
    let path = level_dir.join("foo_0.cjt");
    let data = serde_json::to_vec(&page).unwrap();
    std::fs::write(&path, &data).unwrap();

    let res = storage.load_leaf_by_hash(&leaf_hash).await.unwrap();
    assert!(res.is_none());
}

#[tokio::test]
async fn test_load_leaf_by_hash_ignores_l1_pages() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();

    // Store an L0 page with a real leaf
    let page0 = make_page(0, &cfg);
    let leaf_hash = match &page0.content { PageContent::Leaves(v) => v[0].leaf_hash, _ => [0u8;32] };
    storage.store_page(&page0).await.unwrap();

    // Store an L1 page containing only a thrall hash
    let mut page1 = JournalPage::new(1, None, Utc::now(), &cfg);
    if let PageContent::ThrallHashes(ref mut v) = page1.content {
        v.push(page0.page_hash);
    }
    page1.recalculate_merkle_root_and_page_hash();
    storage.store_page(&page1).await.unwrap();

    // Leaf in L0 should be found
    let found = storage.load_leaf_by_hash(&leaf_hash).await.unwrap();
    assert!(found.is_some());

    // Thrall hash from L1 should not be treated as a leaf
    let none = storage.load_leaf_by_hash(&page0.page_hash).await.unwrap();
    assert!(none.is_none());
}

#[tokio::test]
async fn test_backup_empty_and_restore() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
    let backup = dir.path().join("backup.zip");
    storage.backup_journal(&backup).await.unwrap();
    assert!(backup.exists());
    let f = std::fs::File::open(&backup).unwrap();
    let mut zip = zip::ZipArchive::new(f).unwrap();
    let mut manifest = String::new();
    zip.by_name("backup_manifest.json").unwrap().read_to_string(&mut manifest).unwrap();
    let json: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert!(json["files"].as_array().unwrap().is_empty());
    let restore_dir = tempdir().unwrap();
    storage.restore_journal(&backup, &restore_dir.path().join("journal")).await.unwrap();
    assert!(restore_dir.path().join("journal").exists());
}

#[tokio::test]
async fn test_backup_non_empty_restore() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
    let p0 = make_page(0,&cfg); storage.store_page(&p0).await.unwrap();
    let p1 = make_page(1,&cfg); storage.store_page(&p1).await.unwrap();
    let backup = dir.path().join("backup.zip");
    storage.backup_journal(&backup).await.unwrap();
    let f = std::fs::File::open(&backup).unwrap();
    let mut zip = zip::ZipArchive::new(f).unwrap();
    let mut manifest = String::new();
    zip.by_name("backup_manifest.json").unwrap().read_to_string(&mut manifest).unwrap();
    let json: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let restore_dir = tempdir().unwrap();
    storage.restore_journal(&backup, &restore_dir.path().join("journal")).await.unwrap();
    assert!(restore_dir.path().join("journal/level_0/page_0.cjt").exists());
    assert!(restore_dir.path().join("journal/level_0/page_1.cjt").exists());
}

#[tokio::test]
async fn test_restore_nonexistent_path_error() {
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();
    let res = storage.restore_journal(Path::new("/no/such/file.zip"), dir.path()).await;
    assert!(matches!(res.unwrap_err(), CJError::StorageError(_)));
}


#[tokio::test]
async fn test_new_permission_denied() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let res = FileStorage::new("/proc/deny_test", cfg.compression.clone()).await;
    assert!(matches!(res, Err(CJError::StorageError(_))));
}

#[tokio::test]
async fn test_load_page_by_hash_skips_bad_files() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let dir = tempdir().unwrap();
    let cfg = cfg(CompressionAlgorithm::None, false);
    let storage = FileStorage::new(dir.path(), cfg.compression.clone()).await.unwrap();

    let level_dir = dir.path().join("journal/level_0");
    std::fs::create_dir_all(&level_dir).unwrap();

    std::fs::write(level_dir.join("page_bad.txt"), b"junk").unwrap();
    std::fs::write(level_dir.join("page_0.cjt"), b"XXXX12").unwrap();

    let res = storage.load_page_by_hash([9u8; 32]).await.unwrap();
    assert!(res.is_none());
}

