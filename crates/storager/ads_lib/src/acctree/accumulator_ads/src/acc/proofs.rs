use anyhow::Result;
use ark_bls12_381::{Bls12_381 as Curve, Fr, G1Affine, G2Affine};
use ark_ec::{AffineCurve, PairingEngine};
use serde::{Deserialize, Serialize};

use crate::acc::dynamic_accumulator::DynamicAccumulator;
use crate::acc::serde_impl;
use crate::acc::setup::{get_g1s, get_g2s, E_G_G};
use ark_ec::ProjectiveCurve;
use std::ops::Neg;

/// A proof that an 'add' operation was performed correctly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddProof {
    #[serde(with = "serde_impl")]
    pub old_acc_value: G1Affine,
    #[serde(with = "serde_impl")]
    pub new_acc_value: G1Affine,
    #[serde(with = "serde_impl")]
    pub element: Fr,
}

impl AddProof {
    /// Generates proof and updates accumulator.
    pub fn new(acc: &mut DynamicAccumulator, element: Fr) -> Result<Self> {
        let old_acc = acc.acc_value;
        let new_acc = acc.compute_add(element);
        acc.acc_value = new_acc;

        Ok(Self {
            old_acc_value: old_acc,
            new_acc_value: new_acc,
            element,
        })
    }

    /// Verifies that the new accumulator is the result of adding the element to the old one.
    /// Uses PUBLIC pairing verification: e(new_acc, g2) = e(old_acc, g2^(s-element))
    /// This is equivalent to: new_acc = old_acc^(s-element)
    ///
    /// SECURITY: Uses ONLY public parameters. No secret knowledge required.
    pub fn verify(&self) -> bool {
        let g2 = G2Affine::prime_subgroup_generator();
        let g2_s = get_g2s(1_usize); // g2^s from public parameters

        // Compute g2^(-element) = g2^{-element}
        let g2_neg_elem = g2.mul(self.element.neg()).into_affine();

        // Compute g2^(s-element) = g2^s * g2^{-element}
        let g2_s_minus_elem =
            (g2_s.into_projective() + g2_neg_elem.into_projective()).into_affine();

        let lhs = Curve::pairing(self.new_acc_value, g2);
        let rhs = Curve::pairing(self.old_acc_value, g2_s_minus_elem);

        lhs == rhs
    }
}

/// A proof that a 'delete' operation was performed correctly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteProof {
    #[serde(with = "serde_impl")]
    pub old_acc_value: G1Affine,
    #[serde(with = "serde_impl")]
    pub new_acc_value: G1Affine,
    #[serde(with = "serde_impl")]
    pub element: Fr,
}

impl DeleteProof {
    /// Generates proof and updates accumulator.
    pub fn new(acc: &mut DynamicAccumulator, element: Fr) -> Result<Self> {
        let old_acc = acc.acc_value;
        let new_acc = acc.compute_delete(element)?;
        acc.acc_value = new_acc;

        Ok(Self {
            old_acc_value: old_acc,
            new_acc_value: new_acc,
            element,
        })
    }

    /// Verifies that the new accumulator is the result of deleting the element from the old one.
    /// Uses PUBLIC pairing verification: e(new_acc, g2^(s-element)) = e(old_acc, g2)
    /// This is equivalent to: new_acc^(s-element) = old_acc
    ///
    /// SECURITY: Uses ONLY public parameters. No secret knowledge required.
    pub fn verify(&self) -> bool {
        let g2 = G2Affine::prime_subgroup_generator();
        let g2_s = get_g2s(1_usize); // g2^s from public parameters

        // Compute g2^(-element)
        let g2_neg_elem = g2.mul(self.element.neg()).into_affine();

        // Compute g2^(s-element) = g2^s * g2^{-element}
        let g2_s_minus_elem =
            (g2_s.into_projective() + g2_neg_elem.into_projective()).into_affine();

        let lhs = Curve::pairing(self.new_acc_value, g2_s_minus_elem);
        let rhs = Curve::pairing(self.old_acc_value, g2);

        lhs == rhs
    }
}

/// A proof of membership for an element in the accumulator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipProof {
    #[serde(with = "serde_impl")]
    pub witness: G1Affine,
    #[serde(with = "serde_impl")]
    pub element: Fr,
}

impl MembershipProof {
    pub fn new(acc: &DynamicAccumulator, element: Fr) -> Result<Self> {
        let witness = acc.compute_membership_witness(element)?;
        Ok(Self { witness, element })
    }

    /// Verifies that this proof is valid for the given accumulator value.
    /// Uses PUBLIC pairing verification: e(witness, g2^(s-element)) = e(accumulator, g2)
    /// This verifies that witness^(s-element) = accumulator, proving membership.
    ///
    /// SECURITY: Uses ONLY public parameters. No secret knowledge required.
    /// This enables PUBLIC VERIFIABILITY - anyone can verify membership.
    pub fn verify(&self, accumulator: G1Affine) -> bool {
        let g2 = G2Affine::prime_subgroup_generator();
        let g2_s = get_g2s(1_usize); // g2^s from public parameters

        // Compute g2^(-element)
        let g2_neg_elem = g2.mul(self.element.neg()).into_affine();

        // Compute g2^(s-element) = g2^s * g2^{-element}
        let g2_s_minus_elem =
            (g2_s.into_projective() + g2_neg_elem.into_projective()).into_affine();

        let lhs = Curve::pairing(self.witness, g2_s_minus_elem);
        let rhs = Curve::pairing(accumulator, g2);

        lhs == rhs
    }
}

/// A proof of non-membership for an element in the accumulator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NonMembershipProof {
    #[serde(with = "serde_impl")]
    pub element: Fr,
    /// Witness g2^B(s)
    #[serde(with = "serde_impl")]
    pub witness: G2Affine,
    /// Witness g2^A(s)
    #[serde(with = "serde_impl")]
    pub g2_a: G2Affine,
}

impl NonMembershipProof {
    pub fn new(element: Fr, elements: &[Fr]) -> Result<Self> {
        let (witness, g2_a) =
            DynamicAccumulator::compute_non_membership_witness(element, elements)?;
        Ok(Self {
            element,
            witness,
            g2_a,
        })
    }

    /// Verifies non-membership using Bezout's identity: A(s)*P(s) + B(s)*(s-x) = 1
    /// Check: e(Acc, g2^A) * e(g1^(s-x), g2^B) = e(g1, g2)
    ///
    /// SECURITY: Uses ONLY public parameters. No secret knowledge required.
    pub fn verify(&self, acc_value: G1Affine) -> bool {
        let g1 = G1Affine::prime_subgroup_generator();
        let g1_s = get_g1s(1_usize); // g1^s from public parameters

        // Compute g1^(-element)
        let g1_neg_elem = g1.mul(self.element.neg()).into_affine();

        // Compute g1^(s-element) = g1^s * g1^{-element}
        let g1_s_minus_elem =
            (g1_s.into_projective() + g1_neg_elem.into_projective()).into_affine();

        // Check: e(Acc, g2^A) * e(g1^(s-x), g2^B) = e(g1, g2)
        let lhs1 = Curve::pairing(acc_value, self.g2_a);
        let lhs2 = Curve::pairing(g1_s_minus_elem, self.witness);

        (lhs1 * lhs2) == *E_G_G
    }
}

/// A proof that a given accumulator represents the intersection of two other accumulators.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntersectionProof {
    /// g2^Q1(s)
    #[serde(with = "serde_impl")]
    pub witness_a: G2Affine,
    /// g2^Q2(s)
    #[serde(with = "serde_impl")]
    pub witness_b: G2Affine,
    /// g1^A(s) - coefficient for Bezout identity
    #[serde(with = "serde_impl")]
    pub witness_coprime_a: G1Affine,
    /// g1^B(s) - coefficient for Bezout identity
    #[serde(with = "serde_impl")]
    pub witness_coprime_b: G1Affine,
}

impl IntersectionProof {
    pub fn new(
        set1: &[Fr],
        set2: &[Fr],
        intersection_set: &[Fr],
    ) -> Result<(DynamicAccumulator, Self)> {
        // 1. Create the intersection accumulator
        let trapdoor = super::setup::PRI_S.clone();
        let intersection_acc = DynamicAccumulator::from_set(trapdoor, intersection_set);

        // 2. Compute witnesses using DynamicAccumulator logic
        let (witness_a, witness_b, witness_coprime_a, witness_coprime_b) =
            DynamicAccumulator::compute_intersection_witnesses(set1, set2, intersection_set)?;

        Ok((
            intersection_acc,
            Self {
                witness_a,
                witness_b,
                witness_coprime_a,
                witness_coprime_b,
            },
        ))
    }

    pub fn verify(
        &self,
        acc1_value: G1Affine,
        acc2_value: G1Affine,
        intersection_value: G1Affine,
    ) -> bool {
        let lhs1 = Curve::pairing(acc1_value, G2Affine::prime_subgroup_generator());
        let rhs1 = Curve::pairing(intersection_value, self.witness_a);

        let lhs2 = Curve::pairing(acc2_value, G2Affine::prime_subgroup_generator());
        let rhs2 = Curve::pairing(intersection_value, self.witness_b);

        // Verify coprimality: e(g1^A, g2^Q1) * e(g1^B, g2^Q2) = e(g1, g2)
        let coprimality_lhs1 = Curve::pairing(self.witness_coprime_a, self.witness_a);
        let coprimality_lhs2 = Curve::pairing(self.witness_coprime_b, self.witness_b);
        let coprimality_rhs = Curve::pairing(
            G1Affine::prime_subgroup_generator(),
            G2Affine::prime_subgroup_generator(),
        );

        lhs1 == rhs1 && lhs2 == rhs2 && (coprimality_lhs1 * coprimality_lhs2 == coprimality_rhs)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnionProof {
    #[serde(with = "serde_impl")]
    pub intersection_acc_value: G1Affine,
    pub intersection_proof: IntersectionProof,
}

impl UnionProof {
    pub fn new(
        intersection_acc: &DynamicAccumulator,
        intersection_proof: IntersectionProof,
        union_set: &[Fr],
    ) -> Result<(DynamicAccumulator, Self)> {
        // Reconstruct union accumulator
        let trapdoor = super::setup::PRI_S.clone();
        let union_acc = DynamicAccumulator::from_set(trapdoor, union_set);

        let union_proof = Self {
            intersection_acc_value: intersection_acc.acc_value,
            intersection_proof,
        };

        Ok((union_acc, union_proof))
    }

    pub fn verify(
        &self,
        acc1_value: G1Affine,
        acc2_value: G1Affine,
        union_acc_value: G1Affine,
    ) -> bool {
        let is_intersection_valid =
            self.intersection_proof
                .verify(acc1_value, acc2_value, self.intersection_acc_value);

        if !is_intersection_valid {
            return false;
        }

        // P(Union) = P(1) * P(2) / P(Intersection)
        //          = P(1) * Q2(s)
        // Where Q2(s) is witness_b from intersection proof (P(2) = P(Inter) * Q2)
        // Check: e(Union, g2) == e(Acc1, witness_b)
        let lhs = Curve::pairing(union_acc_value, G2Affine::prime_subgroup_generator());
        let rhs = Curve::pairing(acc1_value, self.intersection_proof.witness_b);

        lhs == rhs
    }
}

/// Disjointness Proof (formerly AccProof)
/// Prove Da ∩ Db = Ø
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisjointnessProof {
    #[serde(with = "serde_impl")]
    pub f1: G2Affine,
    #[serde(with = "serde_impl")]
    pub f2: G2Affine,
}

impl DisjointnessProof {
    pub fn new(set1: &[Fr], set2: &[Fr]) -> Result<Self> {
        let (f1, f2) = DynamicAccumulator::compute_disjointness_witnesses(set1, set2)?;
        Ok(Self { f1, f2 })
    }

    pub fn verify(&self, acc1: &G1Affine, acc2: &G1Affine) -> bool {
        Curve::product_of_pairings(&[
            ((*acc1).into(), self.f1.into()),
            ((*acc2).into(), self.f2.into()),
        ]) == *E_G_G
    }
}

#[cfg(test)]
mod tests {
    // Tests removed: UpdateProof is no longer supported
}
