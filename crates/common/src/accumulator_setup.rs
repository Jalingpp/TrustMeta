use accumulator_ads::acc::setup::PRI_S;
use accumulator_ads::acc::{
    init_public_parameters, init_public_parameters_direct, Fr, G1Affine, G1Projective, G2Affine,
    G2Projective, PublicParameters,
};
use anyhow::{Context, Result};
use ark_ec::{AffineCurve, ProjectiveCurve};
use ark_ff::PrimeField;
use std::fs;
use std::path::PathBuf;

const DEFAULT_MAX_DEGREE: usize = 5000;
const PUBLIC_PARAMS_FILE_ENV: &str = "ACCUMULATOR_PUBLIC_PARAMS_FILE";
const MAX_DEGREE_ENV: &str = "ACCUMULATOR_PUBLIC_PARAMS_MAX_DEGREE";

fn resolve_max_degree() -> usize {
    std::env::var(MAX_DEGREE_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_DEGREE)
}

fn generate_public_parameters(max_degree: usize) -> PublicParameters {
    let g1 = G1Affine::prime_subgroup_generator();
    let g2 = G2Affine::prime_subgroup_generator();

    let mut g1_s_vec = Vec::with_capacity(max_degree + 1);
    let mut g2_s_vec = Vec::with_capacity(max_degree + 1);

    let mut s_power = Fr::from(1u64);
    for _ in 0..=max_degree {
        g1_s_vec.push(
            G1Projective::from(g1)
                .mul(s_power.into_repr())
                .into_affine(),
        );
        g2_s_vec.push(
            G2Projective::from(g2)
                .mul(s_power.into_repr())
                .into_affine(),
        );
        s_power *= *PRI_S;
    }

    PublicParameters {
        g1,
        g2,
        g1_s_vec,
        g2_s_vec,
    }
}

pub fn init_accumulator_public_parameters() -> Result<()> {
    if let Some(path) = std::env::var_os(PUBLIC_PARAMS_FILE_ENV) {
        let path = PathBuf::from(path);
        if path.exists() {
            return init_public_parameters(path);
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create parameters directory: {:?}", parent))?;
        }

        let params = generate_public_parameters(resolve_max_degree());
        params.save_to_file(&path)?;
        return init_public_parameters_direct(params);
    }

    #[cfg(any(test, debug_assertions))]
    {
        let params = generate_public_parameters(resolve_max_degree());
        return init_public_parameters_direct(params);
    }

    #[cfg(not(any(test, debug_assertions)))]
    {
        let params = generate_public_parameters(resolve_max_degree());
        return init_public_parameters_direct(params);
    }
}
