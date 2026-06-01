use anyhow::{Context, Result};
use ark_bls12_381::{Bls12_381 as Curve, Fq12, G1Affine, G2Affine};
use ark_ec::{AffineCurve, PairingEngine};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize, Read, SerializationError, Write};
use lazy_static::lazy_static;
use log::info;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::RwLock;

use ark_bls12_381::{Fr, G1Projective, G2Projective};
use ark_ec::ProjectiveCurve;

#[cfg(test)]
#[allow(dead_code)]
const GS_VEC_LEN: usize = 20;
#[cfg(not(test))]
#[allow(dead_code)]
const GS_VEC_LEN: usize = 5000;

// Since the project holds the private key, we expose it always
lazy_static! {
    pub static ref PRI_S: Fr = Fr::from(259535143263514268207918833918737523409u128);
}

/// Public parameters loaded from trusted setup
/// These parameters are generated through a trusted setup ceremony
/// The secret trapdoor s must be destroyed after generating these parameters
#[derive(Debug, Clone, CanonicalSerialize, CanonicalDeserialize)]
pub struct PublicParameters {
    /// Generator g1
    pub g1: G1Affine,
    /// Generator g2
    pub g2: G2Affine,
    /// Precomputed powers: g1, g1^s, g1^(s^2), ..., g1^(s^n)
    pub g1_s_vec: Vec<G1Affine>,
    /// Precomputed powers: g2, g2^s, g2^(s^2), ..., g2^(s^n)
    pub g2_s_vec: Vec<G2Affine>,
}

impl PublicParameters {
    /// Load public parameters from a file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref())
            .with_context(|| format!("Failed to open parameters file: {:?}", path.as_ref()))?;
        let mut reader = BufReader::new(file);
        let params = Self::deserialize_unchecked(&mut reader)
            .context("Failed to deserialize public parameters")?;

        info!(
            "Loaded public parameters with {} G1 powers and {} G2 powers",
            params.g1_s_vec.len(),
            params.g2_s_vec.len()
        );

        Ok(params)
    }

    /// Save public parameters to a file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let file = File::create(path.as_ref())
            .with_context(|| format!("Failed to create parameters file: {:?}", path.as_ref()))?;
        let mut writer = BufWriter::new(file);
        self.serialize_unchecked(&mut writer)
            .context("Failed to serialize public parameters")?;

        info!("Saved public parameters to {:?}", path.as_ref());
        Ok(())
    }

    /// Generate parameters using the secret trapdoor (FOR TESTING ONLY)
    /// In production, this must be done through a trusted setup ceremony
    /// and the secret s MUST be destroyed immediately after generation
    ///
    /// This function is available in both test and debug builds for development convenience.
    /// It should NEVER be used in production/release builds.
    #[cfg(any(test, debug_assertions))]
    pub fn generate_for_testing(secret_s: Fr, max_degree: usize) -> Self {
        use ark_ff::PrimeField;

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
            s_power *= secret_s;
        }

        Self {
            g1,
            g2,
            g1_s_vec,
            g2_s_vec,
        }
    }
}

lazy_static! {
    /// Global public parameters
    /// Must be initialized before use via init_public_parameters()
    static ref PUBLIC_PARAMS: RwLock<Option<PublicParameters>> = RwLock::new(None);

    // Precomputed Pairing(g1, g2)
    pub static ref E_G_G: Fq12 = Curve::pairing(
        G1Affine::prime_subgroup_generator(),
        G2Affine::prime_subgroup_generator()
    );
}

/// Initialize public parameters from a file
/// This must be called before using any accumulator operations
pub fn init_public_parameters<P: AsRef<Path>>(path: P) -> Result<()> {
    let params = PublicParameters::load_from_file(path)?;
    let mut global_params = PUBLIC_PARAMS.write().unwrap();
    *global_params = Some(params);
    info!("Public parameters initialized successfully");
    Ok(())
}

/// Initialize public parameters directly (for testing)
pub fn init_public_parameters_direct(params: PublicParameters) -> Result<()> {
    let mut global_params = PUBLIC_PARAMS.write().unwrap();
    *global_params = Some(params);
    info!("Public parameters initialized directly");
    Ok(())
}

/// Get reference to public parameters
/// Panics if parameters are not initialized
pub fn get_public_parameters() -> PublicParameters {
    PUBLIC_PARAMS
        .read()
        .unwrap()
        .as_ref()
        .expect("Public parameters not initialized. Call init_public_parameters() first.")
        .clone()
}

/// Get a specific G1 power: g1^(s^i)
pub fn get_g1s(i: usize) -> G1Affine {
    let params = get_public_parameters();
    params.g1_s_vec[i]
}

/// Get a specific G2 power: g2^(s^i)
pub fn get_g2s(i: usize) -> G2Affine {
    let params = get_public_parameters();
    params.g2_s_vec[i]
}

/// Get all G1 powers as a vector reference
pub fn get_g1s_vec() -> Vec<G1Affine> {
    let params = get_public_parameters();
    params.g1_s_vec.clone()
}

/// Get all G2 powers as a vector reference  
pub fn get_g2s_vec() -> Vec<G2Affine> {
    let params = get_public_parameters();
    params.g2_s_vec.clone()
}
