// src/storage/file.rs

use std::path::{Path, PathBuf};
use async_trait::async_trait;
use tokio::fs;

use crate::core::page::{JournalPage, JournalPageSummary, PageContent};
use crate::core::leaf::JournalLeaf;
use crate::error::CJError;
use crate::storage::StorageBackend;
use crate::config::CompressionConfig;
use crate::CompressionAlgorithm;
use serde::{Deserialize, Serialize};
use chrono::Utc; // For backup_timestamp_utc

use std::fs::{self as StdFs, File as StdFile}; // StdFs for module, StdFile for File type
use std::io::{self as StdIo, Read, Write}; // StdIo for module, keeping Read/Write
use zip::{ZipArchive, ZipWriter, write::FileOptions}; // CompressionMethod from zip::write is private, rely on qualified zip::CompressionMethod
use walkdir::WalkDir;
use tokio::task; // For spawn_blocking
// Removed duplicate tokio::fs import, one at line 5 is kept.
use sha2::{Sha256, Digest}; // For hashing file contents
use hex; // For encoding hash as string

// Backup Manifest Constants
const MANIFEST_VERSION: &str = "1.0.0";
const BACKUP_MANIFEST_FILENAME: &str = "backup_manifest.json";


#[derive(Serialize, Deserialize, Debug, Clone)]
struct ManifestCompressionInfo {
    algorithm: String, // e.g., "Zstd", "None"
    level: Option<i32>, // Compression level, if applicable
}

// Helper to convert from the main CompressionConfig
impl From<&CompressionConfig> for ManifestCompressionInfo {
    fn from(config: &CompressionConfig) -> Self {
        ManifestCompressionInfo {
            algorithm: format!("{:?}", config.algorithm), // Converts enum like CompressionAlgorithm::Zstd to "Zstd"
            level: Some(config.level as i32),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ManifestStorageDetails {
    storage_type: String, // e.g., "FileStorage"
    base_path: String,
    compression: ManifestCompressionInfo,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ManifestFileEntry {
    relative_path: String,
    // Hash of the original, uncompressed page data (after header, before page-level compression)
    sha256_hash_uncompressed: String,
    // Size of the original, uncompressed page data
    original_size_bytes_uncompressed: u64,
    // Size of the .cjt file as stored in the backup (includes header and page-level compression)
    stored_size_bytes_in_backup: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct BackupManifest {
    // Version of the manifest_file_format itself
    manifest_file_format_version: String,
    backup_timestamp_utc: String, // ISO 8601 format
    source_storage_details: ManifestStorageDetails,
    files: Vec<ManifestFileEntry>,
    backup_tool_version: String, // Version of civicjournal-time that created this
}

// File Header Constants
const MAGIC_STRING: [u8; 4] = *b"CJTP"; // CivicJournal Time Page
const FORMAT_VERSION: u8 = 1;

// CompressionAlgorithm to u8 mapping
// None: 0, Zstd: 1, Lz4: 2, Snappy: 3
fn compression_algorithm_to_u8(algo: CompressionAlgorithm) -> u8 {
    match algo {
        CompressionAlgorithm::None => 0,
        CompressionAlgorithm::Zstd => 1,
        CompressionAlgorithm::Lz4 => 2,
        CompressionAlgorithm::Snappy => 3,
    }
}

fn u8_to_compression_algorithm(val: u8) -> Result<CompressionAlgorithm, CJError> {
    match val {
        0 => Ok(CompressionAlgorithm::None),
        1 => Ok(CompressionAlgorithm::Zstd),
        2 => Ok(CompressionAlgorithm::Lz4),
        3 => Ok(CompressionAlgorithm::Snappy),
        _ => Err(CJError::InvalidFileFormat(format!("Unknown compression algorithm byte: {}", val))),
    }
}

// Note: Ensure zstd, lz4_flex, and snap are in Cargo.toml with appropriate features enabled.
// For zstd
use zstd::stream::{encode_all, decode_all};
// For lz4
use lz4_flex::{compress_prepend_size, decompress_size_prepended};
// For snappy
use snap::raw::{Encoder as SnapEncoder, Decoder as SnapDecoder};

const MARKER_FILE_NAME: &str = ".civicjournal-time";
const JOURNAL_SUBDIR: &str = "journal";

/// A storage backend that persists journal pages to the file system.
///
/// Pages are stored in a structured directory format:
/// `base_path/journal/level_<L>/page_<ID>.json`
#[derive(Debug)]
pub struct FileStorage {
    base_path: PathBuf,
    compression_config: CompressionConfig,
}

impl FileStorage {
    /// Creates a new `FileStorage` instance.
    ///
    /// This will create the base directory and a marker file (`.civicjournal-time`)
    /// if they don't already exist.
    ///
    /// # Arguments
    ///
    /// * `base_path` - The root directory where journal data will be stored.
    pub async fn new<P: AsRef<Path>>(base_path: P, compression_config: CompressionConfig) -> Result<Self, CJError> {
        let path = base_path.as_ref().to_path_buf();

        // Create the base path if it doesn't exist
        fs::create_dir_all(&path).await.map_err(|e| CJError::StorageError(format!("Failed to create base path '{}': {}", path.display(), e)))?;

        // Create the marker file
        let marker_path = path.join(MARKER_FILE_NAME);
        if !fs::try_exists(&marker_path).await.map_err(|e| CJError::StorageError(format!("Failed to check marker file existence '{}': {}", marker_path.display(), e)))? {
            fs::File::create(&marker_path).await.map_err(|e| CJError::StorageError(format!("Failed to create marker file '{}': {}", marker_path.display(), e)))?;
        }

        Ok(Self { base_path: path, compression_config })
    }

    /// Constructs the path for a specific journal page file.
    /// Path: `base_path/journal/level_<L>/page_<ID>.json`
    /// Helper to create a boxed version of `FileStorage`.
    /// This is useful when a `Box<dyn StorageBackend>` is needed.
    pub fn boxed(self) -> Box<dyn StorageBackend> {
        Box::new(self)
    }

    /// Constructs the path for a specific journal page file.
    /// Path: `base_path/journal/level_<L>/page_<ID>.cjt[.compression_alg]`
    fn get_page_path(&self, level: u8, page_id: u64) -> PathBuf {
        let filename = format!("page_{:08}.cjt", page_id);
        self.base_path
            .join(JOURNAL_SUBDIR)
            .join(format!("level_{}", level))
            .join(filename)
    }
}

#[async_trait]
impl StorageBackend for FileStorage {
    async fn store_page(&self, page: &JournalPage) -> Result<(), CJError> {
        let page_path = self.get_page_path(page.level, page.page_id);

        if let Some(parent_dir) = page_path.parent() {
            fs::create_dir_all(parent_dir).await.map_err(|e| 
                CJError::StorageError(format!("Failed to create parent directory for page '{}': {}", page_path.display(), e))
            )?;
        }

        let serialized_page = serde_json::to_vec(page).map_err(CJError::from)?;

        let (compressed_page_data, algo_used) = if self.compression_config.enabled {
            let algo = self.compression_config.algorithm;
            let data = match algo {
                CompressionAlgorithm::Zstd => {
                    encode_all(&serialized_page[..], self.compression_config.level as i32)
                        .map_err(|e| CJError::CompressionError(format!("Zstd compression failed: {}", e)))?
                }
                CompressionAlgorithm::Lz4 => {
                    compress_prepend_size(&serialized_page)
                }
                CompressionAlgorithm::Snappy => {
                    SnapEncoder::new().compress_vec(&serialized_page)
                        .map_err(|e| CJError::CompressionError(format!("Snappy compression failed: {}", e)))?
                }
                CompressionAlgorithm::None => serialized_page,
            };
            (data, algo)
        } else {
            (serialized_page, CompressionAlgorithm::None)
        };

        let mut file_content = Vec::with_capacity(6 + compressed_page_data.len());
        file_content.extend_from_slice(&MAGIC_STRING);
        file_content.push(FORMAT_VERSION);
        file_content.push(compression_algorithm_to_u8(algo_used));
        file_content.extend_from_slice(&compressed_page_data);

        fs::write(&page_path, file_content).await.map_err(|e| 
            CJError::StorageError(format!("Failed to write page '{}': {}", page_path.display(), e))
        )?;

        Ok(())
    }

    async fn load_page(&self, level: u8, page_id: u64) -> Result<Option<JournalPage>, CJError> {
        let page_path = self.get_page_path(level, page_id);

        if !fs::try_exists(&page_path).await.map_err(|e| 
            CJError::StorageError(format!("Failed to check page existence '{}': {}", page_path.display(), e))
        )? {
            return Ok(None);
        }

        let file_data = fs::read(&page_path).await.map_err(|e| 
            CJError::StorageError(format!("Failed to read page '{}': {}", page_path.display(), e))
        )?;

        if file_data.len() < 6 {
            return Err(CJError::InvalidFileFormat(format!("File {} is too short to contain a valid header", page_path.display())));
        }

        let magic = &file_data[0..4];
        if magic != MAGIC_STRING {
            return Err(CJError::InvalidFileFormat(format!("Invalid magic string in file {}. Expected {:?}, got {:?}", page_path.display(), MAGIC_STRING, magic)));
        }

        let version = file_data[4];
        if version != FORMAT_VERSION {
            // For now, strict version match. Could allow older versions later.
            return Err(CJError::InvalidFileFormat(format!("Unsupported format version {} in file {}. Expected {}", version, page_path.display(), FORMAT_VERSION)));
        }

        let algo_byte = file_data[5];
        let header_algo = u8_to_compression_algorithm(algo_byte)?;
        
        let actual_compressed_data = &file_data[6..];

        let decompressed_data = match header_algo {
            CompressionAlgorithm::Zstd => {
                decode_all(actual_compressed_data)
                    .map_err(|e| CJError::DecompressionError(format!("Zstd decompression failed for {}: {}", page_path.display(), e)))?
            }
            CompressionAlgorithm::Lz4 => {
                decompress_size_prepended(actual_compressed_data)
                    .map_err(|e| CJError::DecompressionError(format!("Lz4 decompression failed for {}: {}", page_path.display(), e)))?
            }
            CompressionAlgorithm::Snappy => {
                SnapDecoder::new().decompress_vec(actual_compressed_data)
                    .map_err(|e| CJError::DecompressionError(format!("Snappy decompression failed for {}: {}", page_path.display(), e)))?
            }
            CompressionAlgorithm::None => actual_compressed_data.to_vec(), // Make it a Vec<u8> like other arms
        };

        let page: JournalPage = serde_json::from_slice(&decompressed_data).map_err(CJError::SerdeJson // Ensure correct error mapping
        )?;
        Ok(Some(page))
    }

    async fn page_exists(&self, level: u8, page_id: u64) -> Result<bool, CJError> {
        let page_path = self.get_page_path(level, page_id);
        fs::try_exists(&page_path).await.map_err(|e| CJError::StorageError(format!("Failed to check page existence '{}': {}", page_path.display(), e)))
    }

    async fn delete_page(&self, level: u8, page_id: u64) -> Result<(), CJError> {
        let path = self.get_page_path(level, page_id);
        if fs::try_exists(&path).await.map_err(|e| CJError::StorageError(format!("Failed to check page existence for deletion '{}': {}", path.display(), e)))? {
            tokio::fs::remove_file(path).await.map_err(|e| {
                CJError::StorageError(format!("Failed to delete page file L{}/{}: {}", level, page_id, e))
            })?
        }
        Ok(())
    }

    async fn list_finalized_pages_summary(&self, level: u8) -> Result<Vec<JournalPageSummary>, CJError> {
        let level_path = self.base_path.join(JOURNAL_SUBDIR).join(format!("level_{}", level));
        if !fs::try_exists(&level_path).await.map_err(|e| CJError::StorageError(format!("Failed to check level directory existence '{}': {}", level_path.display(), e)))? {
            return Ok(Vec::new()); // No directory for this level, so no pages
        }

        let mut summaries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&level_path).await.map_err(|e| { // Added & to level_path
            CJError::StorageError(format!("Failed to read level directory L{}: {}", level, e))
        })?;

        while let Some(entry) = read_dir.next_entry().await.map_err(|e| {
            CJError::StorageError(format!("Failed to read directory entry in L{}: {}", level, e))
        })? {
            let path = entry.path();
            // Check if it's a file and has the .cjt extension
            if path.is_file() && path.extension().is_some_and(|ext| ext == "cjt") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if stem.starts_with("page_") {
                        if let Ok(page_id) = stem.trim_start_matches("page_").parse::<u64>() {
                            // Attempt to load the full page to get its summary details
                            match self.load_page(level, page_id).await {
                                Ok(Some(page)) => {
                                    summaries.push(JournalPageSummary {
                                        page_id: page.page_id,
                                        level: page.level,
                                        creation_timestamp: page.creation_timestamp,
                                        end_time: page.end_time,
                                        page_hash: page.page_hash,
                                    });
                                }
                                Ok(None) => {
                                    log::warn!("Found page file {} but it could not be loaded as a JournalPage for summary.", path.display());
                                }
                                Err(e) => {
                                    log::warn!("Error loading page {} for summary: {}", path.display(), e);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(summaries)
    }

    async fn load_page_by_hash(&self, target_page_hash: [u8; 32]) -> Result<Option<JournalPage>, CJError> {
        // TODO: This is inefficient. Consider adding an index (page_hash -> file_path)
        // or limiting search if possible (e.g., if child level is known).
        // For now, iterates through all levels and pages.

        let journal_dir = self.base_path.join(JOURNAL_SUBDIR);
        if !journal_dir.exists() {
            return Ok(None);
        }

        let mut dir_entries = match tokio::fs::read_dir(&journal_dir).await {
            Ok(de) => de,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(CJError::StorageError(format!("Failed to read journal directory {}: {}", journal_dir.display(), e))),
        };

        while let Some(entry) = dir_entries.next_entry().await
            .map_err(|e| CJError::StorageError(format!("Error reading entry in {}: {}", journal_dir.display(), e)))? {
            
            let path = entry.path();
            if path.is_dir() {
                let dir_name = path.file_name().unwrap_or_default().to_string_lossy();
                if dir_name.starts_with("level_") {
                    let mut page_files = match tokio::fs::read_dir(&path).await {
                        Ok(pf) => pf,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue, // Level dir might be empty or gone
                        Err(e) => return Err(CJError::StorageError(format!("Failed to read level directory {}: {}", path.display(), e))),
                    };
                    
                    while let Some(page_entry) = page_files.next_entry().await
                        .map_err(|e| CJError::StorageError(format!("Error reading entry in {}: {}", path.display(), e)))? {
                        
                        let page_path = page_entry.path();
                        // Check for .cjt and known compressed variants
                        let extension = page_path.extension().and_then(std::ffi::OsStr::to_str);
                        let is_page_file = match extension {
                            Some("cjt") | Some("cjtgz") | Some("cjtzst") | Some("cjtLz4") | Some("cjtsnappy") => true,
                            _ => false,
                        };

                        if page_path.is_file() && is_page_file {
                            let file_content = match tokio::fs::read(&page_path).await {
                                Ok(fc) => fc,
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue, // File might have been deleted concurrently
                                Err(e) => return Err(CJError::StorageError(format!("Failed to read page file {}: {}", page_path.display(), e))),
                            };

                            if file_content.len() < 6 { // MAGIC (4) + VERSION (1) + COMPRESSION_ALGO (1)
                                log::warn!("[FileStorage::load_page_by_hash] Skipping malformed file (too short): {}", page_path.display());
                                continue;
                            }
                            if &file_content[0..4] != MAGIC_STRING {
                                log::warn!("[FileStorage::load_page_by_hash] Skipping file with incorrect magic string: {}", page_path.display());
                                continue;
                            }
                            // let _format_version = file_content[4]; // Could check this
                            let compression_algo_byte = file_content[5];
                            let compression_algo = match u8_to_compression_algorithm(compression_algo_byte) {
                                Ok(ca) => ca,
                                Err(_) => {
                                    log::warn!("[FileStorage::load_page_by_hash] Skipping file with unknown compression algorithm byte {}: {}", compression_algo_byte, page_path.display());
                                    continue;
                                }
                            };
                            
                            let actual_data = &file_content[6..];
                            let decompressed_data = match compression_algo {
                                CompressionAlgorithm::None => actual_data.to_vec(),
                                CompressionAlgorithm::Zstd => decode_all(actual_data)
                                    .map_err(|e| CJError::CompressionError(format!("Zstd decompression failed for {}: {}", page_path.display(), e)))?,
                                CompressionAlgorithm::Lz4 => decompress_size_prepended(actual_data)
                                    .map_err(|e| CJError::CompressionError(format!("Lz4 decompression failed for {}: {}", page_path.display(), e)))?,
                                CompressionAlgorithm::Snappy => {
                                    let mut decoder = SnapDecoder::new();
                                    decoder.decompress_vec(actual_data)
                                        .map_err(|e| CJError::CompressionError(format!("Snappy decompression failed for {}: {}", page_path.display(), e)))?
                                }
                            };

                            let page: JournalPage = match serde_json::from_slice(&decompressed_data) {
                                Ok(p) => p,
                                Err(e) => {
                                    log::error!("[FileStorage::load_page_by_hash] Failed to deserialize page from {}: {}. Skipping.", page_path.display(), e);
                                    continue;
                                }
                            };

                            if page.page_hash == target_page_hash {
                                return Ok(Some(page));
                            }
                        }
                    }
                }
            }
        }
        Ok(None) // Page with the given hash not found
    }

    async fn load_leaf_by_hash(&self, target_leaf_hash: &[u8; 32]) -> Result<Option<JournalLeaf>, CJError> {
        let level0_dir_path = self.base_path.join(JOURNAL_SUBDIR).join("level_0");

        if !level0_dir_path.exists() {
            return Ok(None); // No L0 directory, so no L0 pages to search
        }

        let mut entries = match fs::read_dir(&level0_dir_path).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // If the directory was checked to exist but then disappeared,
                // or if there's a race condition, treat as not found.
                return Ok(None);
            }
            Err(e) => {
                return Err(CJError::StorageError(format!(
                    "Failed to read level 0 directory '{}': {}",
                    level0_dir_path.display(), e
                )));
            }
        };

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            CJError::StorageError(format!(
                "Failed to read directory entry in level 0 path '{}': {}",
                level0_dir_path.display(), e
            ))
        })? {
            let path = entry.path();
            if path.is_file() {
                if let Some(file_name_os_str) = path.file_name() {
                    if let Some(file_name_str) = file_name_os_str.to_str() {
                        if file_name_str.starts_with("page_") {
                            if let Some(end_of_id_idx) = file_name_str.find('.') {
                                let id_part = &file_name_str[5..end_of_id_idx];
                                if let Ok(page_id) = id_part.parse::<u64>() {
                                    match self.load_page(0, page_id).await {
                                        Ok(Some(page)) => {
                                            if page.level == 0 { // Sanity check
                                                if let PageContent::Leaves(leaves) = page.content {
                                                    for leaf_in_page in leaves {
                                                        if leaf_in_page.leaf_hash == *target_leaf_hash {
                                                            return Ok(Some(leaf_in_page.clone()));
                                                        }
                                                    }
                                                }
                                            } else {
                                                log::warn!("Loaded page {} from level_0 directory but its level is {}. Skipping.", page_id, page.level);
                                            }
                                        }
                                        Ok(None) => {
                                            log::warn!(
                                                "File '{}' (parsed as page_id {}) was not found by load_page or is not a valid page. Skipping.",
                                                path.display(), page_id
                                            );
                                        }
                                        Err(e) => {
                                            log::error!(
                                                "Failed to load or parse page file '{}' (parsed as page_id {}): {}. Skipping.",
                                                path.display(), page_id, e
                                            );
                                        }
                                    }
                                } else {
                                    log::warn!("Could not parse page_id from part '{}' in filename '{}'. Skipping.", id_part, path.display());
                                }
                            } else {
                                log::warn!("Filename '{}' starts with 'page_' but does not contain '.' to delimit page_id. Skipping.", path.display());
                            }
                        }
                        // else: filename does not start with "page_", so ignore.
                    }
                    // else: filename not valid UTF-8, ignore.
                }
                // else: no filename, ignore.
            }
            // else: not a file, ignore.
        }
        Ok(None) // Leaf not found
    }

    async fn backup_journal(&self, backup_path: &Path) -> Result<(), CJError> {
    let source_dir_to_zip = self.base_path.join(JOURNAL_SUBDIR);

    // If the source journal directory doesn't exist, create an empty zip file.
    if !source_dir_to_zip.exists() {
        log::info!("Source journal directory '{}' does not exist. Creating empty backup zip at '{}'.", source_dir_to_zip.display(), backup_path.display());
        let backup_file_handle = StdFile::create(backup_path).map_err(|e| 
            CJError::StorageError(format!("Failed to create empty backup file '{}': {}", backup_path.display(), e))
        )?;
        let mut zip_writer = ZipWriter::new(backup_file_handle);
        // Add an empty manifest to an empty backup for consistency
        let empty_manifest = BackupManifest {
            manifest_file_format_version: MANIFEST_VERSION.to_string(),
            backup_timestamp_utc: Utc::now().to_rfc3339(),
            source_storage_details: ManifestStorageDetails {
                storage_type: "FileStorage".to_string(),
                base_path: self.base_path.to_string_lossy().into_owned(),
                compression: ManifestCompressionInfo::from(&self.compression_config),
            },
            files: Vec::new(),
            backup_tool_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        let manifest_json = serde_json::to_string_pretty(&empty_manifest).map_err(|e| 
            CJError::StorageError(format!("Failed to serialize empty backup manifest: {}", e))
        )?;
        let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip_writer.start_file(BACKUP_MANIFEST_FILENAME, options).map_err(|e| 
            CJError::StorageError(format!("Zip start_file failed for empty manifest: {}", e))
        )?;
        zip_writer.write_all(manifest_json.as_bytes()).map_err(|e| 
            CJError::StorageError(format!("Zip write_all failed for empty manifest: {}", e))
        )?;
        zip_writer.finish().map_err(|e| 
            CJError::StorageError(format!("Failed to finalize empty zip archive '{}': {}", backup_path.display(), e))
        )?;
        return Ok(());
    }

    if let Some(parent_dir) = backup_path.parent() {
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir).await.map_err(|e| {
                CJError::StorageError(format!(
                    "Failed to create parent directory for backup file '{}': {}",
                    backup_path.display(),
                    e
                ))
            })?;
        }
    }

    let backup_path_owned = backup_path.to_path_buf();
    let source_dir_owned = source_dir_to_zip.clone();
    let base_path_clone = self.base_path.clone(); // Clone for closure
    let compression_config_clone = self.compression_config.clone(); // Clone for closure

    task::spawn_blocking(move || -> Result<(), CJError> {
        let temp_backup_path = backup_path_owned.with_extension(
            format!("{}.tmp", backup_path_owned.extension().unwrap_or_else(|| std::ffi::OsStr::new("zip")).to_str().unwrap_or("zip"))
        );
        
        if temp_backup_path.exists() {
            if let Err(e) = StdFs::remove_file(&temp_backup_path) {
                log::warn!("Failed to remove existing temp file '{}': {}. Continuing...", temp_backup_path.display(), e);
            }
        }

        let file = StdFile::create(&temp_backup_path).map_err(|e| {
            CJError::StorageError(format!("Failed to create temporary backup file '{}': {}",temp_backup_path.display(),e))
        })?;
        
        let mut manifest_files: Vec<ManifestFileEntry> = Vec::new();
        let backup_timestamp_utc = Utc::now().to_rfc3339();
        let source_storage_details = ManifestStorageDetails {
            storage_type: "FileStorage".to_string(),
            base_path: base_path_clone.to_string_lossy().into_owned(),
            compression: ManifestCompressionInfo::from(&compression_config_clone),
        };
        let backup_tool_version = env!("CARGO_PKG_VERSION").to_string();

        {
            let mut zip_writer = ZipWriter::new(file);
            let options = FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o755);
            let mut file_buffer = Vec::new(); // Buffer for reading file contents

            for entry in WalkDir::new(&source_dir_owned).into_iter().filter_map(|e| e.ok()) {
                let path = entry.path();
                let name_in_zip = path.strip_prefix(&source_dir_owned).map_err(|e| 
                    CJError::StorageError(format!("Failed to strip prefix for path '{}': {}", path.display(), e))
                )?;

                if path == temp_backup_path { continue; } // Don't zip the temp file itself

                let name_in_zip_str = name_in_zip.to_str().ok_or_else(|| {
                    CJError::StorageError(format!("Path '{}' is not valid UTF-8.", name_in_zip.display()))
                })?;

                if path.is_file() {
                    let mut f = StdFile::open(path).map_err(|e| CJError::StorageError(format!("Failed to open source file {}: {}", path.display(), e)))?;
                    file_buffer.clear();
                    f.read_to_end(&mut file_buffer).map_err(|e| CJError::StorageError(format!("Failed to read source file {}: {}", path.display(), e)))?;
                    let stored_size_bytes_in_backup = file_buffer.len() as u64;

                    // --- Start Manifest File Entry Logic ---
                    if file_buffer.len() < 6 { // Basic check for header size
                        log::warn!("File {} is too small to be a valid CJT page, skipping for manifest.", path.display());
                        // Still add to zip, just not to manifest detailed entry
                        zip_writer.start_file(name_in_zip_str, options).map_err(|e| CJError::StorageError(format!("Zip start_file failed for {}: {}", name_in_zip_str, e)))?;
                        zip_writer.write_all(&file_buffer).map_err(|e| CJError::StorageError(format!("Zip write_all failed for {}: {}", name_in_zip_str, e)))?;
                        continue;
                    }

                    if &file_buffer[0..4] != MAGIC_STRING {
                        log::warn!("File {} does not have magic string, skipping for manifest.", path.display());
                        zip_writer.start_file(name_in_zip_str, options).map_err(|e| CJError::StorageError(format!("Zip start_file failed for {}: {}", name_in_zip_str, e)))?;
                        zip_writer.write_all(&file_buffer).map_err(|e| CJError::StorageError(format!("Zip write_all failed for {}: {}", name_in_zip_str, e)))?;
                        continue;
                    }
                    // We don't strictly check FORMAT_VERSION for manifest generation, load_page does that.
                    
                    let page_compression_byte = file_buffer[5];
                    let page_compression_algo = u8_to_compression_algorithm(page_compression_byte)?;
                    
                    let page_data_with_header_stripped = &file_buffer[6..];
                    let uncompressed_page_data = match page_compression_algo {
                        CompressionAlgorithm::None => page_data_with_header_stripped.to_vec(),
                        CompressionAlgorithm::Zstd => decode_all(page_data_with_header_stripped).map_err(|e| CJError::CompressionError(format!("Zstd decompression failed for manifest entry {}: {}", path.display(), e)))?,
                        CompressionAlgorithm::Lz4 => decompress_size_prepended(page_data_with_header_stripped).map_err(|e| CJError::CompressionError(format!("Lz4 decompression failed for manifest entry {}: {}", path.display(), e)))?,
                        CompressionAlgorithm::Snappy => SnapDecoder::new().decompress_vec(page_data_with_header_stripped).map_err(|e| CJError::CompressionError(format!("Snappy decompression failed for manifest entry {}: {}", path.display(), e)))?,
                    };

                    let original_size_bytes_uncompressed = uncompressed_page_data.len() as u64;
                    let mut hasher = Sha256::new();
                    hasher.update(&uncompressed_page_data);
                    let sha256_hash_uncompressed = hex::encode(hasher.finalize());

                    manifest_files.push(ManifestFileEntry {
                        relative_path: name_in_zip_str.to_string().replace("\\", "/"), // Ensure cross-platform path separators
                        sha256_hash_uncompressed,
                        original_size_bytes_uncompressed,
                        stored_size_bytes_in_backup,
                    });
                    // --- End Manifest File Entry Logic ---

                    zip_writer.start_file(name_in_zip_str, options).map_err(|e| CJError::StorageError(format!("Zip start_file failed for {}: {}", name_in_zip_str, e)))?;
                    zip_writer.write_all(&file_buffer).map_err(|e| CJError::StorageError(format!("Zip write_all failed for {}: {}", name_in_zip_str, e)))?;
                
                } else if !name_in_zip.as_os_str().is_empty() { // path.is_dir() implicitly handled by WalkDir, add if not root
                    zip_writer.add_directory(name_in_zip_str, options).map_err(|e| CJError::StorageError(format!("Zip add_directory failed for {}: {}", name_in_zip_str, e)))?;
                }
            }

            // Add manifest to zip
            let manifest = BackupManifest {
                manifest_file_format_version: MANIFEST_VERSION.to_string(),
                backup_timestamp_utc,
                source_storage_details,
                files: manifest_files,
                backup_tool_version,
            };
            let manifest_json = serde_json::to_string_pretty(&manifest).map_err(|e| 
                CJError::StorageError(format!("Failed to serialize backup manifest: {}", e))
            )?;
            zip_writer.start_file(BACKUP_MANIFEST_FILENAME, options.clone()).map_err(|e| 
                CJError::StorageError(format!("Zip start_file failed for manifest: {}", e))
            )?;
            zip_writer.write_all(manifest_json.as_bytes()).map_err(|e| 
                CJError::StorageError(format!("Zip write_all failed for manifest: {}", e))
            )?;

            zip_writer.finish().map_err(|e| {
                CJError::StorageError(format!("Failed to finalize zip archive for temp file '{}': {}",temp_backup_path.display(),e))
            })?;
        } // zip_writer (and file) dropped here

        if backup_path_owned.exists() {
             if backup_path_owned.is_dir() {
                return Err(CJError::StorageError(format!("Target backup path '{}' exists and is a directory.", backup_path_owned.display())));
             }
            StdFs::remove_file(&backup_path_owned).map_err(|e| {
                CJError::StorageError(format!("Failed to remove existing backup file before rename '{}': {}", backup_path_owned.display(), e))
            })?;
        }

        StdFs::rename(&temp_backup_path, &backup_path_owned).map_err(|e| {
            let _ = StdFs::remove_file(&temp_backup_path); // Attempt to clean up temp file
            CJError::StorageError(format!("Failed to rename temp backup file '{}' to '{}': {}", temp_backup_path.display(), backup_path_owned.display(), e))
        })?;
        
        Ok(())
    }).await??;

    Ok(())
    }

    async fn restore_journal(&self, backup_path: &Path, target_journal_dir: &Path) -> Result<(), CJError> {
        if !tokio::fs::try_exists(backup_path).await.map_err(|e| CJError::StorageError(format!("Failed to check existence of backup path '{}': {}", backup_path.display(), e)))? {
            return Err(CJError::StorageError(format!("Backup path '{}' does not exist.", backup_path.display())));
        }

        let backup_path_buf = backup_path.to_path_buf();
        let target_journal_dir_buf = target_journal_dir.to_path_buf();

        tokio::task::spawn_blocking(move || -> Result<(), CJError> {
            log::info!("Starting restore from '{}' to '{}'", backup_path_buf.display(), target_journal_dir_buf.display());

            if target_journal_dir_buf.exists() {
                log::info!("Target directory '{}' exists, clearing it before restore.", target_journal_dir_buf.display());
                StdFs::remove_dir_all(&target_journal_dir_buf).map_err(|e| CJError::StorageError(format!("Failed to remove target directory '{}': {}", target_journal_dir_buf.display(), e)))?;
            }
            StdFs::create_dir_all(&target_journal_dir_buf).map_err(|e| CJError::StorageError(format!("Failed to create target directory '{}': {}", target_journal_dir_buf.display(), e)))?;

            let effective_extraction_root = target_journal_dir_buf.clone(); // Path already points to the 'journal' dir where levels should be created
            // StdFs::create_dir_all(&effective_extraction_root) is already done for target_journal_dir_buf above.

            let backup_file = StdFile::open(&backup_path_buf).map_err(|e| CJError::StorageError(format!("Failed to open backup file '{}': {}", backup_path_buf.display(), e)))?;
            let mut archive = ZipArchive::new(backup_file).map_err(|e| CJError::StorageError(format!("Failed to read zip archive from '{}': {}", backup_path_buf.display(), e)))?;

            for i in 0..archive.len() {
                let mut file_in_zip = archive.by_index(i).map_err(|e| CJError::StorageError(format!("Failed to get file at index {} from zip: {}", i, e)))?;
                
                let Some(enclosed_name) = file_in_zip.enclosed_name() else {
                    log::warn!("Skipping potentially unsafe file path in zip: '{}'", file_in_zip.name());
                    continue;
                };
                let outpath = effective_extraction_root.join(enclosed_name);

                if file_in_zip.name().ends_with('/') { 
                    StdFs::create_dir_all(&outpath).map_err(|e| CJError::StorageError(format!("Failed to create directory '{}': {}", outpath.display(), e)))?;
                } else {
                    if let Some(p) = outpath.parent() {
                        if !p.exists() {
                            StdFs::create_dir_all(p).map_err(|e| CJError::StorageError(format!("Failed to create parent directory '{}': {}", p.display(), e)))?;
                        }
                    }
                    let mut outfile = StdFile::create(&outpath).map_err(|e| CJError::StorageError(format!("Failed to create output file '{}': {}", outpath.display(), e)))?;
                    StdIo::copy(&mut file_in_zip, &mut outfile).map_err(|e| CJError::StorageError(format!("Failed to copy content to '{}': {}", outpath.display(), e)))?;
                }

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Some(mode) = file_in_zip.unix_mode() {
                        StdFs::set_permissions(&outpath, StdFs::Permissions::from_mode(mode)).map_err(|e| 
                            CJError::StorageError(format!("Failed to set permissions on '{}': {}", outpath.display(), e))
                        )?;
                    }
                }
            }
            log::info!("Successfully restored journal from '{}' to '{}'", backup_path_buf.display(), target_journal_dir_buf.display());
            Ok(())
        }).await.map_err(|e| CJError::StorageError(format!("Restore task panicked: {}", e)))??;

        Ok(())
    }

}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    // use std::time::SystemTime; // Marked as unused
    use chrono::Utc;
    use tempfile::tempdir; // Added missing import
    use super::FileStorage; // Added missing import

    use crate::config::{Config, CompressionConfig};
    
    use crate::core::leaf::JournalLeaf;
    use crate::core::page::JournalPage;
    use crate::types::RollupContentType;
    use crate::storage::StorageBackend;



    // Helper function to create a FileStorage instance in a temporary directory
    async fn setup_test_filestorage() -> (FileStorage, tempfile::TempDir) {
        let dir = tempdir().expect("Failed to create temp dir for tests");
        let storage = FileStorage::new(dir.path(), CompressionConfig::default()).await.expect("Failed to create FileStorage in setup");
        (storage, dir)
    }

    // Helper to create a simple leaf
    fn create_test_leaf(leaf_data_v1: crate::core::leaf::LeafDataV1, container_id_suffix: &str, prev_leaf_hash: Option<[u8; 32]>) -> JournalLeaf {
        let payload = serde_json::to_value(crate::core::leaf::LeafData::V1(leaf_data_v1.clone()))
            .expect("Failed to serialize LeafDataV1 for test leaf");
        JournalLeaf::new(
            leaf_data_v1.timestamp,
            prev_leaf_hash,
            format!("test_container_{}", container_id_suffix),
            payload
        ).expect("Failed to create JournalLeaf in helper")
    }

    #[tokio::test]
    async fn test_load_leaf_by_hash_file_storage() {
        let (storage, _temp_dir) = setup_test_filestorage().await; // _temp_dir ensures directory is cleaned up
        let page_config = Arc::new(Config::default()); // Use default config for tests

        // 1. Create some leaves
        let leaf_data_1 = crate::core::leaf::LeafDataV1 {
            timestamp: Utc::now() - chrono::Duration::seconds(30),
            content_type: "text/plain".to_string(),
            content: b"Leaf 1 content".to_vec(),
            author: "test".to_string(),
            signature: "sig1".to_string(),
        };
        let leaf1 = create_test_leaf(leaf_data_1.clone(), "p0_1_l1", None);

        let leaf_data_2 = crate::core::leaf::LeafDataV1 {
            timestamp: Utc::now() - chrono::Duration::seconds(20),
            content_type: "text/plain".to_string(),
            content: b"Leaf 2 content - find me".to_vec(),
            author: "test".to_string(),
            signature: "sig2".to_string(),
        };
        let leaf2 = create_test_leaf(leaf_data_2.clone(), "p0_2_l2", Some(leaf1.leaf_hash));

        let leaf_data_3 = crate::core::leaf::LeafDataV1 {
            timestamp: Utc::now() - chrono::Duration::seconds(10),
            content_type: "text/plain".to_string(),
            content: b"Leaf 3 content - also on L0".to_vec(),
            author: "test".to_string(),
            signature: "sig3".to_string(),
        };
        let leaf3 = create_test_leaf(leaf_data_3.clone(), "p0_2_l3", Some(leaf2.leaf_hash));

        let leaf_data_4 = crate::core::leaf::LeafDataV1 { // For L1 page
            timestamp: Utc::now(),
            content_type: "text/plain".to_string(),
            content: b"Leaf 4 content - on L1 page".to_vec(),
            author: "test".to_string(),
            signature: "sig4".to_string(),
        };
        let leaf4 = create_test_leaf(leaf_data_4.clone(), "p1_1_l4", None);

        // 2. Create and store L0 pages
        let mut page0_1 = JournalPage::new(0, None, leaf_data_1.timestamp, &page_config);
        page0_1.page_id = 1; // Manually set for test predictability
        page0_1.add_leaf(leaf1.clone());
        page0_1.recalculate_merkle_root_and_page_hash();
        storage.store_page(&page0_1).await.expect("Failed to store page0_1");

        let mut page0_2 = JournalPage::new(0, Some(page0_1.page_hash), leaf_data_2.timestamp, &page_config);
        page0_2.page_id = 2; // Manually set for test predictability
        page0_2.add_leaf(leaf2.clone());
        page0_2.add_leaf(leaf3.clone());
        page0_2.recalculate_merkle_root_and_page_hash();
        storage.store_page(&page0_2).await.expect("Failed to store page0_2");

        // 3. Create and store an L1 page (should be ignored by load_leaf_by_hash)
        let mut page1_1 = JournalPage::new(1, None, leaf_data_4.timestamp, &page_config);
        page1_1.page_id = 3; // Manually set for test predictability
        // Populate the page based on the configured content type. load_leaf_by_hash
        // only scans L0 pages, so any content here should be ignored.
        match page_config.time_hierarchy.levels[1].rollup_config.content_type {
            RollupContentType::ChildHashes => {
                page1_1.add_thrall_hash(page0_1.page_hash, page0_1.end_time);
            }
            RollupContentType::NetPatches => {
                let mut patch = std::collections::HashMap::new();
                patch.insert(
                    "dummy_field".to_string(),
                    serde_json::Value::String("dummy".to_string()),
                );
                let mut patch_map = std::collections::HashMap::new();
                patch_map.insert("obj1".to_string(), patch);
                page1_1.merge_net_patches(patch_map, leaf_data_4.timestamp);
            }
            RollupContentType::ChildHashesAndNetPatches => {
                page1_1.add_thrall_hash(page0_1.page_hash, page0_1.end_time);
                let mut patch = std::collections::HashMap::new();
                patch.insert(
                    "dummy_field".to_string(),
                    serde_json::Value::String("dummy".to_string()),
                );
                let mut patch_map = std::collections::HashMap::new();
                patch_map.insert("obj1".to_string(), patch);
                page1_1.merge_net_patches(patch_map, leaf_data_4.timestamp);
            }
        }
        page1_1.recalculate_merkle_root_and_page_hash();
        storage.store_page(&page1_1).await.expect("Failed to store page1_1");

        // 4. Test retrieving existing leaves
        let found_leaf2 = storage.load_leaf_by_hash(&leaf2.leaf_hash).await.expect("Error loading leaf2");
        assert!(found_leaf2.is_some(), "Leaf 2 was not found");
        assert_eq!(found_leaf2.unwrap().leaf_hash, leaf2.leaf_hash, "Found leaf hash does not match leaf2");

        let found_leaf3 = storage.load_leaf_by_hash(&leaf3.leaf_hash).await.expect("Error loading leaf3");
        assert!(found_leaf3.is_some(), "Leaf 3 was not found");
        assert_eq!(found_leaf3.unwrap().leaf_hash, leaf3.leaf_hash, "Found leaf hash does not match leaf3");

        // 5. Test retrieving a leaf from the first L0 page
        let found_leaf1 = storage.load_leaf_by_hash(&leaf1.leaf_hash).await.expect("Error loading leaf1");
        assert!(found_leaf1.is_some(), "Leaf 1 was not found");
        assert_eq!(found_leaf1.unwrap().leaf_hash, leaf1.leaf_hash, "Found leaf hash does not match leaf1");

        // 6. Test retrieving a non-existent leaf
        let non_existent_leaf_data = crate::core::leaf::LeafDataV1 {
            timestamp: Utc::now() + chrono::Duration::days(10), // Ensure distinct timestamp
            content_type: "text/plain".to_string(),
            content: b"non-existent-content".to_vec(),
            author: "non_existent_author".to_string(),
            signature: "non_existent_sig".to_string(),
        };
        let non_existent_payload = serde_json::to_value(crate::core::leaf::LeafData::V1(non_existent_leaf_data.clone()))
            .expect("Failed to serialize non_existent_leaf_data");
        let non_existent_hash = JournalLeaf::new(
            non_existent_leaf_data.timestamp,
            None, 
            "non_existent_container".to_string(),
            non_existent_payload
        ).expect("Failed to create non_existent_leaf for hash generation").leaf_hash;
        let not_found_leaf = storage.load_leaf_by_hash(&non_existent_hash).await.expect("Error loading non-existent leaf");
        assert!(not_found_leaf.is_none(), "Found a leaf that should not exist");

        // 7. Test retrieving a leaf that might have been on an L1 page (leaf4 was created but not added as a leaf to page1_1)
        // load_leaf_by_hash only scans L0 pages, so leaf4 should not be found even if it were on an L1 page as a leaf.
        let found_leaf4 = storage.load_leaf_by_hash(&leaf4.leaf_hash).await.expect("Error loading leaf4");
        assert!(found_leaf4.is_none(), "Leaf 4 (from L1 page context) should not be found by load_leaf_by_hash which scans L0");
    }
}


impl FileStorage {
    /* pub async fn backup_journal_to_directory_raw(&self, backup_target_path: &Path) -> Result<(), CJError> {
        let source_journal_root = self.base_path.join(JOURNAL_SUBDIR);
        let target_journal_root = backup_target_path.join(JOURNAL_SUBDIR);

        if !fs::try_exists(&source_journal_root).await.map_err(|e| 
            CJError::StorageError(format!("Source journal directory {} does not exist: {}", source_journal_root.display(), e))
        )? {
            // If source doesn't exist, there's nothing to backup, so no manifest needed.
            return Ok(());
        }

        fs::create_dir_all(&target_journal_root).await.map_err(|e| 
            CJError::StorageError(format!("Failed to create target journal root directory {}: {}", target_journal_root.display(), e))
        )?;

        let mut manifest_files: Vec<ManifestFileEntry> = Vec::new();

        let mut level_dirs = fs::read_dir(&source_journal_root).await.map_err(|e| 
            CJError::StorageError(format!("Failed to read source journal root {}: {}", source_journal_root.display(), e))
        )?;

        while let Some(level_dir_entry) = level_dirs.next_entry().await.map_err(|e| 
            CJError::StorageError(format!("Failed to read entry in source journal root {}: {}", source_journal_root.display(), e))
        )? {
            let source_level_path = level_dir_entry.path();
            if source_level_path.is_dir() && level_dir_entry.file_name().to_string_lossy().starts_with("level_") {
                let target_level_path = target_journal_root.join(level_dir_entry.file_name());
                fs::create_dir_all(&target_level_path).await.map_err(|e| 
                    CJError::StorageError(format!("Failed to create target level directory {}: {}", target_level_path.display(), e))
                )?;

                let mut page_files = fs::read_dir(&source_level_path).await.map_err(|e| 
                    CJError::StorageError(format!("Failed to read source level directory {}: {}", source_level_path.display(), e))
                )?;

                while let Some(page_file_entry) = page_files.next_entry().await.map_err(|e| 
                    CJError::StorageError(format!("Failed to read entry in source level directory {}: {}", source_level_path.display(), e))
                )? {
                    let source_page_file_path = page_file_entry.path();
                    if source_page_file_path.is_file() && 
                       page_file_entry.file_name().to_string_lossy().starts_with("page_") && 
                       source_page_file_path.extension().is_some_and(|ext| ext == "cjt") {
                        
                        // 1. Read source file content for hashing and size
                        let file_content = fs::read(&source_page_file_path).await.map_err(|e|
                            CJError::StorageError(format!("Failed to read source page file {} for hashing: {}", source_page_file_path.display(), e))
                        )?;
                        let file_size = file_content.len() as u64;

                        let mut hasher = Sha256::new();
                        hasher.update(&file_content);
                        let hash_bytes = hasher.finalize();
                        let sha256_hash_str = hex::encode(hash_bytes);

                        // 2. Write the file content to the target path
                        let target_page_file_path = target_level_path.join(page_file_entry.file_name());
                        fs::write(&target_page_file_path, &file_content).await.map_err(|e|
                             CJError::StorageError(format!("Failed to write page file to {}: {}", target_page_file_path.display(), e))
                        )?;

                        // 3. Prepare ManifestFileInfo
                        let path_in_journal_subdir = source_page_file_path.strip_prefix(&source_journal_root)
                            .map_err(|_e| CJError::StorageError(format!("Failed to strip prefix for manifest path: {} from {}", source_page_file_path.display(), source_journal_root.display())))?
                            .to_string_lossy().into_owned();
                        
                        let manifest_relative_path = Path::new(JOURNAL_SUBDIR).join(path_in_journal_subdir).to_string_lossy().into_owned();

                        manifest_files.push(ManifestFileEntry {
                            relative_path: manifest_relative_path,
                            size_bytes: file_size,
                            sha256_hash: sha256_hash_str,
                        });
                    }
                }
            }
        }

        // After all files are copied (or if no files), create and write the manifest
        let manifest_storage_config = ManifestStorageConfig {
            base_path: self.base_path.to_string_lossy().into_owned(),
            compression_algorithm: format!("{:?}", self.compression_config.algorithm),
        };

        let backup_manifest = BackupManifest {
            manifest_file_format_version: MANIFEST_VERSION.to_string(),
            backup_timestamp_utc: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            source_storage_details: manifest_storage_config,
            files: manifest_files,
            backup_tool_version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let manifest_content = serde_json::to_string_pretty(&backup_manifest).map_err(|e|
            CJError::StorageError(format!("Failed to serialize backup manifest: {}", e))
        )?;

        let manifest_path = backup_target_path.join(BACKUP_MANIFEST_FILENAME);
        fs::write(&manifest_path, manifest_content).await.map_err(|e|
            CJError::StorageError(format!("Failed to write backup manifest to {}: {}", manifest_path.display(), e))
        )?;

        Ok(())
    } */

}

#[cfg(test)]
mod tests_to_merge {
    use super::*;
    
    use chrono::Utc;
    use tempfile::tempdir;
    use crate::core::page::JournalPage;
use crate::LevelRollupConfig;
    
    use crate::config::{Config, StorageConfig, CompressionConfig, LoggingConfig, MetricsConfig, RetentionConfig, SnapshotConfig};
    use crate::types::time::{TimeLevel};
    use crate::{StorageType, TimeHierarchyConfig, CompressionAlgorithm};
    use std::fs::File as StdFile; // For opening the backup zip file

    fn get_base_test_config() -> Config { 
        Config {
            force_rollup_on_shutdown: false,
            time_hierarchy: TimeHierarchyConfig {
                levels: vec![
                    TimeLevel {
                            rollup_config: LevelRollupConfig::default(),
                            retention_policy: None, name: "second".to_string(), duration_seconds: 1 },
                    TimeLevel {
                            rollup_config: LevelRollupConfig::default(),
                            retention_policy: None, name: "minute".to_string(), duration_seconds: 60 },
                ]
            },
            // force_rollup_on_shutdown defaults to false in Config::default()
            // For file storage tests, we might still use Memory for config simplicity unless test needs file specifics
            storage: StorageConfig { storage_type: StorageType::Memory, base_path: "./cjtmp_file_test".to_string(), max_open_files: 100 }, 
            compression: CompressionConfig::default(),
            logging: LoggingConfig::default(),
            metrics: MetricsConfig::default(),
            retention: RetentionConfig::default(),
            snapshot: SnapshotConfig::default(),
        }
    }

    fn create_test_page(level: u8, creation_timestamp: chrono::DateTime<Utc>, prev_hash: Option<[u8; 32]>, config: &Config) -> JournalPage {
        let mut page = JournalPage::new(level, prev_hash, creation_timestamp, config);
        if level == 0 {
            let dummy_leaf_data = crate::core::leaf::LeafDataV1 {
                timestamp: creation_timestamp,
                content_type: "application/octet-stream".to_string(),
                content: vec![0u8; 10],
                author: "dummy_author".to_string(),
                signature: "dummy_signature".to_string(),
            };
            let dummy_leaf_json = serde_json::to_value(crate::core::leaf::LeafData::V1(dummy_leaf_data)).expect("Failed to serialize dummy leaf data");
            let dummy_journal_leaf = JournalLeaf::new(
                creation_timestamp, 
                None, 
                "dummy_container".to_string(), 
                dummy_leaf_json
            ).expect("Failed to create dummy journal leaf");
            page.add_leaf(dummy_journal_leaf);
        } else {
            // Add a dummy thrall page hash for L1+
            page.add_thrall_hash([0u8; 32], creation_timestamp);
        }
        page.recalculate_merkle_root_and_page_hash();
        page
    }

    #[tokio::test]
    async fn test_new_file_storage() {
        let dir = tempdir().unwrap();
        let config = get_base_test_config();
        let storage = FileStorage::new(dir.path(), config.compression.clone()).await;
        assert!(storage.is_ok());
        assert!(dir.path().join(MARKER_FILE_NAME).exists());
    }

    #[tokio::test]
    async fn test_store_and_load_page_with_zstd_compression() {
                let dir = tempdir().unwrap();
        let compression_config = CompressionConfig {
            enabled: true,
            algorithm: CompressionAlgorithm::Zstd,
            level: 3,
        };
        let storage = FileStorage::new(dir.path(), compression_config).await.unwrap();
        
        let page_config = get_base_test_config(); // Use base config for creating the page itself
        let now = Utc::now();
        let page_to_store = create_test_page(0, now, None, &page_config);

        let store_result = storage.store_page(&page_to_store).await;
        assert!(store_result.is_ok(), "Store failed: {:?}", store_result.err());

        // Verify file extension
        let expected_path = storage.get_page_path(page_to_store.level, page_to_store.page_id);
        assert!(expected_path.to_string_lossy().ends_with(".cjt"), "File extension should be .cjt");
        assert!(fs::try_exists(&expected_path).await.unwrap(), "Compressed file should exist");

        let loaded_page_opt = storage.load_page(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(loaded_page_opt.is_some(), "Load failed to find page");
        let loaded_page = loaded_page_opt.unwrap();
        assert_eq!(loaded_page.page_id, page_to_store.page_id);
        assert_eq!(loaded_page.level, page_to_store.level);
        assert_eq!(loaded_page.merkle_root, page_to_store.merkle_root);
    }

    #[tokio::test]
    async fn test_store_and_load_page_with_lz4_compression() {
                let dir = tempdir().unwrap();
        let compression_config = CompressionConfig {
            enabled: true,
            algorithm: CompressionAlgorithm::Lz4,
            level: 0, // LZ4 level is typically not configured like Zstd, 0 is fine
        };
        let storage = FileStorage::new(dir.path(), compression_config).await.unwrap();
        
        let page_config = get_base_test_config();
        let now = Utc::now();
        let page_to_store = create_test_page(0, now, None, &page_config);

        let store_result = storage.store_page(&page_to_store).await;
        assert!(store_result.is_ok(), "Store failed: {:?}", store_result.err());

        let expected_path = storage.get_page_path(page_to_store.level, page_to_store.page_id);
        assert!(expected_path.to_string_lossy().ends_with(".cjt"), "File extension should be .cjt");
        assert!(fs::try_exists(&expected_path).await.unwrap(), "Compressed file should exist");

        let loaded_page_opt = storage.load_page(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(loaded_page_opt.is_some(), "Load failed to find page");
        let loaded_page = loaded_page_opt.unwrap();
        assert_eq!(loaded_page.page_id, page_to_store.page_id);
        assert_eq!(loaded_page.level, page_to_store.level);
        assert_eq!(loaded_page.merkle_root, page_to_store.merkle_root);
    }

    #[tokio::test]
    async fn test_store_and_load_page_with_snappy_compression() {
                let dir = tempdir().unwrap();
        let compression_config = CompressionConfig {
            enabled: true,
            algorithm: CompressionAlgorithm::Snappy,
            level: 0, // Snappy does not have levels
        };
        let storage = FileStorage::new(dir.path(), compression_config).await.unwrap();
        
        let page_config = get_base_test_config();
        let now = Utc::now();
        let page_to_store = create_test_page(0, now, None, &page_config);

        let store_result = storage.store_page(&page_to_store).await;
        assert!(store_result.is_ok(), "Store failed: {:?}", store_result.err());

        let expected_path = storage.get_page_path(page_to_store.level, page_to_store.page_id);
        assert!(expected_path.to_string_lossy().ends_with(".cjt"), "File extension should be .cjt");
        assert!(fs::try_exists(&expected_path).await.unwrap(), "Compressed file should exist");

        let loaded_page_opt = storage.load_page(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(loaded_page_opt.is_some(), "Load failed to find page");
        let loaded_page = loaded_page_opt.unwrap();
        assert_eq!(loaded_page.page_id, page_to_store.page_id);
        assert_eq!(loaded_page.level, page_to_store.level);
        assert_eq!(loaded_page.merkle_root, page_to_store.merkle_root);
    }

    #[tokio::test]
    async fn test_store_and_load_page_with_compression_enabled_none_algorithm() {
                let dir = tempdir().unwrap();
        let compression_config = CompressionConfig {
            enabled: true,
            algorithm: CompressionAlgorithm::None,
            level: 0,
        };
        let storage = FileStorage::new(dir.path(), compression_config).await.unwrap();
        
        let page_config = get_base_test_config();
        let now = Utc::now();
        let page_to_store = create_test_page(0, now, None, &page_config);

        let store_result = storage.store_page(&page_to_store).await;
        assert!(store_result.is_ok(), "Store failed: {:?}", store_result.err());

        let expected_path = storage.get_page_path(page_to_store.level, page_to_store.page_id);
        assert!(expected_path.to_string_lossy().ends_with(".cjt"), "File extension should be .cjt for CompressionAlgorithm::None");
        assert!(fs::try_exists(&expected_path).await.unwrap(), "File should exist");

        let loaded_page_opt = storage.load_page(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(loaded_page_opt.is_some(), "Load failed to find page");
        let loaded_page = loaded_page_opt.unwrap();
        assert_eq!(loaded_page.page_id, page_to_store.page_id);
        assert_eq!(loaded_page.level, page_to_store.level);
        assert_eq!(loaded_page.merkle_root, page_to_store.merkle_root);
    }

    #[tokio::test]
    async fn test_store_and_load_page() {
                let dir = tempdir().unwrap();
        let config = get_base_test_config();
        let storage = FileStorage::new(dir.path(), config.compression.clone()).await.unwrap();
        let now = Utc::now();
        let page_to_store = create_test_page(0, now, None, &config);

        let store_result = storage.store_page(&page_to_store).await;
        assert!(store_result.is_ok(), "Store failed: {:?}", store_result.err());

        // Verify file extension for uncompressed (default test config)
        let expected_path = storage.get_page_path(page_to_store.level, page_to_store.page_id);
        assert!(expected_path.to_string_lossy().ends_with(".cjt"), "File extension should be .cjt for uncompressed");
        assert!(fs::try_exists(&expected_path).await.unwrap(), "Uncompressed file should exist");

        let loaded_page_opt = storage.load_page(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(loaded_page_opt.is_some());
        let loaded_page = loaded_page_opt.unwrap();
        assert_eq!(loaded_page.page_id, page_to_store.page_id);
        assert_eq!(loaded_page.level, page_to_store.level);
        assert_eq!(loaded_page.merkle_root, page_to_store.merkle_root);
    }

    #[tokio::test]
    async fn test_load_non_existent_page() {
        let dir = tempdir().unwrap();
        let config = get_base_test_config();
        let storage = FileStorage::new(dir.path(), config.compression.clone()).await.unwrap();
        let loaded_page_opt = storage.load_page(0, 99).await.unwrap();
        assert!(loaded_page_opt.is_none());
    }

    #[tokio::test]
    async fn test_page_exists() {
                let dir = tempdir().unwrap();
        let config = get_base_test_config();
        let storage = FileStorage::new(dir.path(), config.compression.clone()).await.unwrap();
        let now = Utc::now();
        let page_to_store = create_test_page(1, now, None, &config);

        let exists_before_store = storage.page_exists(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(!exists_before_store);

        storage.store_page(&page_to_store).await.unwrap();

        let exists_after_store = storage.page_exists(page_to_store.level, page_to_store.page_id).await.unwrap();
        assert!(exists_after_store);
    }

    #[tokio::test]
    #[ignore]
    async fn test_backup_journal() {
        
        // 1. Setup FileStorage
        let source_dir = tempdir().unwrap();
        let storage_config = CompressionConfig { // Use default (no compression) for simplicity
            enabled: false,
            algorithm: CompressionAlgorithm::None,
            level: 0,
        };
        let storage = FileStorage::new(source_dir.path(), storage_config.clone()).await.unwrap();
        
        let page_config = get_base_test_config(); // For creating pages
        let now = Utc::now();

        // 2. Store a few pages
        // page_id will be auto-incremented by create_test_page due to Option<u64> for page_id_override being None
        let page_l0_id0 = create_test_page(0, now, None, &page_config); // Expected page_id: 0, level: 0
        let page_l0_id1 = create_test_page(0, now, None, &page_config); // Expected page_id: 1, level: 0
        let page_l1_id2 = create_test_page(1, now, None, &page_config); // Expected page_id: 2, level: 1

        storage.store_page(&page_l0_id0).await.unwrap();
        storage.store_page(&page_l0_id1).await.unwrap();
        storage.store_page(&page_l1_id2).await.unwrap();

        // 3. Perform backup
        let backup_dir_temp = tempdir().unwrap(); // Temporary directory to hold the zip file
        let backup_zip_path = backup_dir_temp.path().join("backup.zip");

        storage.backup_journal(&backup_zip_path).await.unwrap();

        // 4. Unzip the backup to a temporary location for verification
        let verification_dir = tempdir().unwrap();
        let backup_file_for_verification = StdFile::open(&backup_zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(backup_file_for_verification).unwrap();
        archive.extract(verification_dir.path()).unwrap();
        
        // 5. Assertions: Check contents of verification_dir
        // Check level 0
        let backup_level0_path = verification_dir.path().join("level_0");
        assert!(fs::try_exists(&backup_level0_path).await.unwrap(), "Backup level_0 directory does not exist");
        let page_l0_id0_backup_path = backup_level0_path.join(format!("page_{}.cjt", page_l0_id0.page_id));
        let page_l0_id1_backup_path = backup_level0_path.join(format!("page_{}.cjt", page_l0_id1.page_id));
        assert!(fs::try_exists(&page_l0_id0_backup_path).await.unwrap(), "Backup page_{}_L0.cjt does not exist", page_l0_id0.page_id);
        assert!(fs::try_exists(&page_l0_id1_backup_path).await.unwrap(), "Backup page_{}_L0.cjt does not exist", page_l0_id1.page_id);

        // Check level 1
        let backup_level1_path = verification_dir.path().join("level_1");
        assert!(fs::try_exists(&backup_level1_path).await.unwrap(), "Backup level_1 directory does not exist");
        let page_l1_id2_backup_path = backup_level1_path.join(format!("page_{}.cjt", page_l1_id2.page_id));
        assert!(fs::try_exists(&page_l1_id2_backup_path).await.unwrap(), "Backup page_{}_L1.cjt does not exist", page_l1_id2.page_id);

        // Optional: Compare file content/size (simple check: compare sizes)
        let original_page_l0_id0_path = storage.get_page_path(page_l0_id0.level, page_l0_id0.page_id);
        let original_page_l0_id0_meta = fs::metadata(&original_page_l0_id0_path).await.unwrap();
        let backup_page_l0_id0_meta = fs::metadata(&page_l0_id0_backup_path).await.unwrap();
        assert_eq!(original_page_l0_id0_meta.len(), backup_page_l0_id0_meta.len(), "File size mismatch for page_{}_L0.cjt", page_l0_id0.page_id);

        // Manifest verification is not applicable to the zip backup from StorageBackend::backup_journal.
        // That logic is part of backup_journal_to_directory_raw.
    }

    #[tokio::test]
    #[ignore]
    async fn test_restore_journal() {
        // 1. Setup source storage and populate it
        let source_storage_dir = tempdir().unwrap();
        let compression_config = CompressionConfig {
            enabled: false,
            algorithm: CompressionAlgorithm::None,
            level: 0,
        };
        let source_storage = FileStorage::new(source_storage_dir.path(), compression_config.clone()).await.unwrap();

        let now = Utc::now();
        let page_config = get_base_test_config(); // Ensure this helper is available and provides Arc<Config> or &Config

        // Create leaves for L0 pages
        let leaf1_payload_v1_data = crate::core::leaf::LeafDataV1 { timestamp: now, content_type: "text/plain".to_string(), content: b"leaf1 data".to_vec(), author: "test".to_string(), signature: "sig".to_string() };
        let _leaf1 = JournalLeaf::new(
            leaf1_payload_v1_data.timestamp,
            None, // prev_leaf_hash
            "container1".to_string(),
            serde_json::to_value(crate::core::leaf::LeafData::V1(leaf1_payload_v1_data.clone())).expect("Failed to serialize LeafData::V1 for leaf1")
        ).expect("Failed to create leaf1");

        let leaf2_payload_v1_data = crate::core::leaf::LeafDataV1 { timestamp: now, content_type: "text/plain".to_string(), content: b"leaf2 data".to_vec(), author: "test".to_string(), signature: "sig".to_string() };
        let _leaf2 = JournalLeaf::new(
            leaf2_payload_v1_data.timestamp,
            None, // prev_leaf_hash
            "container1".to_string(), 
            serde_json::to_value(crate::core::leaf::LeafData::V1(leaf2_payload_v1_data.clone())).expect("Failed to serialize LeafData::V1 for leaf2")
        ).expect("Failed to create leaf2");

        let leaf1_payload_v1_restore = crate::core::leaf::LeafDataV1 { timestamp: now, content_type: "text/plain".to_string(), content: b"leaf1 data".to_vec(), author: "test".to_string(), signature: "sig".to_string() };
        let leaf1 = JournalLeaf::new(
            leaf1_payload_v1_restore.timestamp,
            None, // prev_leaf_hash
            "restore_container_1".to_string(),
            serde_json::to_value(crate::core::leaf::LeafData::V1(leaf1_payload_v1_restore.clone())).expect("Failed to serialize LeafData::V1 for restore leaf1")
        ).expect("Failed to create leaf1 for restore");

        let leaf2_payload_v1_restore = crate::core::leaf::LeafDataV1 { timestamp: now + chrono::Duration::seconds(2), content_type: "text/plain".to_string(), content: b"leaf2 data".to_vec(), author: "test".to_string(), signature: "sig".to_string() };
        let leaf2 = JournalLeaf::new(
            leaf2_payload_v1_restore.timestamp,
            None, // prev_leaf_hash
            "restore_container_2".to_string(),
            serde_json::to_value(crate::core::leaf::LeafData::V1(leaf2_payload_v1_restore.clone())).expect("Failed to serialize LeafData::V1 for restore leaf2")
        ).expect("Failed to create leaf2 for restore");

        // Create pages with distinct IDs
        let mut p1_l0 = JournalPage::new(0, None, now, &page_config);
        p1_l0.page_id = 0;
        p1_l0.add_leaf(leaf1.clone());

        let mut p2_l0 = JournalPage::new(0, Some(p1_l0.page_hash), now + chrono::Duration::seconds(1), &page_config);
        p2_l0.page_id = 1;
        p2_l0.add_leaf(leaf2.clone());

        let mut p3_l1 = JournalPage::new(1, None, now + chrono::Duration::seconds(4), &page_config);
        p3_l1.page_id = 2;
        // For L1, add thrall hashes (e.g., hash of p1_l0 and p2_l0)
        p3_l1.add_thrall_hash(p1_l0.page_hash, p1_l0.end_time);
        p3_l1.add_thrall_hash(p2_l0.page_hash, p2_l0.end_time);

        source_storage.store_page(&p1_l0).await.unwrap();
        source_storage.store_page(&p2_l0).await.unwrap();
        source_storage.store_page(&p3_l1).await.unwrap();

        // 2. Create a backup
        let backup_zip_dir = tempdir().unwrap();
        let backup_zip_path = backup_zip_dir.path().join("test_restore_backup.zip");
        source_storage.backup_journal(&backup_zip_path).await.unwrap();
        assert!(tokio::fs::try_exists(&backup_zip_path).await.unwrap(), "Backup zip file was not created.");

        // 3. Setup target storage
        let target_storage_temp_dir = tempdir().unwrap();
        let target_storage_base_path = target_storage_temp_dir.path();
        // The restore_journal function expects the direct path to the 'journal' subdirectory for its target_journal_dir argument.
        let target_journal_dir_path = target_storage_base_path.join(JOURNAL_SUBDIR);

        let target_storage = FileStorage::new(target_storage_base_path, compression_config.clone()).await.unwrap();

        // 4. Create a dummy file in the target journal directory to ensure it's cleared by restore
        let dummy_level_dir = target_journal_dir_path.join("level_0");
        tokio::fs::create_dir_all(&dummy_level_dir).await.unwrap();
        let dummy_page_file_path = dummy_level_dir.join("page_99.cjt");
        tokio::fs::write(&dummy_page_file_path, "dummy_content").await.unwrap();
        assert!(tokio::fs::try_exists(&dummy_page_file_path).await.unwrap(), "Dummy page file was not created for test.");

        // 5. Restore the journal
        StorageBackend::restore_journal(&target_storage, &backup_zip_path, &target_journal_dir_path).await.unwrap();

        // 6. Verify restored content
        // Check that dummy page is gone
        assert!(!tokio::fs::try_exists(&dummy_page_file_path).await.unwrap(), "Dummy page file should have been removed by restore.");

        // Check that restored pages exist in target_storage using its API
        assert!(target_storage.page_exists(p1_l0.level, p1_l0.page_id).await.unwrap(), "Restored page p1_l0 (L0/P0) not found in target storage.");
        assert!(target_storage.page_exists(p2_l0.level, p2_l0.page_id).await.unwrap(), "Restored page p2_l0 (L0/P1) not found in target storage.");
        assert!(target_storage.page_exists(p3_l1.level, p3_l1.page_id).await.unwrap(), "Restored page p3_l1 (L1/P2) not found in target storage.");

        // Load a page and compare some details
        let loaded_p1_opt = target_storage.load_page(p1_l0.level, p1_l0.page_id).await.unwrap();
        assert!(loaded_p1_opt.is_some(), "Failed to load restored page p1_l0.");
        let loaded_p1 = loaded_p1_opt.unwrap();
        assert_eq!(loaded_p1.page_id, p1_l0.page_id, "Mismatch in page_id for loaded p1_l0");
        assert_eq!(loaded_p1.level, p1_l0.level, "Mismatch in level for loaded p1_l0");
        assert_eq!(loaded_p1.merkle_root, p1_l0.merkle_root, "Mismatch in merkle_root for loaded p1_l0");
        assert_eq!(loaded_p1.creation_timestamp, p1_l0.creation_timestamp, "Mismatch in creation_timestamp for loaded p1_l0");
        assert_eq!(loaded_p1.end_time, p1_l0.end_time, "Mismatch in end_time for loaded p1_l0");
        match (&loaded_p1.content, &p1_l0.content) {
            (PageContent::Leaves(loaded_leaves), PageContent::Leaves(original_leaves)) => {
                assert_eq!(loaded_leaves.len(), original_leaves.len(), "Mismatch in number of leaves for loaded p1_l0");
                if !original_leaves.is_empty() && !loaded_leaves.is_empty() {
                    assert_eq!(loaded_leaves[0].leaf_hash, original_leaves[0].leaf_hash, "Mismatch in leaf hash for p1_l0's first leaf");
                }
            }
            _ => panic!("Content type mismatch or unexpected content for p1_l0"),
        }

        // Verify file structure directly in the target_journal_dir_path
        assert!(tokio::fs::try_exists(&target_journal_dir_path).await.unwrap(), "Target journal directory does not exist after restore.");

        let target_level0_path = target_journal_dir_path.join("level_0");
        assert!(tokio::fs::try_exists(&target_level0_path).await.unwrap(), "Target L0 directory does not exist in filesystem.");
        assert!(tokio::fs::try_exists(&target_level0_path.join(format!("page_{}.cjt", p1_l0.page_id))).await.unwrap(), "File for p1_l0 (L0/P0) not found in filesystem.");
        assert!(tokio::fs::try_exists(&target_level0_path.join(format!("page_{}.cjt", p2_l0.page_id))).await.unwrap(), "File for p2_l0 (L0/P1) not found in filesystem.");
        
        let target_level1_path = target_journal_dir_path.join("level_1");
        assert!(tokio::fs::try_exists(&target_level1_path).await.unwrap(), "Target L1 directory does not exist in filesystem.");
        assert!(tokio::fs::try_exists(&target_level1_path.join(format!("page_{}.cjt", p3_l1.page_id))).await.unwrap(), "File for p3_l1 (L1/P2) not found in filesystem.");
    }
}
