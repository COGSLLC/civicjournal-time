use civicjournal_time::storage::file::FileStorage;
use civicjournal_time::storage::StorageBackend;
use civicjournal_time::core::page::{JournalPage, PageContent};
use civicjournal_time::core::leaf::JournalLeaf;
use civicjournal_time::config::Config;
use civicjournal_time::CompressionAlgorithm;
use civicjournal_time::config::CompressionConfig;
use std::fs::File;
use std::io::Read;
use zip::ZipArchive;
use chrono::{DateTime, Utc};
use serde_json::json;
use tempfile::tempdir;
use std::sync::Arc;

// Helper to create a test config with specified compression
fn create_test_config(compression_algo: CompressionAlgorithm) -> Arc<Config> {
    let compression_config = match compression_algo {
        CompressionAlgorithm::None => CompressionConfig {
            enabled: false,
            algorithm: compression_algo,
            level: 0,
        },
        _ => CompressionConfig {
            enabled: true,
            algorithm: compression_algo,
            level: 3,
        },
    };

    Arc::new(Config {
        time_hierarchy: Default::default(),
        storage: Default::default(),
        compression: compression_config,
        logging: Default::default(),
        metrics: Default::default(),
        retention: Default::default(),
        snapshot: Default::default(), // Added missing field
        force_rollup_on_shutdown: false,
    })
}

// Helper to create a test page with multiple leaves
fn create_test_page_with_leaves(level: u8, page_id: u64, timestamp: DateTime<Utc>, leaf_count: usize, config: &Config) -> JournalPage {
    let mut page = JournalPage::new(level, None, timestamp, config);
    page.page_id = page_id;
    
    if let PageContent::Leaves(ref mut leaves) = &mut page.content {
        for i in 0..leaf_count {
            let leaf = JournalLeaf::new(
                timestamp + chrono::Duration::seconds(i as i64),
                None,
                format!("container-{}", i),
                json!({ "test": format!("data-{}", i) }),
            ).unwrap();
            leaves.push(leaf);
        }
        // Recalculate hashes after adding leaves
        page.recalculate_merkle_root_and_page_hash();
    }
    
    page
}

#[tokio::test]
async fn test_backup_manifest_creation() {
    let temp_dir = tempdir().unwrap();
    let config = create_test_config(CompressionAlgorithm::Zstd);
    let storage = FileStorage::new(temp_dir.path().to_path_buf(), config.compression.clone())
        .await
        .unwrap();

    // Create and save test pages
    let timestamp = Utc::now();
    for i in 0..3 {
        let page = create_test_page_with_leaves(0, i, timestamp, 2, &config);
        storage.store_page(&page).await.unwrap();
    }

    // Create backup
    let backup_path = temp_dir.path().join("backup.zip");
    storage.backup_journal(&backup_path).await.unwrap();

    // Verify backup contains manifest
    let file = File::open(&backup_path).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    
    // Read and parse manifest
    let mut manifest_content = String::new();
    let mut manifest_file = archive.by_name("backup_manifest.json")
        .expect("Manifest file not found in backup");
    manifest_file.read_to_string(&mut manifest_content).unwrap();
    
    let manifest: serde_json::Value = serde_json::from_str(&manifest_content).unwrap();
    
    // Print manifest for debugging
    println!("Manifest content: {}", serde_json::to_string_pretty(&manifest).unwrap());
    
    // Verify manifest structure
    let manifest_version = &manifest["manifest_file_format_version"];
    let storage_type = &manifest["source_storage_details"]["storage_type"];
    let tool_version = &manifest["backup_tool_version"];
    
    assert_eq!(manifest_version, "1.0.0", "Manifest version mismatch");
    assert_eq!(storage_type, "FileStorage", "Storage type mismatch");
    assert_eq!(tool_version, env!("CARGO_PKG_VERSION"), "Tool version mismatch");
    
    // Verify file entries have expected fields
    let files = manifest["files"].as_array().unwrap();
    for file in files {
        assert!(file["relative_path"].is_string(), "Missing relative_path in file entry");
        assert!(file["sha256_hash_uncompressed"].is_string(), "Missing sha256_hash_uncompressed in file entry");
        assert!(file["original_size_bytes_uncompressed"].is_number(), "Missing original_size_bytes_uncompressed in file entry");
        assert!(file["stored_size_bytes_in_backup"].is_number(), "Missing stored_size_bytes_in_backup in file entry");
    }
    
    // Verify file entries
    let files = manifest["files"].as_array().unwrap();
    assert_eq!(files.len(), 3); // Should have 3 pages
    
    // Cleanup
    temp_dir.close().unwrap();
}

#[tokio::test]
async fn test_restore_with_verification() {
    // Setup source storage
    let source_dir = tempdir().unwrap();
    let config = create_test_config(CompressionAlgorithm::Zstd);
    let storage = FileStorage::new(source_dir.path().to_path_buf(), config.compression.clone())
        .await
        .unwrap();

    // Create and save test pages
    let timestamp = Utc::now();
    let mut original_hashes = Vec::new();
    
    for i in 0..3 {
        let mut page = create_test_page_with_leaves(0, i, timestamp, 2, &config);
        page.recalculate_merkle_root_and_page_hash();
        let page_hash = page.page_hash;
        original_hashes.push((i, page_hash));
        storage.store_page(&page).await.unwrap();
    }

    // Create backup
    let backup_path = source_dir.path().join("backup.zip");
    storage.backup_journal(&backup_path).await.unwrap();

    // Create target storage for restore
    let target_dir = tempdir().unwrap();
    let restore_storage = FileStorage::new(target_dir.path().to_path_buf(), config.compression.clone())
        .await
        .unwrap();

    // Restore from backup
    restore_storage.restore_journal(&backup_path, &target_dir.path().join("journal"))
        .await
        .unwrap();

    // Verify restored pages
    for (page_id, original_hash) in original_hashes {
        let restored_page = restore_storage.load_page(0, page_id)
            .await
            .unwrap()
            .expect("Page not found after restore");
        
        assert_eq!(restored_page.page_hash, original_hash, "Page hash mismatch after restore");
    }

    // Cleanup
    source_dir.close().unwrap();
    target_dir.close().unwrap();
}

#[tokio::test]
async fn test_corrupted_backup_handling() {
    let temp_dir = tempdir().unwrap();
    let config = create_test_config(CompressionAlgorithm::None);
    let storage = FileStorage::new(temp_dir.path().to_path_buf(), config.compression.clone())
        .await
        .unwrap();

    // Create a corrupted backup file
    let corrupt_backup = temp_dir.path().join("corrupt.zip");
    std::fs::write(&corrupt_backup, b"not a zip file").unwrap();

    // Try to restore corrupted backup
    let result = storage.restore_journal(&corrupt_backup, &temp_dir.path().join("journal"))
        .await;

    assert!(result.is_err(), "Should fail to restore corrupted backup");

    // Cleanup
    temp_dir.close().unwrap();
}

#[tokio::test]
async fn test_empty_journal_backup() {
    let temp_dir = tempdir().unwrap();
    let config = create_test_config(CompressionAlgorithm::None);
    let storage = FileStorage::new(temp_dir.path().to_path_buf(), config.compression.clone())
        .await
        .unwrap();

    // Create backup of empty journal
    let backup_path = temp_dir.path().join("empty_backup.zip");
    storage.backup_journal(&backup_path).await.unwrap();

    // Verify backup exists
    assert!(backup_path.exists());

    // Verify manifest indicates no files
    let file = File::open(&backup_path).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    
    let mut manifest_content = String::new();
    archive.by_name("backup_manifest.json")
        .unwrap()
        .read_to_string(&mut manifest_content)
        .unwrap();
    
    let manifest: serde_json::Value = serde_json::from_str(&manifest_content).unwrap();
    let files = manifest["files"].as_array().unwrap();
    assert!(files.is_empty(), "Backup of empty journal should have no files");

    // Cleanup
    temp_dir.close().unwrap();
}