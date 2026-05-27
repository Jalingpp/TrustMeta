use crate::SetProofLeaf;
use ads_rust::acctrie::acc::digest_set::DigestSet as RawDigestSet;
use ads_rust::acctrie::acc::utils::xgcd;
use ads_rust::acctrie::acc::{Fr, MultiSet};
use ark_ff::{Field, PrimeField, Zero};
use ark_poly::{
    univariate::{DenseOrSparsePolynomial, DensePolynomial},
    Polynomial, UVPolynomial,
};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const POLYNOMIAL_INTERSECTION_PROOF_MAGIC: &[u8] = b"POLYI1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransparentPolynomial {
    pub commitment: Vec<u8>,
    pub coefficients: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolynomialIntersectionLeafProof {
    pub leaf: SetProofLeaf,
    pub set_polynomial: TransparentPolynomial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolynomialSetProofNode {
    Leaf(PolynomialIntersectionLeafProof),
    And(Box<PolynomialIntersectionNodeProof>),
    Or(Box<PolynomialUnionNodeProof>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolynomialIntersectionNodeProof {
    pub set_polynomial: TransparentPolynomial,
    pub intersection_polynomial: TransparentPolynomial,
    pub left: PolynomialSetProofNode,
    pub right: PolynomialSetProofNode,
    pub quotient_left: TransparentPolynomial,
    pub quotient_right: TransparentPolynomial,
    pub bezout_u: TransparentPolynomial,
    pub bezout_v: TransparentPolynomial,
    pub challenge_point: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolynomialUnionNodeProof {
    pub set_polynomial: TransparentPolynomial,
    pub intersection_polynomial: TransparentPolynomial,
    pub left: PolynomialSetProofNode,
    pub right: PolynomialSetProofNode,
    pub quotient_left: TransparentPolynomial,
    pub quotient_right: TransparentPolynomial,
    pub bezout_u: TransparentPolynomial,
    pub bezout_v: TransparentPolynomial,
    pub challenge_point: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolynomialIntersectionAggregateProof {
    pub expr: String,
    pub result_fids: Vec<String>,
    pub root: PolynomialSetProofNode,
}

pub fn encode_polynomial_intersection_proof(
    proof: &PolynomialIntersectionAggregateProof,
) -> Result<Vec<u8>, String> {
    let payload = bincode::serialize(proof)
        .map_err(|e| format!("serialize polynomial intersection proof failed: {}", e))?;
    let mut bytes = Vec::with_capacity(POLYNOMIAL_INTERSECTION_PROOF_MAGIC.len() + payload.len());
    bytes.extend_from_slice(POLYNOMIAL_INTERSECTION_PROOF_MAGIC);
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

pub fn is_polynomial_intersection_proof(proof: &[u8]) -> bool {
    proof.starts_with(POLYNOMIAL_INTERSECTION_PROOF_MAGIC)
}

pub fn decode_polynomial_intersection_proof(
    proof: &[u8],
) -> Result<PolynomialIntersectionAggregateProof, String> {
    if !is_polynomial_intersection_proof(proof) {
        return Err("invalid polynomial intersection proof magic".to_string());
    }

    bincode::deserialize(&proof[POLYNOMIAL_INTERSECTION_PROOF_MAGIC.len()..])
        .map_err(|e| format!("deserialize polynomial intersection proof failed: {}", e))
}

pub fn polynomial_intersection_root_hash(proof: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(proof);
    hasher.finalize().to_vec()
}

pub fn build_characteristic_polynomial(values: &[String]) -> Result<TransparentPolynomial, String> {
    let set = MultiSet::from_vec(values.to_vec());
    let poly = RawDigestSet::<Fr>::new(&set).expand_to_poly();
    transparent_polynomial_from_dense(&poly)
}

pub fn quotient_polynomial(
    numerator: &TransparentPolynomial,
    denominator: &TransparentPolynomial,
) -> Result<TransparentPolynomial, String> {
    let numerator_poly = dense_from_transparent(numerator)?;
    let denominator_poly = dense_from_transparent(denominator)?;
    let (quotient, remainder) = DenseOrSparsePolynomial::from(&numerator_poly)
        .divide_with_q_and_r(&DenseOrSparsePolynomial::from(&denominator_poly))
        .ok_or_else(|| "polynomial division failed".to_string())?;
    if !remainder.is_zero() {
        return Err("polynomial division has non-zero remainder".to_string());
    }
    transparent_polynomial_from_dense(&quotient)
}

pub fn normalize_bezout_identity(
    left: &TransparentPolynomial,
    right: &TransparentPolynomial,
) -> Result<(TransparentPolynomial, TransparentPolynomial), String> {
    let left_poly = dense_from_transparent(left)?;
    let right_poly = dense_from_transparent(right)?;
    let (gcd, mut u, mut v) = xgcd(&left_poly, &right_poly)
        .ok_or_else(|| "extended gcd for quotient polynomials failed".to_string())?;

    if gcd.is_zero() {
        return Err("gcd is zero".to_string());
    }
    if gcd.degree() != 0 {
        return Err("quotient polynomials are not coprime".to_string());
    }

    let constant = gcd.coeffs.first().cloned().unwrap_or_else(Fr::zero);
    if constant.is_zero() {
        return Err("gcd constant is zero".to_string());
    }
    let inv = constant
        .inverse()
        .ok_or_else(|| "failed to invert gcd constant".to_string())?;
    for coeff in &mut u.coeffs {
        *coeff *= inv;
    }
    for coeff in &mut v.coeffs {
        *coeff *= inv;
    }

    Ok((
        transparent_polynomial_from_dense(&u)?,
        transparent_polynomial_from_dense(&v)?,
    ))
}

pub fn build_polynomial_intersection_node(
    left: PolynomialSetProofNode,
    right: PolynomialSetProofNode,
    result_fids: Vec<String>,
) -> Result<PolynomialIntersectionNodeProof, String> {
    let set_polynomial = build_characteristic_polynomial(&result_fids)?;
    let left_poly = polynomial_for_node(&left);
    let right_poly = polynomial_for_node(&right);
    let intersection_polynomial = set_polynomial.clone();
    let quotient_left = quotient_polynomial(left_poly, &intersection_polynomial)?;
    let quotient_right = quotient_polynomial(right_poly, &intersection_polynomial)?;
    let (bezout_u, bezout_v) = normalize_bezout_identity(&quotient_left, &quotient_right)?;
    let challenge_point = derive_polynomial_challenge(&[
        left_poly,
        right_poly,
        &set_polynomial,
        &intersection_polynomial,
        &quotient_left,
        &quotient_right,
        &bezout_u,
        &bezout_v,
    ]);

    Ok(PolynomialIntersectionNodeProof {
        set_polynomial,
        intersection_polynomial,
        left,
        right,
        quotient_left,
        quotient_right,
        bezout_u,
        bezout_v,
        challenge_point,
    })
}

pub fn build_polynomial_union_node(
    left: PolynomialSetProofNode,
    right: PolynomialSetProofNode,
    intersection_fids: Vec<String>,
    result_fids: Vec<String>,
) -> Result<PolynomialUnionNodeProof, String> {
    let set_polynomial = build_characteristic_polynomial(&result_fids)?;
    let intersection_polynomial = build_characteristic_polynomial(&intersection_fids)?;
    let left_poly = polynomial_for_node(&left);
    let right_poly = polynomial_for_node(&right);
    let quotient_left = quotient_polynomial(left_poly, &intersection_polynomial)?;
    let quotient_right = quotient_polynomial(right_poly, &intersection_polynomial)?;
    let (bezout_u, bezout_v) = normalize_bezout_identity(&quotient_left, &quotient_right)?;
    let expected_union = multiply_polynomials(
        &intersection_polynomial,
        &multiply_polynomials(&quotient_left, &quotient_right)?,
    )?;
    if expected_union != set_polynomial {
        return Err("union polynomial does not match the provided result set".to_string());
    }
    let challenge_point = derive_polynomial_challenge(&[
        left_poly,
        right_poly,
        &set_polynomial,
        &intersection_polynomial,
        &quotient_left,
        &quotient_right,
        &bezout_u,
        &bezout_v,
    ]);

    Ok(PolynomialUnionNodeProof {
        set_polynomial,
        intersection_polynomial,
        left,
        right,
        quotient_left,
        quotient_right,
        bezout_u,
        bezout_v,
        challenge_point,
    })
}

pub fn build_polynomial_intersection_proof(
    expr: String,
    result_fids: Vec<String>,
    left: PolynomialSetProofNode,
    right: PolynomialSetProofNode,
) -> Result<PolynomialIntersectionAggregateProof, String> {
    Ok(PolynomialIntersectionAggregateProof {
        expr,
        result_fids: result_fids.clone(),
        root: PolynomialSetProofNode::And(Box::new(build_polynomial_intersection_node(
            left,
            right,
            result_fids,
        )?)),
    })
}

pub fn derive_polynomial_challenge(polys: &[&TransparentPolynomial]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    for poly in polys {
        hasher.update(&poly.commitment);
    }
    let scalar = Fr::from_le_bytes_mod_order(&hasher.finalize());
    serialize_fr(&scalar)
}

pub fn verify_transparent_polynomial(poly: &TransparentPolynomial) -> bool {
    match dense_from_transparent(poly) {
        Ok(dense) => {
            polynomial_commitment_hash(&polynomial_to_coefficient_bytes(&dense)) == poly.commitment
        }
        Err(_) => false,
    }
}

pub fn evaluate_transparent_polynomial(
    poly: &TransparentPolynomial,
    point_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    let dense = dense_from_transparent(poly)?;
    let point = deserialize_fr(point_bytes)?;
    Ok(serialize_fr(&dense.evaluate(&point)))
}

pub fn polynomial_to_coefficient_bytes(poly: &DensePolynomial<Fr>) -> Vec<Vec<u8>> {
    poly.coeffs.iter().map(serialize_fr).collect()
}

pub fn transparent_polynomial_from_coefficients(
    coefficients: Vec<Vec<u8>>,
) -> Result<TransparentPolynomial, String> {
    let coeffs = coefficients
        .iter()
        .map(|bytes| deserialize_fr(bytes))
        .collect::<Result<Vec<_>, _>>()?;
    let dense = DensePolynomial::from_coefficients_vec(coeffs);
    transparent_polynomial_from_dense(&dense)
}

pub fn polynomial_commitment_hash(coefficients: &[Vec<u8>]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    for coeff in coefficients {
        hasher.update((coeff.len() as u32).to_le_bytes());
        hasher.update(coeff);
    }
    hasher.finalize().to_vec()
}

pub fn polynomial_for_node(node: &PolynomialSetProofNode) -> &TransparentPolynomial {
    match node {
        PolynomialSetProofNode::Leaf(leaf) => &leaf.set_polynomial,
        PolynomialSetProofNode::And(binary) => &binary.set_polynomial,
        PolynomialSetProofNode::Or(binary) => &binary.set_polynomial,
    }
}

pub fn multiply_polynomials(
    left: &TransparentPolynomial,
    right: &TransparentPolynomial,
) -> Result<TransparentPolynomial, String> {
    let left_dense = dense_from_transparent(left)?;
    let right_dense = dense_from_transparent(right)?;
    let degree = left_dense.coeffs.len() + right_dense.coeffs.len() - 1;
    let mut coeffs = vec![Fr::zero(); degree];

    for (i, left_coeff) in left_dense.coeffs.iter().enumerate() {
        for (j, right_coeff) in right_dense.coeffs.iter().enumerate() {
            coeffs[i + j] += *left_coeff * right_coeff;
        }
    }

    transparent_polynomial_from_dense(&DensePolynomial::from_coefficients_vec(coeffs))
}

fn transparent_polynomial_from_dense(
    poly: &DensePolynomial<Fr>,
) -> Result<TransparentPolynomial, String> {
    let coefficients = polynomial_to_coefficient_bytes(poly);
    Ok(TransparentPolynomial {
        commitment: polynomial_commitment_hash(&coefficients),
        coefficients,
    })
}

fn dense_from_transparent(poly: &TransparentPolynomial) -> Result<DensePolynomial<Fr>, String> {
    if polynomial_commitment_hash(&poly.coefficients) != poly.commitment {
        return Err("transparent polynomial commitment mismatch".to_string());
    }
    let coeffs = poly
        .coefficients
        .iter()
        .map(|bytes| deserialize_fr(bytes))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DensePolynomial::from_coefficients_vec(coeffs))
}

fn serialize_fr(value: &Fr) -> Vec<u8> {
    let mut bytes = Vec::new();
    value
        .serialize(&mut bytes)
        .expect("serializing field element should not fail");
    bytes
}

fn deserialize_fr(bytes: &[u8]) -> Result<Fr, String> {
    Fr::deserialize(bytes).map_err(|e| format!("deserialize Fr failed: {:?}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdsMode, ProofVerifier, SetProofLeaf};

    fn leaf(keyword: &str, node_name: &str, fids: &[&str]) -> PolynomialSetProofNode {
        let fids_vec = fids
            .iter()
            .map(|fid| (*fid).to_string())
            .collect::<Vec<_>>();
        PolynomialSetProofNode::Leaf(PolynomialIntersectionLeafProof {
            leaf: SetProofLeaf::new(
                keyword.to_string(),
                node_name.to_string(),
                AdsMode::Mest,
                Vec::new(),
                Vec::new(),
                fids_vec.clone(),
            ),
            set_polynomial: build_characteristic_polynomial(&fids_vec).unwrap(),
        })
    }

    #[test]
    fn test_polynomial_intersection_proof_roundtrip_and_verify() {
        let proof = build_polynomial_intersection_proof(
            "(A AND B)".to_string(),
            vec!["f2".to_string(), "f3".to_string()],
            leaf("A", "n1", &["f1", "f2", "f3"]),
            leaf("B", "n2", &["f2", "f3", "f4"]),
        )
        .unwrap();
        let encoded = encode_polynomial_intersection_proof(&proof).unwrap();
        let verifier = ProofVerifier::new(AdsMode::Mest);
        assert!(verifier.verify(&encoded, &polynomial_intersection_root_hash(&encoded)));
        assert!(verifier.verify_query_result_fids(&encoded, &["f2".to_string(), "f3".to_string()]));
    }

    #[test]
    fn test_nested_and_polynomial_intersection_proof_roundtrip_and_verify() {
        let left = PolynomialSetProofNode::And(Box::new(
            build_polynomial_intersection_node(
                leaf("A", "n1", &["f1", "f2", "f3"]),
                leaf("B", "n2", &["f2", "f3", "f4"]),
                vec!["f2".to_string(), "f3".to_string()],
            )
            .unwrap(),
        ));
        let proof = build_polynomial_intersection_proof(
            "((A AND B) AND C)".to_string(),
            vec!["f3".to_string()],
            left,
            leaf("C", "n3", &["f3", "f5"]),
        )
        .unwrap();
        let encoded = encode_polynomial_intersection_proof(&proof).unwrap();
        let verifier = ProofVerifier::new(AdsMode::Mest);
        assert!(verifier.verify(&encoded, &polynomial_intersection_root_hash(&encoded)));
        assert!(verifier.verify_query_result_fids(&encoded, &["f3".to_string()]));
    }

    #[test]
    fn test_polynomial_union_proof_roundtrip_and_verify() {
        let proof = PolynomialIntersectionAggregateProof {
            expr: "(A OR B)".to_string(),
            result_fids: vec![
                "f1".to_string(),
                "f2".to_string(),
                "f3".to_string(),
                "f4".to_string(),
            ],
            root: PolynomialSetProofNode::Or(Box::new(
                build_polynomial_union_node(
                    leaf("A", "n1", &["f1", "f2", "f3"]),
                    leaf("B", "n2", &["f2", "f3", "f4"]),
                    vec!["f2".to_string(), "f3".to_string()],
                    vec![
                        "f1".to_string(),
                        "f2".to_string(),
                        "f3".to_string(),
                        "f4".to_string(),
                    ],
                )
                .unwrap(),
            )),
        };
        let encoded = encode_polynomial_intersection_proof(&proof).unwrap();
        let verifier = ProofVerifier::new(AdsMode::Mest);
        assert!(verifier.verify(&encoded, &polynomial_intersection_root_hash(&encoded)));
        assert!(verifier.verify_query_result_fids(
            &encoded,
            &[
                "f1".to_string(),
                "f2".to_string(),
                "f3".to_string(),
                "f4".to_string(),
            ]
        ));
    }

    #[test]
    fn test_polynomial_leaf_verification_uses_leaf_ads_mode() {
        let proof = PolynomialIntersectionAggregateProof {
            expr: "A".to_string(),
            result_fids: vec!["f1".to_string()],
            root: PolynomialSetProofNode::Leaf(PolynomialIntersectionLeafProof {
                leaf: SetProofLeaf::new(
                    "A".to_string(),
                    "n1".to_string(),
                    AdsMode::Mest,
                    vec![7u8; 32],
                    vec![7u8; 32],
                    vec!["f1".to_string()],
                ),
                set_polynomial: build_characteristic_polynomial(&["f1".to_string()]).unwrap(),
            }),
        };
        let encoded = encode_polynomial_intersection_proof(&proof).unwrap();

        assert!(ProofVerifier::new(AdsMode::Mpt)
            .verify(&encoded, &polynomial_intersection_root_hash(&encoded)));
    }
}
