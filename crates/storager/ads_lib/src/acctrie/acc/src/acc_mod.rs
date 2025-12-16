use crate::digest_set;
use crate::serde_impl;
use crate::utils;

pub use ark_bls12_381::{
    Bls12_381 as Curve, Fq12, Fr, G1Affine, G1Projective, G2Affine, G2Projective,
};
pub type DigestSet = digest_set::DigestSet<Fr>;

use crate::digest::{Digest, Digestible};
use crate::set::{MultiSet, SetElement};
use anyhow::{self, bail};
use ark_ec::{msm::VariableBaseMSM, AffineCurve, PairingEngine, ProjectiveCurve};
use ark_ff::{PrimeField, ToBytes, Zero};
use core::any::Any;
use core::str::FromStr;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use crate::utils::{digest_to_prime_field, FixedBaseCurvePow, FixedBaseScalarPow};

lazy_static! {
    // 250 bits
    static ref PUB_Q: Fr = Fr::from_str("480721077433357505777975950918924200361380912084288598463024400624539293706").unwrap();
    // 128 bits
    static ref PRI_S: Fr = Fr::from_str("259535143263514268207918833918737523409").unwrap();
    static ref G1_POWER: FixedBaseCurvePow<G1Projective> =
        FixedBaseCurvePow::build(&G1Projective::prime_subgroup_generator());
    static ref G2_POWER: FixedBaseCurvePow<G2Projective> =
        FixedBaseCurvePow::build(&G2Projective::prime_subgroup_generator());
    static ref PRI_S_POWER: FixedBaseScalarPow<Fr> = FixedBaseScalarPow::build(&PRI_S);
    static ref E_G_G: Fq12 = Curve::pairing(
        G1Affine::prime_subgroup_generator(),
        G2Affine::prime_subgroup_generator()
    );
}

fn get_g1s(coeff: Fr) -> G1Affine {
    let si = PRI_S_POWER.apply(&coeff);
    G1_POWER.apply(&si).into_affine()
}

fn get_g2s(coeff: Fr) -> G2Affine {
    let si = PRI_S_POWER.apply(&coeff);
    G2_POWER.apply(&si).into_affine()
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Type {
    ACC,
}

pub trait Accumulator {
    const TYPE: Type;
    type Proof;

    fn cal_acc_g1_sk<T: SetElement>(set: &MultiSet<T>) -> G1Affine {
        Self::cal_acc_g1_sk_d(&DigestSet::new(set))
    }
    fn cal_acc_g1<T: SetElement>(set: &MultiSet<T>) -> G1Affine {
        Self::cal_acc_g1_d(&DigestSet::new(set))
    }
    fn cal_acc_g2_sk<T: SetElement>(set: &MultiSet<T>) -> G2Affine {
        Self::cal_acc_g2_sk_d(&DigestSet::new(set))
    }
    fn cal_acc_g2<T: SetElement>(set: &MultiSet<T>) -> G2Affine {
        Self::cal_acc_g2_d(&DigestSet::new(set))
    }
    fn cal_acc_g1_sk_d(set: &DigestSet) -> G1Affine;
    fn cal_acc_g1_d(set: &DigestSet) -> G1Affine;
    fn cal_acc_g2_sk_d(set: &DigestSet) -> G2Affine;
    fn cal_acc_g2_d(set: &DigestSet) -> G2Affine;
    fn gen_proof(set1: &DigestSet, set2: &DigestSet) -> anyhow::Result<Self::Proof>;
}

pub trait AccumulatorProof: Eq + PartialEq {
    const TYPE: Type;

    fn gen_proof(set1: &DigestSet, set2: &DigestSet) -> anyhow::Result<Self>
    where
        Self: core::marker::Sized;

    fn combine_proof(&mut self, other: &Self) -> anyhow::Result<()>;

    fn as_any(&self) -> &dyn Any;
}

pub struct Acc;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AccProof {
    #[serde(with = "serde_impl")]
    f: G1Affine,
}

impl AccumulatorProof for AccProof {
    const TYPE: Type = Type::ACC;

    fn gen_proof(set1: &DigestSet, set2: &DigestSet) -> anyhow::Result<Self> {
        Acc::gen_proof(set1, set2)
    }

    fn combine_proof(&mut self, other: &Self) -> anyhow::Result<()> {
        let mut f = self.f.into_projective();
        f.add_assign_mixed(&other.f);
        self.f = f.into_affine();
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl AccProof {
    pub fn verify(&self, acc1: &G1Affine, acc2: &G2Affine) -> bool {
        let a = Curve::pairing(*acc1, *acc2);
        let b = Curve::pairing(self.f, G2Affine::prime_subgroup_generator());
        a == b
    }
}

impl Accumulator for Acc {
    const TYPE: Type = Type::ACC;
    type Proof = AccProof;

    fn cal_acc_g1_sk_d(set: &DigestSet) -> G1Affine {
        let x = set
            .par_iter()
            .map(|(a, b)| {
                let s = PRI_S_POWER.apply(a);
                s * &Fr::from(*b)
            })
            .reduce(Fr::zero, |a, b| a + &b);
        G1_POWER.apply(&x).into_affine()
    }
    fn cal_acc_g1_d(set: &DigestSet) -> G1Affine {
        let mut bases: Vec<G1Affine> = Vec::with_capacity(set.len());
        let mut scalars: Vec<<Fr as PrimeField>::BigInt> = Vec::with_capacity(set.len());
        (0..set.len())
            .into_par_iter()
            .map(|i| get_g1s(set[i].0))
            .collect_into_vec(&mut bases);
        (0..set.len())
            .into_par_iter()
            .map(|i| <Fr as PrimeField>::BigInt::from(set[i].1 as u64))
            .collect_into_vec(&mut scalars);
        VariableBaseMSM::multi_scalar_mul(&bases[..], &scalars[..]).into_affine()
    }
    fn cal_acc_g2_sk_d(set: &DigestSet) -> G2Affine {
        let x = set
            .par_iter()
            .map(|(a, b)| {
                let s = PRI_S_POWER.apply(&(*PUB_Q - a));
                s * &Fr::from(*b)
            })
            .reduce(Fr::zero, |a, b| a + &b);
        G2_POWER.apply(&x).into_affine()
    }
    fn cal_acc_g2_d(set: &DigestSet) -> G2Affine {
        let mut bases: Vec<G2Affine> = Vec::with_capacity(set.len());
        let mut scalars: Vec<<Fr as PrimeField>::BigInt> = Vec::with_capacity(set.len());
        (0..set.len())
            .into_par_iter()
            .map(|i| get_g2s(*PUB_Q - &set[i].0))
            .collect_into_vec(&mut bases);
        (0..set.len())
            .into_par_iter()
            .map(|i| <Fr as PrimeField>::BigInt::from(set[i].1 as u64))
            .collect_into_vec(&mut scalars);
        VariableBaseMSM::multi_scalar_mul(&bases[..], &scalars[..]).into_affine()
    }
    fn gen_proof(set1: &DigestSet, set2: &DigestSet) -> anyhow::Result<Self::Proof> {
        let produce_size = set1.len() * set2.len();
        let mut product: Vec<(Fr, u64)> = Vec::with_capacity(produce_size);
        (0..produce_size)
            .into_par_iter()
            .map(|i| {
                let set1idx = i / set2.len();
                let set2idx = i % set2.len();
                let (s1, q1) = set1[set1idx];
                let (s2, q2) = set2[set2idx];
                (*PUB_Q + &s1 - &s2, (q1 * q2) as u64)
            })
            .collect_into_vec(&mut product);
        if product.par_iter().any(|(x, _)| *x == *PUB_Q) {
            bail!("cannot generate proof");
        }

        let mut bases: Vec<G1Affine> = Vec::with_capacity(produce_size);
        let mut scalars: Vec<<Fr as PrimeField>::BigInt> = Vec::with_capacity(produce_size);
        (0..produce_size)
            .into_par_iter()
            .map(|i| get_g1s(product[i].0))
            .collect_into_vec(&mut bases);
        (0..produce_size)
            .into_par_iter()
            .map(|i| <Fr as PrimeField>::BigInt::from(product[i].1))
            .collect_into_vec(&mut scalars);
        let f = VariableBaseMSM::multi_scalar_mul(&bases[..], &scalars[..]).into_affine();
        Ok(AccProof { f })
    }
}

impl Acc {
    // New methods for dynamic operations with proofs
    pub fn add_element(acc: &G1Affine, element: &impl Digestible) -> (G1Affine, G1Affine) {
        let v: Fr = digest_to_prime_field(&element.to_digest());
        let delta = get_g1s(v);
        let mut acc_proj = acc.into_projective();
        acc_proj.add_assign_mixed(&delta);
        let new_acc = acc_proj.into_affine();
        // Proof is the old accumulator (which is the witness for the new element in the new accumulator)
        (new_acc, *acc)
    }

    pub fn remove_element(acc: &G1Affine, element: &impl Digestible) -> (G1Affine, G1Affine) {
        let v: Fr = digest_to_prime_field(&element.to_digest());
        let delta = get_g1s(v);
        let mut acc_proj = acc.into_projective();
        acc_proj.add_assign_mixed(&-delta);
        let new_acc = acc_proj.into_affine();
        // Proof is the new accumulator (which is the witness for the removed element in the old accumulator)
        (new_acc, new_acc)
    }

    pub fn update_element(
        acc: &G1Affine,
        old_element: &impl Digestible,
        new_element: &impl Digestible,
    ) -> (G1Affine, G1Affine) {
        let v_old: Fr = digest_to_prime_field(&old_element.to_digest());
        let v_new: Fr = digest_to_prime_field(&new_element.to_digest());
        let delta_old = get_g1s(v_old);
        let delta_new = get_g1s(v_new);

        let mut acc_proj = acc.into_projective();
        acc_proj.add_assign_mixed(&-delta_old);
        let mid_acc = acc_proj.into_affine(); // Intermediate state: old removed, new not added
        acc_proj.add_assign_mixed(&delta_new);
        let new_acc = acc_proj.into_affine();

        // Proof is the intermediate accumulator
        (new_acc, mid_acc)
    }

    pub fn verify_add(
        old_acc: &G1Affine,
        new_acc: &G1Affine,
        proof: &G1Affine,
        element: &impl Digestible,
    ) -> bool {
        // 1. Proof should be the old accumulator
        if proof != old_acc {
            return false;
        }
        // 2. Verify transition: Old + Element == New
        let v: Fr = digest_to_prime_field(&element.to_digest());
        let delta = get_g1s(v);
        let mut proof_proj = proof.into_projective();
        proof_proj.add_assign_mixed(&delta);
        proof_proj.into_affine() == *new_acc
    }

    pub fn verify_remove(
        old_acc: &G1Affine,
        new_acc: &G1Affine,
        proof: &G1Affine,
        element: &impl Digestible,
    ) -> bool {
        // 1. Proof should be the new accumulator
        if proof != new_acc {
            return false;
        }
        // 2. Verify transition: New + Element == Old
        let v: Fr = digest_to_prime_field(&element.to_digest());
        let delta = get_g1s(v);
        let mut proof_proj = proof.into_projective();
        proof_proj.add_assign_mixed(&delta);
        proof_proj.into_affine() == *old_acc
    }

    pub fn verify_update(
        old_acc: &G1Affine,
        new_acc: &G1Affine,
        proof: &G1Affine,
        old_element: &impl Digestible,
        new_element: &impl Digestible,
    ) -> bool {
        // Proof is the intermediate state (Old - OldElement)

        // 1. Verify removal of old element: Proof + OldElement == OldAcc
        let v_old: Fr = digest_to_prime_field(&old_element.to_digest());
        let delta_old = get_g1s(v_old);
        let mut proof_proj = proof.into_projective();
        proof_proj.add_assign_mixed(&delta_old);
        if proof_proj.into_affine() != *old_acc {
            return false;
        }

        // 2. Verify addition of new element: Proof + NewElement == NewAcc
        let v_new: Fr = digest_to_prime_field(&new_element.to_digest());
        let delta_new = get_g1s(v_new);
        let mut proof_proj = proof.into_projective();
        proof_proj.add_assign_mixed(&delta_new);
        proof_proj.into_affine() == *new_acc
    }

    pub fn create_witness(acc: &G1Affine, element: &impl Digestible) -> G1Affine {
        Self::remove_element(acc, element).0
    }

    pub fn verify_membership(
        acc: &G1Affine,
        witness: &G1Affine,
        element: &impl Digestible,
    ) -> bool {
        let v: Fr = digest_to_prime_field(&element.to_digest());
        let delta = get_g1s(v);
        let mut witness_proj = witness.into_projective();
        witness_proj.add_assign_mixed(&delta);
        witness_proj.into_affine() == *acc
    }

    // Helper to get the public parameter g^{s^v} for a specific element
    // This is useful for the Manager to send to the Client for verification
    pub fn get_element_commitment(element: &impl Digestible) -> G1Affine {
        let v: Fr = digest_to_prime_field(&element.to_digest());
        get_g1s(v)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum Proof {
    ACC(Box<AccProof>),
}

impl Digestible for G1Affine {
    fn to_digest(&self) -> Digest {
        let mut buf = Vec::<u8>::new();
        self.write(&mut buf)
            .unwrap_or_else(|_| panic!("failed to serialize {:?}", self));
        buf.to_digest()
    }
}

impl Digestible for Fr {
    fn to_digest(&self) -> Digest {
        let mut buf = Vec::<u8>::new();
        self.write(&mut buf)
            .unwrap_or_else(|_| panic!("failed to serialize Fr"));
        buf.to_digest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_logger() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_cal_acc() {
        init_logger();
        let set = MultiSet::from_vec(vec![1, 1, 2, 3, 4, 4, 5, 6, 6, 7, 8, 9]);
        assert_eq!(Acc::cal_acc_g1(&set), Acc::cal_acc_g1_sk(&set));
        assert_eq!(Acc::cal_acc_g2(&set), Acc::cal_acc_g2_sk(&set));
    }

    #[test]
    fn test_acc_proof() {
        init_logger();
        let set1 = DigestSet::new(&MultiSet::from_vec(vec![1, 2, 3]));
        let set2 = DigestSet::new(&MultiSet::from_vec(vec![4, 5, 6]));
        let set3 = DigestSet::new(&MultiSet::from_vec(vec![1, 1]));
        let proof = Acc::gen_proof(&set1, &set2).unwrap();
        let acc1 = Acc::cal_acc_g1_sk_d(&set1);
        let acc2 = Acc::cal_acc_g2_sk_d(&set2);
        assert!(proof.verify(&acc1, &acc2));
        assert!(Acc::gen_proof(&set1, &set3).is_err());
    }

    #[test]
    fn test_acc_proof_sum() {
        init_logger();
        let set1 = DigestSet::new(&MultiSet::from_vec(vec![1, 2, 3]));
        let set2 = DigestSet::new(&MultiSet::from_vec(vec![4, 5, 6]));
        let set3 = DigestSet::new(&MultiSet::from_vec(vec![7, 8, 9]));
        let mut proof1 = Acc::gen_proof(&set1, &set2).unwrap();
        let proof2 = Acc::gen_proof(&set1, &set3).unwrap();
        proof1.combine_proof(&proof2).unwrap();
        let acc1 = Acc::cal_acc_g1_sk_d(&set1);
        let acc2 = Acc::cal_acc_g2_sk_d(&set2);
        let acc3 = Acc::cal_acc_g2_sk_d(&set3);
        let acc4 = {
            let mut acc = acc2.into_projective();
            acc.add_assign_mixed(&acc3);
            acc.into_affine()
        };
        assert!(proof1.verify(&acc1, &acc4));
    }

    #[test]
    fn test_acc_dynamic_ops() {
        init_logger();
        let mut set_vec = vec![1, 2, 3];
        let set = MultiSet::from_vec(set_vec.clone());
        let mut acc = Acc::cal_acc_g1_sk(&set);

        // Test Add
        let new_elem = 4;
        let old_acc_for_add = acc;
        let (new_acc, proof_add) = Acc::add_element(&acc, &new_elem);
        acc = new_acc;

        set_vec.push(new_elem);
        let expected_set = MultiSet::from_vec(set_vec.clone());
        let expected_acc = Acc::cal_acc_g1_sk(&expected_set);
        assert_eq!(acc, expected_acc, "Add failed");
        assert!(
            Acc::verify_add(&old_acc_for_add, &acc, &proof_add, &new_elem),
            "Verify Add failed"
        );

        // Test Remove
        let remove_elem = 2;
        let old_acc_for_remove = acc;
        let (new_acc, proof_remove) = Acc::remove_element(&acc, &remove_elem);
        acc = new_acc;

        set_vec.retain(|&x| x != remove_elem);
        let expected_set = MultiSet::from_vec(set_vec.clone());
        let expected_acc = Acc::cal_acc_g1_sk(&expected_set);
        assert_eq!(acc, expected_acc, "Remove failed");
        assert!(
            Acc::verify_remove(&old_acc_for_remove, &acc, &proof_remove, &remove_elem),
            "Verify Remove failed"
        );

        // Test Update
        let old_elem = 3;
        let update_elem = 5;
        let old_acc_for_update = acc;
        let (new_acc, proof_update) = Acc::update_element(&acc, &old_elem, &update_elem);
        acc = new_acc;

        set_vec.retain(|&x| x != old_elem);
        set_vec.push(update_elem);
        let expected_set = MultiSet::from_vec(set_vec.clone());
        let expected_acc = Acc::cal_acc_g1_sk(&expected_set);
        assert_eq!(acc, expected_acc, "Update failed");
        assert!(
            Acc::verify_update(
                &old_acc_for_update,
                &acc,
                &proof_update,
                &old_elem,
                &update_elem
            ),
            "Verify Update failed"
        );

        // Test Query (Witness)
        let target = 5;
        let witness = Acc::create_witness(&acc, &target);
        assert!(
            Acc::verify_membership(&acc, &witness, &target),
            "Verify failed"
        );

        let non_member = 999;
        assert!(
            !Acc::verify_membership(&acc, &witness, &non_member),
            "Verify false positive"
        );
    }
}
