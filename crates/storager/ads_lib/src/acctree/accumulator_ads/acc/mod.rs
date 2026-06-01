pub mod dynamic_accumulator;
pub mod proofs;
pub mod serde_impl;
pub mod setup;
pub mod utils;

pub use ark_bls12_381::{
    Bls12_381 as Curve, Fq12, Fr, G1Affine, G1Projective, G2Affine, G2Projective,
};

// Re-export main components
pub use utils::{digest_set_from_set, expand_to_poly};
pub use dynamic_accumulator::{DynamicAccumulator, QueryResult};
pub use proofs::*;
pub use setup::{E_G_G, PublicParameters, init_public_parameters, init_public_parameters_direct, 
                get_public_parameters, get_g1s, get_g2s, get_g1s_vec, get_g2s_vec};

/// Unit tests for basic accumulator operations
/// 
/// These tests verify the fundamental cryptographic operations (add, delete, membership)
/// at the module level. They stay in this file because they test internal module behavior.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::acc::utils::digest_set_from_set;
    use crate::set::Set;

    fn init_logger() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_add_delete_flow() {
        init_logger();
        let mut acc = DynamicAccumulator::default();

        // Create a test element
        let set_elem = Set::from_vec(vec![42]);
        let ds = digest_set_from_set(&set_elem);
        let elem = ds[0]; // Get the Fr element

        // 1. Prove Add
        // Acc: {} -> {42}
        let add_proof = AddProof::new(&mut acc, elem).expect("AddProof generation failed");
        assert!(add_proof.verify(), "AddProof verification failed");

        // 2. Prove Membership (post-add)
        let mem_proof = MembershipProof::new(&acc, elem).expect("Membership generation failed");
        assert!(
            mem_proof.verify(acc.acc_value),
            "Membership verification failed"
        );

        // 3. Prove Delete
        // Acc: {42} -> {}
        let del_proof = DeleteProof::new(&mut acc, elem).expect("DeleteProof generation failed");
        assert!(del_proof.verify(), "DeleteProof verification failed");

        // 4. Prove Non-Membership (post-delete)
        // Note: For non-membership, we need the SET that represents the accumulator state.
        // After clean delete, acc is empty set.
        let empty_set = digest_set_from_set(&Set::<i32>::from_vec(vec![]));
        let non_mem_proof =
            NonMembershipProof::new(elem, &empty_set).expect("NonMembership failed");
        assert!(
            non_mem_proof.verify(acc.acc_value),
            "NonMembership verification failed"
        );
    }

    #[test]
    fn test_disjointness_proof() {
        init_logger();
        let set1 = digest_set_from_set(&Set::from_vec(vec![1, 2, 3]));
        let set2 = digest_set_from_set(&Set::from_vec(vec![4, 5, 6]));
        let set3 = digest_set_from_set(&Set::from_vec(vec![1]));

        // Test disjoint sets (Success)
        let proof =
            DisjointnessProof::new(&set1, &set2).expect("Disjointness Proof generation failed");
        let acc1 = DynamicAccumulator::calculate_commitment(&set1);
        let acc2 = DynamicAccumulator::calculate_commitment(&set2);
        assert!(
            proof.verify(&acc1, &acc2),
            "Disjointness Verification failed"
        );

        // Test non-disjoint sets (Failure)
        assert!(
            DisjointnessProof::new(&set1, &set3).is_err(),
            "Sets should NOT be disjoint"
        );
    }

    #[test]
    fn test_intersection_and_union() {
        init_logger();
        let s1 = digest_set_from_set(&Set::from_vec(vec![1, 2, 3, 4]));
        let s2 = digest_set_from_set(&Set::from_vec(vec![3, 4, 5, 6]));
        let s_inter = digest_set_from_set(&Set::from_vec(vec![3, 4]));
        let s_union = digest_set_from_set(&Set::from_vec(vec![1, 2, 3, 4, 5, 6]));

        let acc1_val = DynamicAccumulator::calculate_commitment(&s1);
        let acc2_val = DynamicAccumulator::calculate_commitment(&s2);
        let inter_val = DynamicAccumulator::calculate_commitment(&s_inter);
        let union_val = DynamicAccumulator::calculate_commitment(&s_union);

        // Intersection
        let (inter_acc, inter_proof) =
            IntersectionProof::new(&s1, &s2, &s_inter).expect("Intersection failed");
        assert_eq!(
            inter_acc.acc_value, inter_val,
            "Intersection acc value mismatch"
        );
        assert!(
            inter_proof.verify(acc1_val, acc2_val, inter_acc.acc_value),
            "Intersection verify failed"
        );

        // Union
        let (union_acc, union_proof) =
            UnionProof::new(&inter_acc, inter_proof, &s_union).expect("Union failed");
        assert_eq!(union_acc.acc_value, union_val, "Union acc value mismatch");
        assert!(
            union_proof.verify(acc1_val, acc2_val, union_acc.acc_value),
            "Union verify failed"
        );
    }
}
