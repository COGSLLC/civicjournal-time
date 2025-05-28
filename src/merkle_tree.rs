use sha2::{Digest, Sha256};

/// Merkle tree implementation for efficient hash-based verification
#[derive(Debug, Clone)]
pub struct MerkleTree {
    /// The root hash of the Merkle tree
    root_hash: [u8; 32],
    /// The leaves of the tree
    leaves: Vec<[u8; 32]>,
    /// Internal nodes of the tree
    nodes: Vec<[u8; 32]>,
}

impl MerkleTree {
    /// Creates a new Merkle tree from a list of leaf hashes
    pub fn new(leaves: Vec<[u8; 32]>) -> Self {
        if leaves.is_empty() {
            return Self {
                root_hash: [0u8; 32],
                leaves: Vec::new(),
                nodes: Vec::new(),
            };
        }

        let mut nodes = Vec::new();
        let mut current_level = leaves.clone();

        while current_level.len() > 1 {
            let mut next_level = Vec::new();
            for pair in current_level.chunks(2) {
                let hash = if pair.len() == 2 {
                    Self::hash_pair(&pair[0], &pair[1])
                } else {
                    // For odd number of nodes, duplicate the last one
                    Self::hash_pair(&pair[0], &pair[0])
                };
                nodes.push(hash);
                next_level.push(hash);
            }
            current_level = next_level;
        }

        let root_hash = current_level[0];
        
        Self {
            root_hash,
            leaves,
            nodes,
        }
    }

    /// Gets the root hash of the Merkle tree
    pub fn root_hash(&self) -> &[u8; 32] {
        &self.root_hash
    }

    /// Generates a Merkle proof for a given leaf index
    pub fn generate_proof(&self, index: usize) -> Option<Vec<[u8; 32]>> {
        if index >= self.leaves.len() {
            return None;
        }

        let mut proof = Vec::new();
        let mut current_index = index;
        let mut level_size = self.leaves.len();
        let mut level_start = 0;
        let mut tree_index = 0;

        while level_size > 1 {
            let is_right_node = current_index % 2 == 1;
            let pair_index = if is_right_node {
                current_index - 1
            } else if current_index + 1 < level_size {
                current_index + 1
            } else {
                // Last node on an odd level
                current_index
            };

            if pair_index != current_index {
                proof.push(if is_right_node {
                    self.leaves[level_start + pair_index]
                } else {
                    self.leaves[level_start + pair_index]
                });
            }

            current_index /= 2;
            level_start += level_size;
            level_size = (level_size + 1) / 2;
            tree_index += level_size;
        }

        Some(proof)
    }

    /// Verifies a Merkle proof
    pub fn verify_proof(leaf_hash: &[u8; 32], proof: &[[u8; 32]], root_hash: &[u8; 32]) -> bool {
        let mut computed_hash = *leaf_hash;
        let mut hash = Sha256::new();

        for proof_hash in proof {
            // Determine order of hashing (left or right)
            if computed_hash <= *proof_hash {
                hash = Sha256::new()
                    .chain_update(&computed_hash)
                    .chain_update(proof_hash);
            } else {
                hash = Sha256::new()
                    .chain_update(proof_hash)
                    .chain_update(&computed_hash);
            }
            computed_hash = hash.finalize().into();
            hash = Sha256::new();
        }

        computed_hash == *root_hash
    }

    /// Hashes a pair of nodes
    fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        if left <= right {
            Sha256::new()
                .chain_update(left)
                .chain_update(right)
                .finalize()
                .into()
        } else {
            Sha256::new()
                .chain_update(right)
                .chain_update(left)
                .finalize()
                .into()
        }
    }
}

impl Default for MerkleTree {
    fn default() -> Self {
        Self {
            root_hash: [0u8; 32],
            leaves: Vec::new(),
            nodes: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merkle_tree() {
        // Test with an even number of leaves
        let leaves = vec![
            *b"This is the first leaf in the tree.",
            *b"Second leaf is right here!",
            *b"Third leaf is getting longer...",
            *b"Fourth and final leaf.",
        ];
        
        let tree = MerkleTree::new(leaves.clone());
        
        // Verify the root hash is consistent
        let proof = tree.generate_proof(0).unwrap();
        assert!(MerkleTree::verify_proof(&leaves[0], &proof, tree.root_hash()));
        
        // Test with an odd number of leaves
        let leaves = vec![
            *b"Single leaf tree test",
        ];
        
        let tree = MerkleTree::new(leaves.clone());
        assert_eq!(tree.root_hash(), &leaves[0]);
    }
}
