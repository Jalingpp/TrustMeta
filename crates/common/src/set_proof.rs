use crate::{AdsMode, ProofVerifier};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetProofLeaf {
    pub keyword: String,
    pub node_name: String,
    pub ads_mode: AdsMode,
    pub root_hash: Vec<u8>,
    pub ads_proof: Vec<u8>,
    pub fids: Vec<String>,
}

impl SetProofLeaf {
    pub fn new(
        keyword: String,
        node_name: String,
        ads_mode: AdsMode,
        root_hash: Vec<u8>,
        ads_proof: Vec<u8>,
        fids: Vec<String>,
    ) -> Self {
        Self {
            keyword,
            node_name,
            ads_mode,
            root_hash,
            ads_proof,
            fids,
        }
    }

    pub fn verify(&self, verifier: &ProofVerifier) -> bool {
        verifier.verify_ads_proof_for_mode(self.ads_mode, &self.ads_proof, &self.root_hash)
            && verifier.verify_query_result_fids(&self.ads_proof, &self.fids)
    }

    pub fn verify_and_collect_fids(&self, verifier: &ProofVerifier) -> Option<HashSet<String>> {
        self.verify(verifier)
            .then(|| self.fids.iter().cloned().collect())
    }
}
