// src/core/hash.rs

use sha2::{Digest, Sha256};

/// Computes the SHA256 hash of the given data.
pub fn sha256_hash(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Computes the SHA256 hash of a list of byte slices concatenated together.
pub fn sha256_hash_concat(data_slices: &[&[u8]]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    for slice in data_slices {
        hasher.update(slice);
    }
    hasher.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex;

    #[test]
    fn test_sha256_hash() {
        let data = b"hello world";
        let expected_hash = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let actual_hash = sha256_hash(data);
        assert_eq!(hex::encode(actual_hash), expected_hash);
    }

    #[test]
    fn test_sha256_hash_concat() {
        let data1 = b"hello";
        let data2 = b" world";
        let expected_hash = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let actual_hash = sha256_hash_concat(&[data1, data2]);
        assert_eq!(hex::encode(actual_hash), expected_hash);
    }
}
