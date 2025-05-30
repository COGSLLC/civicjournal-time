// src/core/merkle.rs

use crate::error::CJError;
use sha2::{Digest, Sha256};

// TODO: Implement Merkle tree logic
// - Function to build a Merkle tree from a list of leaf hashes
// - Function to generate a Merkle proof for a leaf
// - Function to verify a Merkle proof

pub fn calculate_merkle_root(leaf_hashes: &Vec<Vec<u8>>) -> Result<Vec<u8>, CJError> {
    if leaf_hashes.is_empty() {
        return Err(CJError::InvalidInput("Cannot calculate Merkle root for empty list of hashes".to_string()));
    }
    // Simplified: In a real implementation, handle tree construction for non-power-of-2 leaves
    if leaf_hashes.len() == 1 {
        return Ok(leaf_hashes[0].clone());
    }
    // For now, just hash the concatenation of all hashes as a placeholder
    let mut hasher = Sha256::new();
    for hash in leaf_hashes {
        hasher.update(hash);
    }
    Ok(hasher.finalize().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_merkle_root() {
        let hash1 = Sha256::digest(b"leaf1").to_vec();
        let hash2 = Sha256::digest(b"leaf2").to_vec();
        let hashes = vec![hash1, hash2];
        assert!(calculate_merkle_root(&hashes).is_ok());
    }
}
