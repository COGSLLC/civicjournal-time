// src/core/merkle.rs

use crate::error::CJError;
use sha2::{Digest, Sha256};

/// Type alias for a Merkle root, which is a SHA256 hash.
pub type MerkleRoot = [u8; 32];

/// Indicates the position of a sibling hash in a Merkle proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofDirection {
    /// Indicates the sibling hash is to the left of the current hash in the pair.
    Left,
    /// Indicates the sibling hash is to the right of the current hash in the pair.
    Right,
}

/// Represents a Merkle proof for a single leaf.
/// It's a list of sibling hashes and their direction needed to reconstruct the root.
pub type MerkleProof = Vec<([u8; 32], ProofDirection)>;

/// Represents a Merkle tree.
///
/// The tree is stored as a vector of levels, where each level is a vector of hashes.
/// `nodes[0]` is the leaf layer, and `nodes[nodes.len() - 1]` is the root layer,
/// containing a single hash.
#[derive(Debug, Clone)]
pub struct MerkleTree {
    nodes: Vec<Vec<[u8; 32]>>,
}

impl MerkleTree {
    /// Creates a new Merkle tree from a vector of leaf hashes.
    ///
    /// # Arguments
    ///
    /// * `leaf_hashes` - A vector of 32-byte arrays, representing the leaf hashes.
    ///
    /// # Errors
    ///
    /// Returns `CJError::InvalidInput` if `leaf_hashes` is empty.
    pub fn new(leaf_hashes: Vec<[u8; 32]>) -> Result<Self, CJError> {
        if leaf_hashes.is_empty() {
            return Err(CJError::InvalidInput(
                "Cannot construct Merkle tree with no leaf hashes".to_string(),
            ));
        }

        let mut nodes: Vec<Vec<[u8; 32]>> = Vec::new();
        nodes.push(leaf_hashes);

        while nodes.last().unwrap().len() > 1 {
            let previous_level = nodes.last().unwrap();
            let mut current_level: Vec<[u8; 32]> = Vec::new();
            
            let mut i = 0;
            while i < previous_level.len() {
                let left_hash = previous_level[i];
                let right_hash = if i + 1 < previous_level.len() {
                    previous_level[i + 1]
                } else {
                    // If odd number of nodes, duplicate the last one
                    left_hash 
                };

                let mut hasher = Sha256::new();
                hasher.update(left_hash);
                hasher.update(right_hash);
                current_level.push(hasher.finalize().into());
                i += 2;
            }
            nodes.push(current_level);
        }
        Ok(MerkleTree { nodes })
    }

    /// Returns the root hash of the Merkle tree.
    ///
    /// Returns `None` if the tree is somehow malformed (e.g. constructed improperly,
    /// though `new` should prevent this), or if there are no nodes.
    pub fn get_root(&self) -> Option<MerkleRoot> {
        self.nodes
            .last()
            .and_then(|root_level| root_level.first())
            .cloned()
    }

    /// Generates a Merkle proof for the leaf at the given index.
    ///
    /// # Arguments
    ///
    /// * `leaf_index` - The index of the leaf for which to generate the proof.
    ///
    /// # Returns
    ///
    /// An `Option<MerkleProof>`. Returns `None` if the `leaf_index` is out of bounds.
    /// Returns an empty proof if the tree has only one leaf.
    pub fn get_proof(&self, leaf_index: usize) -> Option<MerkleProof> {
        if self.nodes.is_empty() || self.nodes[0].len() <= leaf_index {
            return None; // Index out of bounds or no leaves in the first level
        }

        // A tree with a single leaf (root is the leaf itself) has an empty proof.
        if self.nodes.len() == 1 {
            return Some(Vec::new());
        }

        let mut proof = MerkleProof::new();
        let mut current_node_index_in_level = leaf_index;

        // Iterate from the leaf level (level 0) up to the level just below the root
        for level_idx in 0..(self.nodes.len() - 1) {
            let current_level_nodes = &self.nodes[level_idx];
            let sibling_hash;
            let direction;

            if current_node_index_in_level % 2 == 0 { // Current node is a left child
                direction = ProofDirection::Right;
                // Sibling is to the right
                if current_node_index_in_level + 1 < current_level_nodes.len() {
                    sibling_hash = current_level_nodes[current_node_index_in_level + 1];
                } else {
                    // This is the last node in an odd-sized level; it was hashed with itself.
                    // The proof component is its own hash, acting as the right sibling.
                    sibling_hash = current_level_nodes[current_node_index_in_level];
                }
            } else { // Current node is a right child
                direction = ProofDirection::Left;
                // Sibling is to the left. This index must be valid.
                sibling_hash = current_level_nodes[current_node_index_in_level - 1];
            }
            proof.push((sibling_hash, direction));
            current_node_index_in_level /= 2; // Move to the parent's index in the next level up
        }
        Some(proof)
    }
}

/// Verifies a Merkle proof for a given leaf hash against an expected root.
///
/// # Arguments
///
/// * `leaf_hash` - The hash of the leaf to verify.
/// * `proof` - The Merkle proof for the `leaf_hash`.
/// * `expected_root` - The expected Merkle root.
///
/// # Returns
///
/// `true` if the proof is valid and the calculated root matches `expected_root`, `false` otherwise.
pub fn verify_merkle_proof(
    mut current_hash: [u8; 32],
    proof: &MerkleProof,
    expected_root: MerkleRoot,
) -> bool {
    for (sibling_hash, direction) in proof {
        let mut hasher = Sha256::new();
        match direction {
            ProofDirection::Left => {
                hasher.update(sibling_hash);
                hasher.update(current_hash);
            }
            ProofDirection::Right => {
                hasher.update(current_hash);
                hasher.update(sibling_hash);
            }
        }
        current_hash = hasher.finalize().into();
    }
    current_hash == expected_root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_hash(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().into()
    }

    #[test]
    fn test_merkle_tree_empty_leaves() {
        let empty_leaves: Vec<[u8; 32]> = Vec::new();
        assert!(MerkleTree::new(empty_leaves).is_err());
    }

    #[test]
    fn test_merkle_tree_single_leaf() {
        let leaf1 = create_hash(b"leaf1");
        let tree = MerkleTree::new(vec![leaf1]).unwrap();
        assert_eq!(tree.get_root(), Some(leaf1));
        assert_eq!(tree.nodes.len(), 1);
        assert_eq!(tree.nodes[0].len(), 1);
    }

    #[test]
    fn test_merkle_tree_two_leaves() {
        let leaf1 = create_hash(b"leaf1");
        let leaf2 = create_hash(b"leaf2");
        
        let mut expected_root_hasher = Sha256::new();
        expected_root_hasher.update(leaf1);
        expected_root_hasher.update(leaf2);
        let expected_root: [u8; 32] = expected_root_hasher.finalize().into();

        let tree = MerkleTree::new(vec![leaf1, leaf2]).unwrap();
        assert_eq!(tree.get_root(), Some(expected_root));
        assert_eq!(tree.nodes.len(), 2); // leaves, root
        assert_eq!(tree.nodes[0].len(), 2); // leaf level
        assert_eq!(tree.nodes[1].len(), 1); // root level
    }

    #[test]
    fn test_merkle_tree_three_leaves() {
        let leaf1 = create_hash(b"leaf1");
        let leaf2 = create_hash(b"leaf2");
        let leaf3 = create_hash(b"leaf3");

        // Level 0: [h1, h2, h3]
        // Level 1: [h(h1,h2), h(h3,h3)]
        // Level 2: [h(h(h1,h2), h(h3,h3))]

        let mut h12_hasher = Sha256::new();
        h12_hasher.update(leaf1);
        h12_hasher.update(leaf2);
        let h12: [u8; 32] = h12_hasher.finalize().into();

        let mut h33_hasher = Sha256::new();
        h33_hasher.update(leaf3);
        h33_hasher.update(leaf3); // Paired with itself
        let h33: [u8; 32] = h33_hasher.finalize().into();

        let mut expected_root_hasher = Sha256::new();
        expected_root_hasher.update(h12);
        expected_root_hasher.update(h33);
        let expected_root: [u8; 32] = expected_root_hasher.finalize().into();
        
        let tree = MerkleTree::new(vec![leaf1, leaf2, leaf3]).unwrap();
        assert_eq!(tree.get_root(), Some(expected_root));
        assert_eq!(tree.nodes.len(), 3); // leaves, intermediate, root
        assert_eq!(tree.nodes[0].len(), 3); // leaf level
        assert_eq!(tree.nodes[1].len(), 2); // intermediate level
        assert_eq!(tree.nodes[2].len(), 1); // root level
    }

    #[test]
    fn test_merkle_tree_four_leaves() {
        let leaf1 = create_hash(b"leaf1");
        let leaf2 = create_hash(b"leaf2");
        let leaf3 = create_hash(b"leaf3");
        let leaf4 = create_hash(b"leaf4");

        // Level 0: [h1, h2, h3, h4]
        // Level 1: [h(h1,h2), h(h3,h4)]
        // Level 2: [h(h(h1,h2), h(h3,h4))]

        let h12: [u8; 32] = { let mut h = Sha256::new(); h.update(leaf1); h.update(leaf2); h.finalize().into() };
        let h34: [u8; 32] = { let mut h = Sha256::new(); h.update(leaf3); h.update(leaf4); h.finalize().into() };
        let expected_root: [u8; 32] = { let mut h = Sha256::new(); h.update(h12); h.update(h34); h.finalize().into() };
        
        let tree = MerkleTree::new(vec![leaf1, leaf2, leaf3, leaf4]).unwrap();
        assert_eq!(tree.get_root(), Some(expected_root));
        assert_eq!(tree.nodes.len(), 3);
        assert_eq!(tree.nodes[0].len(), 4);
        assert_eq!(tree.nodes[1].len(), 2);
        assert_eq!(tree.nodes[2].len(), 1);
    }

    // Helper to combine two hashes for test readability
    fn combine_hashes(h1: [u8;32], h2: [u8;32]) -> [u8;32] {
        let mut hasher = Sha256::new();
        hasher.update(h1);
        hasher.update(h2);
        hasher.finalize().into()
    }

    #[test]
    fn test_merkle_proof_single_leaf() {
        let leaf1 = create_hash(b"leaf1");
        let tree = MerkleTree::new(vec![leaf1]).unwrap();
        let proof = tree.get_proof(0).unwrap();
        assert!(proof.is_empty());
        assert!(verify_merkle_proof(leaf1, &proof, tree.get_root().unwrap()));
    }

    #[test]
    fn test_merkle_proof_two_leaves() {
        let leaf1 = create_hash(b"leaf1");
        let leaf2 = create_hash(b"leaf2");
        let tree = MerkleTree::new(vec![leaf1, leaf2]).unwrap();
        let root = tree.get_root().unwrap();

        let proof0 = tree.get_proof(0).unwrap();
        assert_eq!(proof0.len(), 1);
        assert_eq!(proof0[0], (leaf2, ProofDirection::Right));
        assert!(verify_merkle_proof(leaf1, &proof0, root));

        let proof1 = tree.get_proof(1).unwrap();
        assert_eq!(proof1.len(), 1);
        assert_eq!(proof1[0], (leaf1, ProofDirection::Left));
        assert!(verify_merkle_proof(leaf2, &proof1, root));
    }

    #[test]
    fn test_merkle_proof_three_leaves() {
        let leaf1 = create_hash(b"leaf1");
        let leaf2 = create_hash(b"leaf2");
        let leaf3 = create_hash(b"leaf3");
        let tree = MerkleTree::new(vec![leaf1, leaf2, leaf3]).unwrap();
        let root = tree.get_root().unwrap();

        let h12 = combine_hashes(leaf1, leaf2);
        let h33 = combine_hashes(leaf3, leaf3);

        let proof0 = tree.get_proof(0).unwrap();
        assert_eq!(proof0.len(), 2);
        assert_eq!(proof0[0], (leaf2, ProofDirection::Right));
        assert_eq!(proof0[1], (h33, ProofDirection::Right));
        assert!(verify_merkle_proof(leaf1, &proof0, root));

        let proof1 = tree.get_proof(1).unwrap();
        assert_eq!(proof1.len(), 2);
        assert_eq!(proof1[0], (leaf1, ProofDirection::Left));
        assert_eq!(proof1[1], (h33, ProofDirection::Right));
        assert!(verify_merkle_proof(leaf2, &proof1, root));

        let proof2 = tree.get_proof(2).unwrap();
        assert_eq!(proof2.len(), 2);
        assert_eq!(proof2[0], (leaf3, ProofDirection::Right)); // Sibling is itself
        assert_eq!(proof2[1], (h12, ProofDirection::Left));
        assert!(verify_merkle_proof(leaf3, &proof2, root));
    }

    #[test]
    fn test_merkle_proof_four_leaves() {
        let leaf1 = create_hash(b"leaf1");
        let leaf2 = create_hash(b"leaf2");
        let leaf3 = create_hash(b"leaf3");
        let leaf4 = create_hash(b"leaf4");

        // Level 0: [h1, h2, h3, h4]
        // Level 1: [h(h1,h2), h(h3,h4)]
        // Level 2: [h(h(h1,h2), h(h3,h4))]

        let h12: [u8; 32] = { let mut h = Sha256::new(); h.update(leaf1); h.update(leaf2); h.finalize().into() };
        let h34: [u8; 32] = { let mut h = Sha256::new(); h.update(leaf3); h.update(leaf4); h.finalize().into() };
        let expected_root: [u8; 32] = { let mut h = Sha256::new(); h.update(h12); h.update(h34); h.finalize().into() };
        
        let tree = MerkleTree::new(vec![leaf1, leaf2, leaf3, leaf4]).unwrap();
        assert_eq!(tree.get_root(), Some(expected_root));
        assert_eq!(tree.nodes.len(), 3);
        assert_eq!(tree.nodes[0].len(), 4);
        assert_eq!(tree.nodes[1].len(), 2);
        assert_eq!(tree.nodes[2].len(), 1);

        // Proof tests for four leaves
        let h12_val: [u8; 32] = { let mut h = Sha256::new(); h.update(leaf1); h.update(leaf2); h.finalize().into() };
        let h34_val: [u8; 32] = { let mut h = Sha256::new(); h.update(leaf3); h.update(leaf4); h.finalize().into() };

        let proof0 = tree.get_proof(0).unwrap();
        assert_eq!(proof0.len(), 2);
        assert_eq!(proof0[0], (leaf2, ProofDirection::Right));
        assert_eq!(proof0[1], (h34_val, ProofDirection::Right));
        assert!(verify_merkle_proof(leaf1, &proof0, expected_root));

        let proof3 = tree.get_proof(3).unwrap();
        assert_eq!(proof3.len(), 2);
        assert_eq!(proof3[0], (leaf3, ProofDirection::Left));
        assert_eq!(proof3[1], (h12_val, ProofDirection::Left));
        assert!(verify_merkle_proof(leaf4, &proof3, expected_root));
    }

    #[test]
    fn test_merkle_proof_invalid_index() {
        let leaf1 = create_hash(b"leaf1");
        let tree = MerkleTree::new(vec![leaf1]).unwrap();
        assert!(tree.get_proof(1).is_none());

        let leaf2 = create_hash(b"leaf2");
        let tree2 = MerkleTree::new(vec![leaf1, leaf2]).unwrap();
        assert!(tree2.get_proof(2).is_none());
    }

    #[test]
    fn test_merkle_proof_five_leaves() {
        let leaves: Vec<[u8;32]> = (0..5).map(|i| create_hash(format!("leaf{}", i).as_bytes())).collect();
        let tree = MerkleTree::new(leaves.clone()).unwrap();
        let root = tree.get_root().unwrap();

        let h01 = combine_hashes(leaves[0], leaves[1]);
        let h23 = combine_hashes(leaves[2], leaves[3]);
        let h44 = combine_hashes(leaves[4], leaves[4]); 

        let h0123 = combine_hashes(h01, h23);
        let h4444 = combine_hashes(h44, h44); 

        // Proof for leaves[0]
        let proof0 = tree.get_proof(0).unwrap();
        assert_eq!(proof0.len(), 3);
        assert_eq!(proof0[0], (leaves[1], ProofDirection::Right));
        assert_eq!(proof0[1], (h23, ProofDirection::Right));
        assert_eq!(proof0[2], (h4444, ProofDirection::Right));
        assert!(verify_merkle_proof(leaves[0], &proof0, root));

        // Proof for leaves[2]
        let proof2 = tree.get_proof(2).unwrap();
        assert_eq!(proof2.len(), 3);
        assert_eq!(proof2[0], (leaves[3], ProofDirection::Right)); 
        assert_eq!(proof2[1], (h01, ProofDirection::Left));       
        assert_eq!(proof2[2], (h4444, ProofDirection::Right));    
        assert!(verify_merkle_proof(leaves[2], &proof2, root));

        // Proof for leaves[4]
        let proof4 = tree.get_proof(4).unwrap();
        assert_eq!(proof4.len(), 3);
        assert_eq!(proof4[0], (leaves[4], ProofDirection::Right)); 
        assert_eq!(proof4[1], (h44, ProofDirection::Right)); 
        assert_eq!(proof4[2], (h0123, ProofDirection::Left));   
        assert!(verify_merkle_proof(leaves[4], &proof4, root));
    }
}
