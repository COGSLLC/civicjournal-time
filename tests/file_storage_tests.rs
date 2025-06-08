use civicjournal_time::storage::file::FileStorage;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::page::{JournalPage, PageContent};
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::config::Config;
use civicjournal_time::config::SnapshotConfig;
use civicjournal_time::CompressionAlgorithm;
use civicjournal_time::config::CompressionConfig;
use std::path::Path;

use chrono::{DateTime, Utc, Duration};
use serde_json::json;
use std::sync::Arc;
use civicjournal_time::test_utils::{SHARED_TEST_ID_MUTEX, reset_global_ids};
use tempfile::tempdir;

// Helper to create a test config with specified compression
fn create_test_config(compression_algo: CompressionAlgorithm) -> Arc<Config> {
    let compression_config = match compression_algo {
        CompressionAlgorithm::None => CompressionConfig {
            enabled: false,
            algorithm: compression_algo,
            level: 0,
        },
        CompressionAlgorithm::Zstd => CompressionConfig {
            enabled: true,
            algorithm: compression_algo,
            level: 3, // Default Zstd level
        },
        CompressionAlgorithm::Lz4 => CompressionConfig {
            enabled: true,
            algorithm: compression_algo,
            level: 4, // Default Lz4 level
        },
        CompressionAlgorithm::Snappy => CompressionConfig {
            enabled: true,
            algorithm: compression_algo,
            level: 0, // Snappy typically doesn't have levels like zstd/lz4
        },
    };

    Arc::new(Config {
        time_hierarchy: Default::default(),
        storage: Default::default(),
        compression: compression_config,
        logging: Default::default(),
        metrics: Default::default(),
        retention: Default::default(),
        snapshot: SnapshotConfig::default(),
        force_rollup_on_shutdown: false,
    })
}

// Helper to create a test page
fn create_test_page(level: u8, page_id: u64, timestamp: DateTime<Utc>, config: &Config) -> JournalPage {
    let mut page = JournalPage::new(level, None, timestamp, config);
    page.page_id = page_id;
    page
}

// Helper to create a test leaf
fn create_test_leaf(timestamp: DateTime<Utc>) -> JournalLeaf {
    JournalLeaf::new(
        timestamp,
        None,
        "test-container".to_string(),
        json!({ "test": "data" }),
    )
    .unwrap()
}

#[tokio::test]
async fn test_file_storage_lifecycle() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let temp_dir = tempdir().unwrap();
    let config = create_test_config(CompressionAlgorithm::None);
    let storage = FileStorage::new(temp_dir.path().to_path_buf(), config.compression.clone()).await.unwrap();
    
    // Test saving and loading a page
    let timestamp = Utc::now();
    let mut page = create_test_page(0, 1, timestamp, &config);
    let leaf = create_test_leaf(timestamp);
    
    // Add leaf to page
    if let PageContent::Leaves(ref mut leaves) = &mut page.content {
        leaves.push(leaf.clone());
    }
    
    // Save page
    storage.store_page(&page).await.unwrap();
    
    // Load page and verify
    let loaded_page = storage.load_page(0, 1).await.unwrap().unwrap();
    assert_eq!(loaded_page.page_id, page.page_id);
    assert_eq!(loaded_page.level, page.level);
    
    // Test page existence
    assert!(storage.page_exists(0, 1).await.unwrap());
    assert!(!storage.page_exists(0, 2).await.unwrap());
    
    // Cleanup
    temp_dir.close().unwrap();
}

#[tokio::test]
async fn test_compression_formats() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let formats = [
        CompressionAlgorithm::None,
        CompressionAlgorithm::Zstd,
        CompressionAlgorithm::Lz4,
        CompressionAlgorithm::Snappy,
    ];
    
    for &format in &formats {
        let temp_dir = tempdir().unwrap();
        let config = create_test_config(format);
        let storage = FileStorage::new(temp_dir.path().to_path_buf(), config.compression.clone()).await.unwrap();
        
        let timestamp = Utc::now();
        let mut page = create_test_page(0, 1, timestamp, &config);
        let leaf = create_test_leaf(timestamp);
        
        if let PageContent::Leaves(ref mut leaves) = &mut page.content {
            leaves.push(leaf);
        }
        
        // Save and load with current compression
        storage.store_page(&page).await.unwrap();
        let loaded_page = storage.load_page(0, 1).await.unwrap().unwrap();
        assert_eq!(loaded_page.page_id, page.page_id);
        
        temp_dir.close().unwrap();
    }
}

#[tokio::test]
async fn test_backup_and_restore() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    // Create test data
    let source_dir = tempdir().unwrap();
    let backup_dir = tempdir().unwrap();
    let config = create_test_config(CompressionAlgorithm::Zstd);
    
    let storage = FileStorage::new(source_dir.path().to_path_buf(), config.compression.clone()).await.unwrap();
    
    // Create and save some test pages
    let timestamp = Utc::now();
    for i in 0..3 {
        let mut page = create_test_page(0, i, timestamp, &config);
        let leaf = create_test_leaf(timestamp + chrono::Duration::seconds(i as i64));
        
        if let PageContent::Leaves(ref mut leaves) = &mut page.content {
            leaves.push(leaf);
        }
        
        storage.store_page(&page).await.unwrap();
    }
    
    // Create backup
    let backup_path = backup_dir.path().join("backup.zip");
    storage.backup_journal(&backup_path).await.unwrap();
    
    // Verify backup exists
    assert!(backup_path.exists());
    
    // Create a new storage for restore test
    let restore_dir = tempdir().unwrap();
    let restore_config = create_test_config(CompressionAlgorithm::Zstd);
    let restore_storage = FileStorage::new(restore_dir.path().to_path_buf(), restore_config.compression.clone()).await.unwrap();
    
    // Restore from backup
    restore_storage.restore_journal(&backup_path, &restore_dir.path().join("journal")).await.unwrap();
    
    // Verify restored data
    for i in 0..3 {
        assert!(restore_storage.page_exists(0, i).await.unwrap());
    }
    
    // Cleanup
    source_dir.close().unwrap();
    backup_dir.close().unwrap();
    restore_dir.close().unwrap();
}

#[tokio::test]
async fn test_concurrent_access() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let temp_dir = tempdir().unwrap();
    let config = create_test_config(CompressionAlgorithm::None);
    let storage = Arc::new(FileStorage::new(temp_dir.path().to_path_buf(), config.compression.clone()).await.unwrap());
    
    let mut handles = vec![];
    
    // Spawn multiple tasks to save pages concurrently
    for i in 0..5 {
        let storage_clone = storage.clone();
        let config_clone = config.clone(); // Clone config for the spawned task
        let handle = tokio::spawn(async move {
            let timestamp = Utc::now();
            let mut page = create_test_page(0, i, timestamp, &config_clone);
            let leaf = create_test_leaf(timestamp + chrono::Duration::seconds(i as i64));
            
            if let PageContent::Leaves(ref mut leaves) = &mut page.content {
                leaves.push(leaf);
            }
            
            storage_clone.store_page(&page).await.unwrap();
        });
        handles.push(handle);
    }
    
    // Wait for all tasks to complete
    for handle in handles {
        handle.await.unwrap();
    }
    
    // Verify all pages were saved
    for i in 0..5 {
        assert!(storage.page_exists(0, i).await.unwrap());
    }
    
    // Cleanup
    temp_dir.close().unwrap();
}

#[tokio::test]
async fn test_error_handling() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let temp_dir = tempdir().unwrap();
    let config = create_test_config(CompressionAlgorithm::None);
    let storage = FileStorage::new(temp_dir.path().to_path_buf(), config.compression.clone()).await.unwrap();
    
    // Test loading non-existent page
    assert!(storage.load_page(99, 99).await.unwrap().is_none());
    
    // Test backup with invalid path
    let result = storage.backup_journal(Path::new("/invalid/path/backup.zip")).await;
    assert!(result.is_err());
    
    // Test restore with invalid file
    let invalid_backup = temp_dir.path().join("invalid.zip");
    std::fs::write(&invalid_backup, "not a zip file").unwrap();
    let result = storage.restore_journal(&invalid_backup, temp_dir.path().join("invalid_restore_target").as_path()).await;
    assert!(result.is_err());
    
    // Cleanup
    temp_dir.close().unwrap();
}

#[tokio::test]
async fn test_large_page_compression() {
    let _guard = SHARED_TEST_ID_MUTEX.lock().await;
    reset_global_ids();
    let formats = [
        CompressionAlgorithm::None,
        CompressionAlgorithm::Zstd,
        CompressionAlgorithm::Lz4,
        CompressionAlgorithm::Snappy,
    ];

    for &format in &formats {
        let temp_dir = tempdir().unwrap();
        let config = create_test_config(format);
        let storage = FileStorage::new(temp_dir.path().to_path_buf(), config.compression.clone())
            .await
            .unwrap();

        let base = Utc::now();
        let mut page = create_test_page(0, 1, base, &config);
        if let PageContent::Leaves(ref mut leaves) = page.content {
            for i in 0..1000 {
                let leaf = create_test_leaf(base + Duration::milliseconds(i));
                leaves.push(leaf);
            }
        }
        page.recalculate_merkle_root_and_page_hash();
        storage.store_page(&page).await.unwrap();
        let loaded = storage.load_page(0, 1).await.unwrap().unwrap();
        assert_eq!(loaded.content_len(), 1000);
        temp_dir.close().unwrap();
    }
}
