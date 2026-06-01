//! General Math Toolbox
//!
//! This module contains pure mathematical algorithms and low-level type conversions
//! used by the accumulator.
//!
//! # Architecture Note
//! This file should NOT contain any business logic (like accumulator structs or proof verification).
//! It is strictly for:
//! - Pure math algorithms (e.g., XGCD, FixedBasePow)
//! - Low-level type conversions (e.g., Digest -> Field)
//! - Set transformations and polynomial expansions

use crate::digest::{Digest, Digestible};
use crate::set::{Set, SetElement};
use crate::acc::setup::{get_g1s, get_g2s};
use ark_bls12_381::{Fr, G1Affine, G2Affine};
use ark_ec::{msm::VariableBaseMSM, ProjectiveCurve};
use ark_ff::{BigInteger, Field, FpParameters, PrimeField, ToBytes, Zero};
use ark_poly::{
    univariate::{DenseOrSparsePolynomial, DensePolynomial},
    Polynomial, UVPolynomial,
};
use log::trace;
use rayon::prelude::*;
use std::borrow::Cow;
use std::iter;

// ==========================================
// Set Transformation Functions
// ==========================================

/// Convert a Set<T> to Vec<F> by hashing each element to prime field.
/// Uses parallel iteration for performance.
pub fn digest_set_from_set<T: SetElement, F: PrimeField>(input: &Set<T>) -> Vec<F> {
    let elements: Vec<&T> = input.iter().collect();
    let mut result: Vec<F> = Vec::with_capacity(elements.len());
    
    (0..elements.len())
        .into_par_iter()
        .map(|i| {
            let k = elements[i];
            let d = k.to_digest();
            digest_to_prime_field(&d)
        })
        .collect_into_vec(&mut result);
    
    result
}

/// Expand a slice of field elements to polynomial ∏(X - xᵢ).
/// Uses parallel divide-and-conquer for performance.
pub fn expand_to_poly<F: PrimeField>(elements: &[F]) -> DensePolynomial<F> {
    let mut inputs = Vec::new();
    for k in elements {
        inputs.push(DensePolynomial::from_coefficients_vec(vec![
            k.neg(),
            F::one(),
        ]));
    }

    fn expand<'a, F: PrimeField>(
        polys: &'a [DensePolynomial<F>],
    ) -> Cow<'a, DensePolynomial<F>> {
        if polys.is_empty() {
            return Cow::Owned(DensePolynomial::from_coefficients_vec(vec![F::one()]));
        } else if polys.len() == 1 {
            return Cow::Borrowed(&polys[0]);
        }
        let mid = polys.len() / 2;
        let (left, right) = rayon::join(|| expand(&polys[..mid]), || expand(&polys[mid..]));
        Cow::Owned(left.as_ref() * right.as_ref())
    }

    expand(&inputs).into_owned()
}

// ==========================================
// Type Conversion Functions
// ==========================================

impl Digestible for G1Affine {
    fn to_digest(&self) -> Digest {
        let mut buf = Vec::<u8>::new();
        self.write(&mut buf)
            .unwrap_or_else(|_| panic!("failed to serialize {:?}", self));
        buf.to_digest()
    }
}

pub fn try_digest_to_prime_field<F: PrimeField>(input: &Digest) -> Option<F> {
    let mut num = F::from_be_bytes_mod_order(&input.0).into_repr();
    // Ensure 248-bit limit to prevent overflow.
    for v in num.as_mut().iter_mut().skip(3) {
        *v = 0;
    }
    if let Some(v) = num.as_mut().get_mut(3) {
        *v &= 0x00ff_ffff_ffff_ffff;
    }
    F::from_repr(num)
}

pub fn digest_to_prime_field<F: PrimeField>(input: &Digest) -> F {
    try_digest_to_prime_field(input).expect("failed to convert digest to prime field")
}

/// Return (g, x, y) s.t. a*x + b*y = g = gcd(a, b)
pub fn xgcd<'a, F: PrimeField>(
    a: impl Into<DenseOrSparsePolynomial<'a, F>>,
    b: impl Into<DenseOrSparsePolynomial<'a, F>>,
) -> Option<(DensePolynomial<F>, DensePolynomial<F>, DensePolynomial<F>)> {
    let mut a = a.into();
    let mut b = b.into();
    let mut x0 = DensePolynomial::<F>::zero();
    let mut x1 = DensePolynomial::<F>::from_coefficients_vec(vec![F::one()]);
    let mut y0 = DensePolynomial::<F>::from_coefficients_vec(vec![F::one()]);
    let mut y1 = DensePolynomial::<F>::zero();
    while !a.is_zero() {
        let (q, r) = b.divide_with_q_and_r(&a)?;
        b = a;
        a = r.into();
        let y1old = y1;
        y1 = &y0 - &(&q * &y1old);
        y0 = y1old;
        let x1old = x1;
        x1 = &x0 - &(&q * &x1old);
        x0 = x1old;
    }
    Some((b.into(), x0, y0))
}

// Ref: https://github.com/blynn/pbc/blob/fbf4589036ce4f662e2d06905862c9e816cf9d08/arith/field.c#L251-L330
pub struct FixedBaseCurvePow<G: ProjectiveCurve> {
    table: Vec<Vec<G>>,
}

impl<G: ProjectiveCurve> FixedBaseCurvePow<G> {
    const K: usize = 5;

    pub fn build(base: &G) -> Self {
        let bits =
            <<G as ProjectiveCurve>::ScalarField as PrimeField>::Params::MODULUS_BITS as usize;
        let num_lookups = bits / Self::K + 1;
        let lookup_size = (1 << Self::K) - 1;
        let last_lookup_size = (1 << (bits - (num_lookups - 1) * Self::K)) - 1;

        let mut table: Vec<Vec<G>> = Vec::with_capacity(num_lookups);

        let mut multiplier = *base;
        for i in 0..num_lookups {
            let table_size = if i == num_lookups - 1 {
                last_lookup_size
            } else {
                lookup_size
            };
            let sub_table: Vec<G> = iter::from_fn({
                let mut current = multiplier;
                move || {
                    let ret = current;
                    current.add_assign(&multiplier);
                    Some(ret)
                }
            })
            .take(table_size)
            .collect();
            table.push(sub_table);
            if i != num_lookups - 1 {
                let last = *table.last().unwrap().last().unwrap();
                multiplier.add_assign(&last);
            }
        }
        Self { table }
    }

    pub fn apply(&self, input: &<G as ProjectiveCurve>::ScalarField) -> G {
        let mut res = G::zero();
        let input_repr = input.into_repr();
        let num_lookups = input_repr.num_bits() as usize / Self::K + 1;
        for i in 0..num_lookups {
            let mut word: usize = 0;
            for j in 0..Self::K {
                if input_repr.get_bit(i * Self::K + j) {
                    word |= 1 << j;
                }
            }
            if word > 0 {
                res.add_assign(&self.table[i][word - 1]);
            }
        }
        res
    }
}

pub struct FixedBaseScalarPow<F: PrimeField> {
    table: Vec<Vec<F>>,
}

impl<F: PrimeField> FixedBaseScalarPow<F> {
    const K: usize = 8;

    pub fn build(base: &F) -> Self {
        let bits = <F as PrimeField>::Params::MODULUS_BITS as usize;
        let num_lookups = bits / Self::K + 1;
        let lookup_size = (1 << Self::K) - 1;
        let last_lookup_size = (1 << (bits - (num_lookups - 1) * Self::K)) - 1;

        let mut table: Vec<Vec<F>> = Vec::with_capacity(num_lookups);

        let mut multiplier = *base;
        for i in 0..num_lookups {
            let table_size = if i == num_lookups - 1 {
                last_lookup_size
            } else {
                lookup_size
            };
            let sub_table: Vec<F> = iter::from_fn({
                let mut current = multiplier;
                move || {
                    let ret = current;
                    current.mul_assign(&multiplier);
                    Some(ret)
                }
            })
            .take(table_size)
            .collect();
            table.push(sub_table);
            if i != num_lookups - 1 {
                let last = *table.last().unwrap().last().unwrap();
                multiplier.mul_assign(&last);
            }
        }
        Self { table }
    }

    pub fn apply(&self, input: &F) -> F {
        let mut res = F::one();
        let input_repr = input.into_repr();
        let num_lookups = input_repr.num_bits() as usize / Self::K + 1;
        for i in 0..num_lookups {
            let mut word: usize = 0;
            for j in 0..Self::K {
                if input_repr.get_bit(i * Self::K + j) {
                    word |= 1 << j;
                }
            }
            if word > 0 {
                res.mul_assign(&self.table[i][word - 1]);
            }
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bls12_381::{Fr, G1Projective, G2Projective};
    use ark_ff::Field;
    use ark_poly::Polynomial;
    use core::ops::MulAssign;
    use rand::Rng;

    #[test]
    fn test_xgcd() {
        let poly1 = DensePolynomial::from_coefficients_vec(vec![Fr::from(1u32), Fr::from(1u32)]);
        let poly2 = DensePolynomial::from_coefficients_vec(vec![Fr::from(2u32), Fr::from(1u32)]);
        let (g, x, y) = xgcd(&poly1, &poly2).unwrap();
        assert_eq!(g.degree(), 0);
        let gcd = &(&poly1 * &x) + &(&poly2 * &y);
        assert_eq!(gcd, g);
    }

    #[test]
    fn test_pow_g1() {
        let g1p = FixedBaseCurvePow::build(&G1Projective::prime_subgroup_generator());
        let mut rng = rand::thread_rng();
        let num: Fr = rng.gen();
        let mut expect = G1Projective::prime_subgroup_generator();
        expect.mul_assign(num);
        assert_eq!(g1p.apply(&num), expect);
    }

    #[test]
    fn test_pow_g2() {
        let g2p = FixedBaseCurvePow::build(&G2Projective::prime_subgroup_generator());
        let mut rng = rand::thread_rng();
        let num: Fr = rng.gen();
        let mut expect = G2Projective::prime_subgroup_generator();
        expect.mul_assign(num);
        assert_eq!(g2p.apply(&num), expect);
    }

    #[test]
    fn test_pow_fr() {
        let mut rng = rand::thread_rng();
        let base: Fr = rng.gen();
        let num: Fr = rng.gen();
        let frp = FixedBaseScalarPow::build(&base);
        let expect = base.pow(num.into_repr());
        assert_eq!(frp.apply(&num), expect);
    }
}

pub fn poly_to_g1(poly: DensePolynomial<Fr>) -> G1Affine {
    let mut idxes: Vec<usize> = Vec::with_capacity(poly.degree() + 1);
    for (i, coeff) in poly.coeffs.iter().enumerate() {
        if coeff.is_zero() {
            continue;
        }
        idxes.push(i);
    }

    let mut bases: Vec<G1Affine> = Vec::with_capacity(idxes.len());
    let mut scalars: Vec<<Fr as PrimeField>::BigInt> = Vec::with_capacity(idxes.len());
    (0..idxes.len())
        .into_par_iter()
        .map(|i| {
            let idx = idxes[i];
            trace!("access g1 pub key at {}", idx);
            get_g1s(idx)
        })
        .collect_into_vec(&mut bases);
    (0..idxes.len())
        .into_par_iter()
        .map(|i| poly.coeffs[idxes[i]].into_repr())
        .collect_into_vec(&mut scalars);

    VariableBaseMSM::multi_scalar_mul(&bases[..], &scalars[..]).into_affine()
}

pub fn poly_to_g2(poly: DensePolynomial<Fr>) -> G2Affine {
    let mut idxes: Vec<usize> = Vec::with_capacity(poly.degree() + 1);
    for (i, coeff) in poly.coeffs.iter().enumerate() {
        if coeff.is_zero() {
            continue;
        }
        idxes.push(i);
    }

    let mut bases: Vec<G2Affine> = Vec::with_capacity(idxes.len());
    let mut scalars: Vec<<Fr as PrimeField>::BigInt> = Vec::with_capacity(idxes.len());
    (0..idxes.len())
        .into_par_iter()
        .map(|i| {
            let idx = idxes[i];
            trace!("access g2 pub key at {}", idx);
            get_g2s(idx)
        })
        .collect_into_vec(&mut bases);
    (0..idxes.len())
        .into_par_iter()
        .map(|i| poly.coeffs[idxes[i]].into_repr())
        .collect_into_vec(&mut scalars);

    VariableBaseMSM::multi_scalar_mul(&bases[..], &scalars[..]).into_affine()
}

/// Solves A*P1 + B*P2 = 1 (GCD normalized).
/// Returns (A, B) such that A*P1 + B*P2 = 1.
pub fn solve_bezout_identity(
    p1: DensePolynomial<Fr>,
    p2: DensePolynomial<Fr>,
) -> anyhow::Result<(DensePolynomial<Fr>, DensePolynomial<Fr>)> {
    let (g, a, b) = xgcd(p1, p2).ok_or_else(|| anyhow::anyhow!("XGCD failed"))?;

    if g.degree() != 0 {
        return Err(anyhow::anyhow!("Polynomials are not coprime"));
    }

    let g_val = g.coeffs[0];
    let g_inv = g_val
        .inverse()
        .ok_or_else(|| anyhow::anyhow!("GCD constant has no inverse"))?;

    let scale_poly = |poly: DensePolynomial<Fr>| -> DensePolynomial<Fr> {
        DensePolynomial::from_coefficients_vec(poly.coeffs.into_iter().map(|c| c * g_inv).collect())
    };

    Ok((scale_poly(a), scale_poly(b)))
}
