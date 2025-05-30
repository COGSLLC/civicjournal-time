// src/storage/mod.rs

// Define sub-modules for storage backends
pub mod memory;
pub mod file;

// Define a common storage trait
// pub trait StorageBackend { ... }

// Re-export implementations or the trait
// e.g., pub use memory::InMemoryStorage;
// e.g., pub use file::FileStorage;
