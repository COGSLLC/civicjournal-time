// src/lib.rs

pub mod core;
pub mod time;
pub mod storage;
pub mod api;
pub mod ffi;
pub mod error;

// Re-export key types or functions for easier use by consumers of the library
// pub use error::CJError; // We'll uncomment this once error.rs is created

/// A simple function to confirm the library is linked.
pub fn hello() -> String {
    "Hello from CivicJournal-Time!".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        assert_eq!(hello(), "Hello from CivicJournal-Time!");
    }
}
