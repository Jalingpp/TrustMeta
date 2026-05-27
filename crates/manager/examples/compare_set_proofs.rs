use std::time::{Duration, Instant};

use accumulator_ads::acc::{IntersectionProof, UnionProof};
use accumulator_ads::digest_set_from_set;
use accumulator_ads::DynamicAccumulator;
use ark_bls12_381::Fr;
use ark_serialize::CanonicalSerialize;
use common::{
    build_characteristic_polynomial, build_polynomial_intersection_node,
    build_polynomial_union_node, encode_polynomial_intersection_proof,
    init_accumulator_public_parameters, polynomial_intersection_root_hash, AdsMode,
    PolynomialIntersectionAggregateProof, PolynomialIntersectionLeafProof, PolynomialSetProofNode,
    ProofVerifier, SetProofLeaf,
};

#[derive(Clone, Copy)]
struct Case {
    size: usize,
    overlap_ratio: f64,
    iterations: usize,
}

#[derive(Clone)]
struct CaseData {
    intersection_strings: Vec<String>,
    union_strings: Vec<String>,
    set1_acc: Vec<Fr>,
    set2_acc: Vec<Fr>,
    intersection_acc: Vec<Fr>,
    union_acc: Vec<Fr>,
    left_leaf: PolynomialSetProofNode,
    right_leaf: PolynomialSetProofNode,
}

#[derive(Default, Clone, Copy)]
struct Measure {
    avg_gen_ms: f64,
    avg_verify_ms: f64,
    proof_bytes: usize,
}

fn main() -> anyhow::Result<()> {
    init_accumulator_public_parameters()?;

    let cases = [
        Case {
            size: 128,
            overlap_ratio: 0.25,
            iterations: 20,
        },
        Case {
            size: 512,
            overlap_ratio: 0.25,
            iterations: 10,
        },
        Case {
            size: 2048,
            overlap_ratio: 0.25,
            iterations: 4,
        },
        Case {
            size: 2048,
            overlap_ratio: 0.75,
            iterations: 4,
        },
    ];

    println!("Benchmarking set-operation proof cores in release mode");
    println!("Caveat: the accumulator implementation proves accumulator values, while the polynomial implementation serializes the full aggregate proof with explicit result_fids.");
    println!();
    println!("| op | size | overlap | impl | gen ms | verify ms | proof bytes |");
    println!("|---|---:|---:|---|---:|---:|---:|");

    for case in cases {
        let data = build_case_data(case);
        let acc_inter = bench_acc_intersection(&data, case.iterations)?;
        let poly_inter = bench_poly_intersection(&data, case.iterations)?;
        let acc_union = bench_acc_union(&data, case.iterations)?;
        let poly_union = bench_poly_union(&data, case.iterations)?;

        print_row("AND", case, "accumulator", acc_inter);
        print_row("AND", case, "polynomial", poly_inter);
        print_row("OR", case, "accumulator", acc_union);
        print_row("OR", case, "polynomial", poly_union);
    }

    Ok(())
}

fn print_row(op: &str, case: Case, implementation: &str, measure: Measure) {
    println!(
        "| {} | {} | {:.0}% | {} | {:.3} | {:.3} | {} |",
        op,
        case.size,
        case.overlap_ratio * 100.0,
        implementation,
        measure.avg_gen_ms,
        measure.avg_verify_ms,
        measure.proof_bytes
    );
}

fn build_case_data(case: Case) -> CaseData {
    let overlap = ((case.size as f64) * case.overlap_ratio).round() as usize;
    let overlap = overlap.min(case.size);
    let left_only = case.size - overlap;
    let right_only = left_only;

    let intersection_strings = (0..overlap).map(|i| format!("f{i:08}")).collect::<Vec<_>>();
    let left_only_strings = (overlap..overlap + left_only)
        .map(|i| format!("f{i:08}"))
        .collect::<Vec<_>>();
    let right_only_strings = (overlap + left_only..overlap + left_only + right_only)
        .map(|i| format!("f{i:08}"))
        .collect::<Vec<_>>();

    let mut set1_strings = intersection_strings.clone();
    set1_strings.extend(left_only_strings.clone());
    let mut set2_strings = intersection_strings.clone();
    set2_strings.extend(right_only_strings.clone());
    let mut union_strings = set1_strings.clone();
    union_strings.extend(right_only_strings);

    let intersection_ids = (0..overlap).map(|i| i as i32).collect::<Vec<_>>();
    let left_only_ids = (overlap..overlap + left_only)
        .map(|i| i as i32)
        .collect::<Vec<_>>();
    let right_only_ids = (overlap + left_only..overlap + left_only + right_only)
        .map(|i| i as i32)
        .collect::<Vec<_>>();

    let mut set1_ids = intersection_ids.clone();
    set1_ids.extend(left_only_ids);
    let mut set2_ids = intersection_ids.clone();
    set2_ids.extend(right_only_ids.clone());
    let mut union_ids = set1_ids.clone();
    union_ids.extend(right_only_ids);

    let set1_acc = digest_set_from_set(&set1_ids.into_iter().collect());
    let set2_acc = digest_set_from_set(&set2_ids.into_iter().collect());
    let intersection_acc = digest_set_from_set(&intersection_ids.into_iter().collect());
    let union_acc = digest_set_from_set(&union_ids.into_iter().collect());

    let left_leaf = build_poly_leaf("A", "n1", &set1_strings);
    let right_leaf = build_poly_leaf("B", "n2", &set2_strings);

    CaseData {
        intersection_strings,
        union_strings,
        set1_acc,
        set2_acc,
        intersection_acc,
        union_acc,
        left_leaf,
        right_leaf,
    }
}

fn build_poly_leaf(keyword: &str, node_name: &str, fids: &[String]) -> PolynomialSetProofNode {
    PolynomialSetProofNode::Leaf(PolynomialIntersectionLeafProof {
        leaf: SetProofLeaf::new(
            keyword.to_string(),
            node_name.to_string(),
            AdsMode::Mest,
            Vec::new(),
            Vec::new(),
            fids.to_vec(),
        ),
        set_polynomial: build_characteristic_polynomial(fids).expect("leaf polynomial"),
    })
}

fn bench_acc_intersection(data: &CaseData, iterations: usize) -> anyhow::Result<Measure> {
    let mut total_gen = Duration::ZERO;
    let mut total_verify = Duration::ZERO;
    let mut proof_bytes = 0usize;

    for _ in 0..iterations {
        let start = Instant::now();
        let (intersection_acc, proof) =
            IntersectionProof::new(&data.set1_acc, &data.set2_acc, &data.intersection_acc)?;
        total_gen += start.elapsed();
        proof_bytes = bincode::serialize(&proof)?.len() + serialize_g1(intersection_acc.acc_value)?;

        let acc1_value = DynamicAccumulator::calculate_commitment(&data.set1_acc);
        let acc2_value = DynamicAccumulator::calculate_commitment(&data.set2_acc);
        let start = Instant::now();
        let ok = proof.verify(acc1_value, acc2_value, intersection_acc.acc_value);
        total_verify += start.elapsed();
        anyhow::ensure!(ok, "accumulator intersection verification failed");
    }

    Ok(Measure {
        avg_gen_ms: total_gen.as_secs_f64() * 1000.0 / iterations as f64,
        avg_verify_ms: total_verify.as_secs_f64() * 1000.0 / iterations as f64,
        proof_bytes,
    })
}

fn bench_acc_union(data: &CaseData, iterations: usize) -> anyhow::Result<Measure> {
    let mut total_gen = Duration::ZERO;
    let mut total_verify = Duration::ZERO;
    let mut proof_bytes = 0usize;

    for _ in 0..iterations {
        let start = Instant::now();
        let (intersection_acc, intersection_proof) =
            IntersectionProof::new(&data.set1_acc, &data.set2_acc, &data.intersection_acc)?;
        let (union_acc, union_proof) =
            UnionProof::new(&intersection_acc, intersection_proof, &data.union_acc)?;
        total_gen += start.elapsed();
        proof_bytes = bincode::serialize(&union_proof)?.len() + serialize_g1(union_acc.acc_value)?;

        let acc1_value = DynamicAccumulator::calculate_commitment(&data.set1_acc);
        let acc2_value = DynamicAccumulator::calculate_commitment(&data.set2_acc);
        let start = Instant::now();
        let ok = union_proof.verify(acc1_value, acc2_value, union_acc.acc_value);
        total_verify += start.elapsed();
        anyhow::ensure!(ok, "accumulator union verification failed");
    }

    Ok(Measure {
        avg_gen_ms: total_gen.as_secs_f64() * 1000.0 / iterations as f64,
        avg_verify_ms: total_verify.as_secs_f64() * 1000.0 / iterations as f64,
        proof_bytes,
    })
}

fn bench_poly_intersection(data: &CaseData, iterations: usize) -> anyhow::Result<Measure> {
    let verifier = ProofVerifier::new(AdsMode::Mest);
    let mut total_gen = Duration::ZERO;
    let mut total_verify = Duration::ZERO;
    let mut proof_bytes = 0usize;

    for _ in 0..iterations {
        let start = Instant::now();
        let node = build_polynomial_intersection_node(
            data.left_leaf.clone(),
            data.right_leaf.clone(),
            data.intersection_strings.clone(),
        )
        .map_err(anyhow::Error::msg)?;
        let aggregate = PolynomialIntersectionAggregateProof {
            expr: "(A AND B)".to_string(),
            result_fids: data.intersection_strings.clone(),
            root: PolynomialSetProofNode::And(Box::new(node)),
        };
        let encoded =
            encode_polynomial_intersection_proof(&aggregate).map_err(anyhow::Error::msg)?;
        let root_hash = polynomial_intersection_root_hash(&encoded);
        total_gen += start.elapsed();
        proof_bytes = encoded.len();

        let start = Instant::now();
        let ok = verifier.verify(&encoded, &root_hash)
            && verifier.verify_query_result_fids(&encoded, &data.intersection_strings);
        total_verify += start.elapsed();
        anyhow::ensure!(ok, "polynomial intersection verification failed");
    }

    Ok(Measure {
        avg_gen_ms: total_gen.as_secs_f64() * 1000.0 / iterations as f64,
        avg_verify_ms: total_verify.as_secs_f64() * 1000.0 / iterations as f64,
        proof_bytes,
    })
}

fn bench_poly_union(data: &CaseData, iterations: usize) -> anyhow::Result<Measure> {
    let verifier = ProofVerifier::new(AdsMode::Mest);
    let mut total_gen = Duration::ZERO;
    let mut total_verify = Duration::ZERO;
    let mut proof_bytes = 0usize;

    for _ in 0..iterations {
        let start = Instant::now();
        let node = build_polynomial_union_node(
            data.left_leaf.clone(),
            data.right_leaf.clone(),
            data.intersection_strings.clone(),
            data.union_strings.clone(),
        )
        .map_err(anyhow::Error::msg)?;
        let aggregate = PolynomialIntersectionAggregateProof {
            expr: "(A OR B)".to_string(),
            result_fids: data.union_strings.clone(),
            root: PolynomialSetProofNode::Or(Box::new(node)),
        };
        let encoded =
            encode_polynomial_intersection_proof(&aggregate).map_err(anyhow::Error::msg)?;
        let root_hash = polynomial_intersection_root_hash(&encoded);
        total_gen += start.elapsed();
        proof_bytes = encoded.len();

        let start = Instant::now();
        let ok = verifier.verify(&encoded, &root_hash)
            && verifier.verify_query_result_fids(&encoded, &data.union_strings);
        total_verify += start.elapsed();
        anyhow::ensure!(ok, "polynomial union verification failed");
    }

    Ok(Measure {
        avg_gen_ms: total_gen.as_secs_f64() * 1000.0 / iterations as f64,
        avg_verify_ms: total_verify.as_secs_f64() * 1000.0 / iterations as f64,
        proof_bytes,
    })
}

fn serialize_g1(value: ark_bls12_381::G1Affine) -> anyhow::Result<usize> {
    let mut bytes = Vec::new();
    value.serialize(&mut bytes)?;
    Ok(bytes.len())
}
