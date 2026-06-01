//! Implements a dynamic cryptographic accumulator that supports additions, deletions,
//! intersection, union, and disjointness proofs.
//!
//! # Trapdoor Note
//! This implementation requires a secret trapdoor (private key) to perform operations.
//! The trapdoor should be injected through the constructor for better testability
//! and separation of concerns.

use anyhow::{anyhow, ensure, Context, Result};
use ark_bls12_381::{Fr, G1Affine, G1Projective, G2Affine};
use ark_ec::{AffineCurve, ProjectiveCurve};
use ark_ff::{Field, One, PrimeField, Zero};
use ark_poly::{
    univariate::{DenseOrSparsePolynomial, DensePolynomial},
    UVPolynomial,
};
use std::ops::Neg;

use super::proofs::{MembershipProof, NonMembershipProof};
use crate::acc::utils::{expand_to_poly, poly_to_g1, poly_to_g2};

/// Represents the result of a query against the accumulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryResult {
    /// The element is in the set, and here is the proof.
    Membership(MembershipProof),
    /// The element is not in the set, and here is the proof.
    NonMembership(Box<NonMembershipProof>),
}

/// A dynamic cryptographic accumulator based on the Acc scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicAccumulator {
    /// The current accumulator value, g1^P(s).
    pub acc_value: G1Affine,
    /// The secret trapdoor used for O(1) operations.
    /// Injected through constructor for better testability and modularity.
    trapdoor: Fr,
}

impl Default for DynamicAccumulator {
    fn default() -> Self {
        // Use the default trapdoor from setup for backward compatibility
        Self::new(super::setup::PRI_S.clone())
    }
}

impl DynamicAccumulator {
    /// Creates a new, empty dynamic accumulator with the given trapdoor.
    ///
    /// # Arguments
    /// * `trapdoor` - The secret key used for O(1) accumulator operations
    ///
    /// # Examples
    /// ```
    /// use accumulator_ads::DynamicAccumulator;
    /// use ark_bls12_381::Fr;
    /// use ark_ff::PrimeField;
    ///
    /// let trapdoor = Fr::from(12345u64);
    /// let acc = DynamicAccumulator::new(trapdoor);
    /// ```
    pub fn new(trapdoor: Fr) -> Self {
        Self {
            acc_value: G1Projective::from(G1Affine::prime_subgroup_generator())
                .mul(Fr::one().into_repr())
                .into_affine(),
            trapdoor,
        }
    }

    /// Returns the commitment value for an empty accumulator.
    ///
    /// This is a convenience method for upper layers to obtain the empty accumulator
    /// commitment without needing to know the internal implementation details
    /// (empty set -> digest -> commitment computation).
    ///
    /// # Returns
    /// The G1 point representing the commitment of an empty set.
    pub fn empty_commitment() -> G1Affine {
        Self::calculate_commitment(&[])
    }

    /// Static method: Fast calculation of set commitment using MSM.
    /// Used when initializing from a large set from scratch.
    pub fn calculate_commitment(elements: &[Fr]) -> G1Affine {
        poly_to_g1(expand_to_poly(elements))
    }

    /// Factory method: Initialize accumulator from field elements with the given trapdoor.
    ///
    /// # Arguments
    /// * `trapdoor` - The secret key used for O(1) accumulator operations
    /// * `elements` - The initial set of elements to accumulate
    pub fn from_set(trapdoor: Fr, elements: &[Fr]) -> Self {
        Self {
            acc_value: Self::calculate_commitment(elements),
            trapdoor,
        }
    }

    /// Helper: Compute G2 commitment
    pub fn calculate_commitment_g2(elements: &[Fr]) -> G2Affine {
        poly_to_g2(expand_to_poly(elements))
    }

    // ==========================================
    // Backward Compatibility Helpers
    // ==========================================

    /// Creates an accumulator using the default trapdoor from setup.
    /// This method provides backward compatibility for existing code.
    ///
    /// # Note
    /// For new code, prefer using `new(trapdoor)` with explicit trapdoor injection.
    pub fn with_default_trapdoor() -> Self {
        Self::new(super::setup::PRI_S.clone())
    }

    /// Creates an accumulator from an existing accumulator value using the default trapdoor.
    /// This is useful when you have an accumulator value but need to perform operations on it.
    ///
    /// # Arguments
    /// * `acc_value` - The existing accumulator value
    ///
    /// # Note
    /// This method assumes the accumulator was created with the default trapdoor.
    pub fn from_value(acc_value: G1Affine) -> Self {
        Self {
            acc_value,
            trapdoor: super::setup::PRI_S.clone(),
        }
    }

    /// Static helper: Incrementally adds elements to an accumulator value using the default trapdoor.
    /// This method provides backward compatibility for existing code.
    ///
    /// # Arguments
    /// * `current_acc` - The current accumulator value
    /// * `new_elements` - The elements to add
    ///
    /// # Note
    /// For new code, prefer using the instance method `incremental_add_elements`.
    pub fn incremental_add_with_default_trapdoor(
        current_acc: G1Affine,
        new_elements: &[Fr],
    ) -> G1Affine {
        let temp_acc = Self {
            acc_value: current_acc,
            trapdoor: super::setup::PRI_S.clone(),
        };
        temp_acc.incremental_add_elements(new_elements)
    }

    // ==========================================
    // 1. Add & Delete (With Trapdoor s)
    // ==========================================

    /// Computes the new accumulator value after adding an element using the trapdoor.
    /// acc' = acc^(s - element)
    pub fn compute_add(&self, element: Fr) -> G1Affine {
        let s_minus_elem: Fr = self.trapdoor - element;
        self.acc_value.mul(s_minus_elem).into_affine()
    }

    /// Computes the new accumulator value after deleting an element using the trapdoor.
    /// acc' = acc^(1 / (s - element))
    pub fn compute_delete(&self, element: Fr) -> Result<G1Affine> {
        let s_minus_elem: Fr = self.trapdoor - element;
        let inverse = s_minus_elem.inverse().ok_or_else(|| {
            anyhow!("Failed to compute inverse: element might be equal to s (Trapdoor collision)")
        })?;

        Ok(self.acc_value.mul(inverse).into_affine())
    }



    // ==========================================
    // Incremental Update Operations
    // ==========================================

    /// Incrementally update accumulator by adding multiple elements using the trapdoor.
    /// Time complexity: O(k) scalar multiplications + O(1) point multiplication.
    ///
    /// Optimized implementation: Instead of doing k point multiplications,
    /// we compute the product of all scalars (s-x₁)(s-x₂)...(s-xₖ) first,
    /// then perform a single point multiplication: Acc^(∏(s-xᵢ))
    ///
    /// Performance gain: Scalar field operations are orders of magnitude faster
    /// than elliptic curve point operations.
    pub fn incremental_add_elements(&self, new_elements: &[Fr]) -> G1Affine {
        if new_elements.is_empty() {
            return self.acc_value;
        }

        // Step 1: Compute the product of all (s - xᵢ) in the scalar field
        // This is much faster than repeated point multiplications
        let exponent_product = new_elements
            .iter()
            .fold(Fr::one(), |acc, &elem| acc * (self.trapdoor - elem));

        // Step 2: Single point multiplication - only one expensive operation
        self.acc_value.mul(exponent_product).into_affine()
    }

    // ==========================================
    // 2. Query
    // ==========================================

    /// Computes the membership witness (proof) for an element.
    /// The witness for `x` is simply the accumulator value as if `x` was deleted.
    /// witness = acc^(1/(s-x))
    pub fn compute_membership_witness(&self, element: Fr) -> Result<G1Affine> {
        self.compute_delete(element)
    }

    /// Computes witnesses for non-membership.
    /// Returns (witness=g2^B(s), g2_a=g2^A(s)) where A(x)P(x) + B(x)(x-element) = 1
    pub fn compute_non_membership_witness(
        element: Fr,
        elements: &[Fr],
    ) -> Result<(G2Affine, G2Affine)> {
        // 1. Construct P(X)
        let p_poly = expand_to_poly(elements);

        // 2. Construct (X - element)
        let elem_poly = DensePolynomial::from_coefficients_vec(vec![element.neg(), Fr::one()]);

        // 3. Solve Bezout identity
        let (a_poly, b_poly) = crate::acc::utils::solve_bezout_identity(p_poly, elem_poly)
            .context("GCD is not constant, element might be in set")?;

        Ok((poly_to_g2(b_poly), poly_to_g2(a_poly)))
    }

    // ==========================================
    // 3. Set Operations Witnesses
    // ==========================================

    /// Computes witnesses for intersection proof.
    pub fn compute_intersection_witnesses(
        set1: &[Fr],
        set2: &[Fr],
        intersection_set: &[Fr],
    ) -> Result<(G2Affine, G2Affine, G1Affine, G1Affine)> {
        let p1_poly = expand_to_poly(set1);
        let p2_poly = expand_to_poly(set2);
        let p_intersect_poly = expand_to_poly(intersection_set);

        // Helper closure for exact division
        let divide_exact = |num: &DensePolynomial<Fr>,
                            den: &DensePolynomial<Fr>,
                            err_msg: &str|
         -> Result<DensePolynomial<Fr>> {
            let (q, r) = DenseOrSparsePolynomial::from(num)
                .divide_with_q_and_r(&DenseOrSparsePolynomial::from(den))
                .ok_or_else(|| anyhow!("Division failed"))?;
            ensure!(r.is_zero(), "{}", err_msg);
            Ok(q)
        };

        let q1_poly = divide_exact(
            &p1_poly,
            &p_intersect_poly,
            "P_intersect does not divide P1",
        )?;
        let q2_poly = divide_exact(
            &p2_poly,
            &p_intersect_poly,
            "P_intersect does not divide P2",
        )?;

        let witness_a = poly_to_g2(q1_poly.clone());
        let witness_b = poly_to_g2(q2_poly.clone());

        let (a_poly, b_poly) = crate::acc::utils::solve_bezout_identity(q1_poly, q2_poly)
            .context("Quotients might not be coprime")?;

        Ok((witness_a, witness_b, poly_to_g1(a_poly), poly_to_g1(b_poly)))
    }

    /// Computes witnesses for disjointness proof.
    pub fn compute_disjointness_witnesses(
        set1: &[Fr],
        set2: &[Fr],
    ) -> Result<(G2Affine, G2Affine)> {
        let poly1 = expand_to_poly(set1);
        let poly2 = expand_to_poly(set2);

        let (x_poly, y_poly) = crate::acc::utils::solve_bezout_identity(poly1, poly2)
            .context("Sets are not disjoint")?;

        Ok((poly_to_g2(x_poly), poly_to_g2(y_poly)))
    }
}
