#[cfg(test)]
mod tests {
    use super::*;
    use crate::acctrie::acc::DynamicAccumulator;
    use crate::acctrie::trie::{AccTrie, Key, Value};
    use std::collections::HashSet;

    #[test]
    fn test_dynamic_accumulator_batch_ops() {
        let mut acc = DynamicAccumulator::new();
        let elements: Vec<i64> = vec![1, 2, 3, 4, 5];

        // Test Batch Add
        acc.add_batch(&elements).expect("Batch add failed");

        // Verify membership for all elements
        for elem in &elements {
            let proof = acc.prove_membership(elem).expect("Proof generation failed");
            assert!(proof.verify(acc.acc_value), "Membership verification failed for {}", elem);
        }

        // Test Batch Delete
        let to_delete = vec![1, 2];
        acc.delete_batch(&to_delete).expect("Batch delete failed");

        // Verify deletion
        for elem in &to_delete {
            assert!(acc.prove_membership(elem).is_err(), "Element {} should be deleted", elem);
        }
        
        // Verify remaining elements
        for elem in vec![3, 4, 5] {
            let proof = acc.prove_membership(&elem).expect("Proof generation failed");
            assert!(proof.verify(acc.acc_value), "Membership verification failed for {}", elem);
        }
    }

    #[test]
    fn test_acctrie_insert_batch() {
        let mut trie = AccTrie::new();
        
        // Prepare batch data: multiple values for same key, and multiple keys
        let key1 = vec![1, 2, 3];
        let key2 = vec![4, 5, 6];
        
        let kvs = vec![
            (key1.clone(), 100),
            (key1.clone(), 101),
            (key1.clone(), 102),
            (key2.clone(), 200),
            (key2.clone(), 201),
        ];

        // Execute batch insert
        trie.insert_batch(kvs).expect("Batch insert failed");

        // Verify Key1
        let values1 = trie.get_values(&key1).expect("Key1 not found");
        assert!(values1.contains(&100));
        assert!(values1.contains(&101));
        assert!(values1.contains(&102));
        assert_eq!(values1.len(), 3);

        // Verify Key2
        let values2 = trie.get_values(&key2).expect("Key2 not found");
        assert!(values2.contains(&200));
        assert!(values2.contains(&201));
        assert_eq!(values2.len(), 2);
    }
}
