use crate::storager::Storager;
use common::rpc::{
    storager_service_server::StoragerService, StoragerAddRequest, StoragerAddResponse,
    StoragerDeleteRequest, StoragerDeleteResponse, StoragerQueryRequest, StoragerQueryResponse,
};
use std::time::Instant;
use tonic::{Request, Response, Status};
use ads_rust::mpt::proof::{MPTProof, compute_mpt_root};
use bincode;

#[tonic::async_trait]
impl StoragerService for Storager {
    async fn add(
        &self,
        request: Request<StoragerAddRequest>,
    ) -> Result<Response<StoragerAddResponse>, Status> {
        let req = request.into_inner();
        println!(
            "Storager received Add request: keyword={}, fid={}",
            req.keyword, req.fid
        );

        let mut ads = self.ads.write()
            .expect("Failed to acquire write lock on ads");
        
        let start = Instant::now();
        let (proof, root_hash) = ads.add(&req.keyword, &req.fid);
        let duration = start.elapsed();
        println!("[METRIC] Proof Generation (Add): {:?}", duration);

        // Debug: attempt to deserialize MPT proof and print summary for investigation
        if !proof.is_empty() {
            match bincode::deserialize::<MPTProof>(&proof) {
                Ok(mpt_proof) => {
                    println!("[DEBUG] Storager Add - deserialized MPTProof: is_exist={}, levels={}, proof_count={}",
                        mpt_proof.get_is_exist(), mpt_proof.get_levels(), mpt_proof.get_proofs().len());
                    if let Some(first) = mpt_proof.get_proofs().first() {
                        println!("[DEBUG] Storager Add - first proof type={}, value_len={}, children_hashes_nonempty={}",
                            first.proof_type,
                            first.value.len(),
                            first.children_hashes.iter().filter(|h| !h.is_empty()).count());
                    }
                }
                Err(e) => {
                    println!("[DEBUG] Storager Add - failed to deserialize MPTProof: {}", e);
                }
            }
        } else {
            println!("[DEBUG] Storager Add - proof is empty");
        }

        // Ensure returned root_hash matches the proof-derived root when possible
        let mut returned_root = root_hash.clone();
        if !proof.is_empty() {
            if let Ok(mpt_proof) = bincode::deserialize::<MPTProof>(&proof) {
                // extract value similar to verifier logic
                let value = if mpt_proof.get_is_exist() && !mpt_proof.get_proofs().is_empty() {
                    if let Some(leaf_value) = mpt_proof
                        .get_proofs()
                        .iter()
                        .find(|p| p.proof_type == 0)
                        .map(|p| String::from_utf8_lossy(&p.value).to_string())
                        .filter(|v| !v.is_empty())
                    {
                        leaf_value
                    } else {
                        mpt_proof
                            .get_proofs()
                            .iter()
                            .find(|p| !p.value.is_empty())
                            .map(|p| String::from_utf8_lossy(&p.value).to_string())
                            .unwrap_or_default()
                    }
                } else {
                    String::new()
                };

                let computed_root = compute_mpt_root(&value, &mpt_proof);
                returned_root = computed_root.to_vec();
                println!("[DEBUG] Storager Add - adjusted returned root_hash to proof-derived root: {:02x?}...", &returned_root[..8]);
            }
        }

        println!("[DEBUG] Storager Add - returning root_hash len={} bytes", returned_root.len());

        Ok(Response::new(StoragerAddResponse { proof, root_hash: returned_root }))
    }

    async fn query(
        &self,
        request: Request<StoragerQueryRequest>,
    ) -> Result<Response<StoragerQueryResponse>, Status> {
        let req = request.into_inner();
        println!("Storager received Query request: keyword={}", req.keyword);

        let ads = self.ads.read()
            .expect("Failed to acquire read lock on ads");
        
        let start = Instant::now();
        let (fids, proof) = ads.query(&req.keyword);
        let duration = start.elapsed();
        println!("[METRIC] Proof Generation (Query): {:?}", duration);

        Ok(Response::new(StoragerQueryResponse { fids, proof }))
    }

    async fn delete(
        &self,
        request: Request<StoragerDeleteRequest>,
    ) -> Result<Response<StoragerDeleteResponse>, Status> {
        let req = request.into_inner();
        println!(
            "Storager received Delete request: keyword={}, fid={}",
            req.keyword, req.fid
        );

        let mut ads = match self.ads.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                eprintln!("⚠️ Delete service: recovering from poisoned Mutex");
                poisoned.into_inner()
            }
        };
        
        let start = Instant::now();
        let (proof, root_hash) = ads.delete(&req.keyword, &req.fid);
        let duration = start.elapsed();
        println!("[METRIC] Proof Generation (Delete): {:?}", duration);

        // Debug: attempt to deserialize MPT proof and print summary for investigation
        if !proof.is_empty() {
            match bincode::deserialize::<MPTProof>(&proof) {
                Ok(mpt_proof) => {
                    println!("[DEBUG] Storager Delete - deserialized MPTProof: is_exist={}, levels={}, proof_count={}",
                        mpt_proof.get_is_exist(), mpt_proof.get_levels(), mpt_proof.get_proofs().len());
                    if let Some(first) = mpt_proof.get_proofs().first() {
                        println!("[DEBUG] Storager Delete - first proof type={}, value_len={}, children_hashes_nonempty={}",
                            first.proof_type,
                            first.value.len(),
                            first.children_hashes.iter().filter(|h| !h.is_empty()).count());
                    }
                }
                Err(e) => {
                    println!("[DEBUG] Storager Delete - failed to deserialize MPTProof: {}", e);
                }
            }
        } else {
            println!("[DEBUG] Storager Delete - proof is empty");
        }

        // Ensure returned root_hash matches the proof-derived root when possible
        let mut returned_root = root_hash.clone();
        if !proof.is_empty() {
            if let Ok(mpt_proof) = bincode::deserialize::<MPTProof>(&proof) {
                let value = if mpt_proof.get_is_exist() && !mpt_proof.get_proofs().is_empty() {
                    if let Some(leaf_value) = mpt_proof
                        .get_proofs()
                        .iter()
                        .find(|p| p.proof_type == 0)
                        .map(|p| String::from_utf8_lossy(&p.value).to_string())
                        .filter(|v| !v.is_empty())
                    {
                        leaf_value
                    } else {
                        mpt_proof
                            .get_proofs()
                            .iter()
                            .find(|p| !p.value.is_empty())
                            .map(|p| String::from_utf8_lossy(&p.value).to_string())
                            .unwrap_or_default()
                    }
                } else {
                    String::new()
                };

                let computed_root = compute_mpt_root(&value, &mpt_proof);
                returned_root = computed_root.to_vec();
                println!("[DEBUG] Storager Delete - adjusted returned root_hash to proof-derived root: {:02x?}...", &returned_root[..8]);
            }
        }

        println!("[DEBUG] Storager Delete - returning root_hash len={} bytes", returned_root.len());

        Ok(Response::new(StoragerDeleteResponse { proof, root_hash: returned_root }))
    }
}
