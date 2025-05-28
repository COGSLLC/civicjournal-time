use anyhow::Result;
use sha2::{Sha256, Digest};

fn main() -> Result<()> {
    println!("Minimal CivicChain Time test");
    
    // Simple test of SHA-2 hashing
    let mut hasher = Sha256::new();
    hasher.update(b"test");
    let result = hasher.finalize();
    println!("SHA-256 of 'test': {:x}", result);
    
    Ok(())
}
