pub mod digest_set;
pub mod dynamic_accumulator;
pub mod serde_impl;
pub mod utils;

pub use dynamic_accumulator::DynamicAccumulator;

pub use ark_bls12_381::{
    Bls12_381 as Curve, Fq12, Fr, G1Affine, G1Projective, G2Affine, G2Projective,
};
pub type DigestSet = digest_set::DigestSet<Fr>;

use crate::digest::{Digest, Digestible};
use crate::set::{MultiSet, SetElement};
use anyhow::{self, bail, ensure, Context};
use ark_ec::{msm::VariableBaseMSM, AffineCurve, PairingEngine, ProjectiveCurve};
use ark_ff::{Field, One, PrimeField, ToBytes, Zero};
use ark_poly::{univariate::DensePolynomial, Polynomial};
use core::any::Any;
use core::str::FromStr;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use utils::{xgcd, FixedBaseCurvePow, FixedBaseScalarPow};

#[cfg(test)]
const GS_VEC_LEN: usize = 0;
#[cfg(not(test))]
const GS_VEC_LEN: usize = 5000;

lazy_static::lazy_static! {
    // 250 bits
    static ref PUB_Q: Fr = Fr::from_str("480721077433357505777975950918924200361380912084288598463024400624539293706").unwrap();
    // 128 bits
    static ref PRI_S: Fr = Fr::from_str("259535143263514268207918833918737523409").unwrap();
    static ref G1_POWER: FixedBaseCurvePow<G1Projective> =
        FixedBaseCurvePow::build(&G1Projective::prime_subgroup_generator());
    static ref G2_POWER: FixedBaseCurvePow<G2Projective> =
        FixedBaseCurvePow::build(&G2Projective::prime_subgroup_generator());
    static ref PRI_S_POWER: FixedBaseScalarPow<Fr> = FixedBaseScalarPow::build(&PRI_S);
    static ref G1_S_VEC: Vec<G1Affine> = {
        let mut res: Vec<G1Affine> = Vec::with_capacity(GS_VEC_LEN);
        (0..GS_VEC_LEN)
            .into_par_iter()
            .map(|i| get_g1s(Fr::from(i as u64)))
            .collect_into_vec(&mut res);
        res
    };
    static ref G2_S_VEC: Vec<G2Affine> = {
        let mut res: Vec<G2Affine> = Vec::with_capacity(GS_VEC_LEN);
        (0..GS_VEC_LEN)
            .into_par_iter()
            .map(|i| get_g2s(Fr::from(i as u64)))
            .collect_into_vec(&mut res);
        res
    };
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
    ACC1,
    ACC2,
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

pub struct Acc1;

impl Acc1 {
    fn poly_to_g1(poly: DensePolynomial<Fr>) -> G1Affine {
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
                G1_S_VEC.get(i).copied().unwrap_or_else(|| {
                    get_g1s(Fr::from(i as u64))
                })
            })
            .collect_into_vec(&mut bases);
        (0..idxes.len())
            .into_par_iter()
            .map(|i| poly.coeffs[i].into_repr())
            .collect_into_vec(&mut scalars);

        VariableBaseMSM::multi_scalar_mul(&bases[..], &scalars[..]).into_affine()
    }

    fn poly_to_g2(poly: DensePolynomial<Fr>) -> G2Affine {
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
                G2_S_VEC.get(i).copied().unwrap_or_else(|| {
                    get_g2s(Fr::from(i as u64))
                })
            })
            .collect_into_vec(&mut bases);
        (0..idxes.len())
            .into_par_iter()
            .map(|i| poly.coeffs[i].into_repr())
            .collect_into_vec(&mut scalars);

        VariableBaseMSM::multi_scalar_mul(&bases[..], &scalars[..]).into_affine()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Acc1Proof {
    #[serde(with = "serde_impl")]
    f1: G2Affine,
    #[serde(with = "serde_impl")]
    f2: G2Affine,
}

impl AccumulatorProof for Acc1Proof {
    const TYPE: Type = Type::ACC1;

    fn gen_proof(set1: &DigestSet, set2: &DigestSet) -> anyhow::Result<Self> {
        Acc1::gen_proof(set1, set2)
    }

    fn combine_proof(&mut self, _other: &Self) -> anyhow::Result<()> {
        bail!("invalid operation");
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Acc1Proof {
    pub fn verify(&self, acc1: &G1Affine, acc2: &G1Affine) -> bool {
        Curve::product_of_pairings(&[
            ((*acc1).into(), self.f1.into()),
            ((*acc2).into(), self.f2.into()),
        ]) == *E_G_G
    }
}

impl Accumulator for Acc1 {
    const TYPE: Type = Type::ACC1;
    type Proof = Acc1Proof;

    fn cal_acc_g1_sk_d(set: &DigestSet) -> G1Affine {
        let x = set
            .par_iter()
            .map(|(v, exp)| {
                let s = *PRI_S - v;
                let exp = [*exp as u64];
                s.pow(exp)
            })
            .reduce(Fr::one, |a, b| a * b);
        G1_POWER.apply(&x).into_affine()
    }
    fn cal_acc_g1_d(set: &DigestSet) -> G1Affine {
        let poly = set.expand_to_poly();
        Self::poly_to_g1(poly)
    }
    fn cal_acc_g2_sk_d(set: &DigestSet) -> G2Affine {
        let x = set
            .par_iter()
            .map(|(v, exp)| {
                let s = *PRI_S - v;
                let exp = [*exp as u64];
                s.pow(exp)
            })
            .reduce(Fr::one, |a, b| a * b);
        G2_POWER.apply(&x).into_affine()
    }
    fn cal_acc_g2_d(set: &DigestSet) -> G2Affine {
        let poly = set.expand_to_poly();
        Self::poly_to_g2(poly)
    }
    fn gen_proof(set1: &DigestSet, set2: &DigestSet) -> anyhow::Result<Self::Proof> {
        let poly1 = set1.expand_to_poly();
        let poly2 = set2.expand_to_poly();
        let (g, x, y) = xgcd(poly1, poly2).context("failed to compute xgcd")?;
        ensure!(g.degree() == 0, "cannot generate proof");
        Ok(Acc1Proof {
            f1: Self::poly_to_g2(&x / &g),
            f2: Self::poly_to_g2(&y / &g),
        })
    }
}

impl Digestible for G1Affine {
    fn to_digest(&self) -> Digest {
        let mut buf = Vec::<u8>::new();
        self.write(&mut buf)
            .unwrap_or_else(|_| panic!("failed to serialize {:?}", self));
        buf.to_digest()
    }
}
