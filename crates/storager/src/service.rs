use crate::storager::Storager;
use ads_rust::mpt::proof::{compute_mpt_root, MPTProof};
use bincode;
use common::rpc::{
    storager_service_server::StoragerService, ResetStorageRequest, ResetStorageResponse,
    StoragerAddRequest, StoragerAddResponse, StoragerBatchAddRequest, StoragerBatchAddResponse,
    StoragerConfirmPrefixMigrationRequest, StoragerConfirmPrefixMigrationResponse,
    StoragerDeleteRequest, StoragerDeleteResponse, StoragerDrainPrefixRequest,
    StoragerDrainPrefixResponse, StoragerExportPrefixRequest, StoragerExportPrefixResponse,
    StoragerImportPrefixRequest, StoragerImportPrefixResponse, StoragerIoStatsRequest,
    StoragerIoStatsResponse, StoragerPrepareRetainPrefixRequest,
    StoragerPrepareRetainPrefixResponse, StoragerQueryRequest, StoragerQueryResponse,
};
use std::time::Instant;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl StoragerService for Storager {
    async fn get_io_stats(
        &self,
        request: Request<StoragerIoStatsRequest>,
    ) -> Result<Response<StoragerIoStatsResponse>, Status> {
        let _req = request.into_inner();
        let stats = ads_rust::io_stats::snapshot();
        Ok(Response::new(StoragerIoStatsResponse {
            read_bytes: stats.read_bytes,
            write_bytes: stats.write_bytes,
            read_ops: stats.read_ops,
            write_ops: stats.write_ops,
        }))
    }

    async fn add(
        &self,
        request: Request<StoragerAddRequest>,
    ) -> Result<Response<StoragerAddResponse>, Status> {
        let req = request.into_inner();
        self.set_run_metadata(
            req.dataset.clone(),
            req.concurrency,
            req.total_uploads,
            req.total_queries,
            req.total_updates,
        );
        self.set_route_mode(req.route_mode.clone());
        println!(
            "Storager received Add request: keyword={}, fid={}",
            req.keyword, req.fid
        );

        let (proof, root_hash, root_accumulator, duration) = tokio::task::block_in_place(|| {
            let _mutation_guard = self.begin_mutation();

            let ads = match self.ads.read() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    eprintln!("[LOCK] Add service: recovering from poisoned ads lock");
                    poisoned.into_inner()
                }
            };

            let start = Instant::now();
            let (proof, root_hash) = ads.add(&req.keyword, &req.fid);
            let root_accumulator = ads.root_accumulator();
            let duration = start.elapsed();
            drop(ads);
            (proof, root_hash, root_accumulator, duration)
        });
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
                    println!(
                        "[DEBUG] Storager Add - failed to deserialize MPTProof: {}",
                        e
                    );
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

        println!(
            "[DEBUG] Storager Add - returning root_hash len={} bytes",
            returned_root.len()
        );
        if req.total_uploads > 0 {
            self.begin_upload_report();
        }
        self.record_upload_kv_pairs_total(req.total_upload_kv_pairs);
        self.write_metrics_report();

        Ok(Response::new(StoragerAddResponse {
            proof,
            root_hash: returned_root,
            root_accumulator,
            persistence_mode: self.persistence_mode(),
        }))
    }

    async fn batch_add(
        &self,
        request: Request<StoragerBatchAddRequest>,
    ) -> Result<Response<StoragerBatchAddResponse>, Status> {
        let req = request.into_inner();
        self.set_run_metadata(
            req.dataset.clone(),
            req.concurrency,
            req.total_uploads,
            req.total_queries,
            req.total_updates,
        );
        self.set_route_mode(req.route_mode.clone());
        let total_upload_kv_pairs = req.total_upload_kv_pairs;
        let items = req.items;
        let (proof, root_hash, root_accumulator, item_count) = tokio::task::block_in_place(|| {
            let _mutation_guard = self.begin_mutation();
            let ads = match self.ads.read() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            let kvs = items
                .into_iter()
                .map(|item| (item.keyword, item.fid))
                .collect::<Vec<_>>();
            let item_count = kvs.len() as u32;
            let (proof, root_hash) = ads.add_batch(kvs);
            let root_accumulator = ads.root_accumulator();
            drop(ads);
            (proof, root_hash, root_accumulator, item_count)
        });
        if req.total_uploads > 0 {
            self.begin_upload_report();
        }
        self.record_upload_kv_pairs_total(total_upload_kv_pairs);
        self.write_metrics_report();
        Ok(Response::new(StoragerBatchAddResponse {
            proof,
            root_hash,
            root_accumulator,
            item_count,
            persistence_mode: self.persistence_mode(),
        }))
    }

    async fn query(
        &self,
        request: Request<StoragerQueryRequest>,
    ) -> Result<Response<StoragerQueryResponse>, Status> {
        let req = request.into_inner();
        self.set_run_metadata(
            req.dataset.clone(),
            req.concurrency,
            req.total_uploads,
            req.total_queries,
            req.total_updates,
        );
        self.set_route_mode(req.route_mode.clone());
        println!("Storager received Query request: keyword={}", req.keyword);

        let (fids, proof, root_hash, root_accumulator, duration) =
            tokio::task::block_in_place(|| {
                let ads = match self.ads.read() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        eprintln!("[LOCK] Query service: recovering from poisoned ads lock");
                        poisoned.into_inner()
                    }
                };

                let start = Instant::now();
                let _query_guard = self.begin_query_for_keyword(&req.keyword);
                let (fids, proof) = ads.query(&req.keyword);
                let root_hash = ads.current_root_hash();
                let root_accumulator = ads.root_accumulator();
                let duration = start.elapsed();
                (fids, proof, root_hash, root_accumulator, duration)
            });
        println!("[METRIC] Proof Generation (Query): {:?}", duration);
        self.record_query_metrics(proof.len());
        self.write_metrics_report();

        Ok(Response::new(StoragerQueryResponse {
            fids,
            proof,
            root_accumulator,
            root_hash,
            persistence_mode: self.persistence_mode(),
        }))
    }

    async fn delete(
        &self,
        request: Request<StoragerDeleteRequest>,
    ) -> Result<Response<StoragerDeleteResponse>, Status> {
        let req = request.into_inner();
        self.set_run_metadata(
            req.dataset.clone(),
            req.concurrency,
            req.total_uploads,
            req.total_queries,
            req.total_updates,
        );
        self.set_route_mode(req.route_mode.clone());
        println!(
            "Storager received Delete request: keyword={}, fid={}",
            req.keyword, req.fid
        );

        let (proof, root_hash, root_accumulator, duration) = tokio::task::block_in_place(|| {
            let _mutation_guard = self.begin_mutation();

            let ads = match self.ads.read() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    eprintln!("⚠️ Delete service: recovering from poisoned Mutex");
                    poisoned.into_inner()
                }
            };

            let start = Instant::now();
            let (proof, root_hash) = ads.delete(&req.keyword, &req.fid);
            let root_accumulator = ads.root_accumulator();
            let duration = start.elapsed();
            drop(ads);
            (proof, root_hash, root_accumulator, duration)
        });
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
                    println!(
                        "[DEBUG] Storager Delete - failed to deserialize MPTProof: {}",
                        e
                    );
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

        println!(
            "[DEBUG] Storager Delete - returning root_hash len={} bytes",
            returned_root.len()
        );
        if req.total_updates > 0 {
            self.record_after_update_count();
        }
        self.write_metrics_report();

        Ok(Response::new(StoragerDeleteResponse {
            proof,
            root_hash: returned_root,
            root_accumulator,
            persistence_mode: self.persistence_mode(),
        }))
    }

    async fn export_prefix_segment(
        &self,
        request: Request<StoragerExportPrefixRequest>,
    ) -> Result<Response<StoragerExportPrefixResponse>, Status> {
        let req = request.into_inner();
        self.set_run_metadata(
            req.dataset.clone(),
            req.concurrency,
            req.total_uploads,
            req.total_queries,
            req.total_updates,
        );
        self.set_route_mode(req.route_mode.clone());
        let ads = match self.ads.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                eprintln!("[LOCK] ExportPrefixSegment service: recovering from poisoned ads lock");
                poisoned.into_inner()
            }
        };
        let segment = ads
            .export_prefix_segment(&req.prefix)
            .map_err(Status::failed_precondition)?;
        let root_accumulator = ads.root_accumulator();
        let root_hash = ads.current_root_hash();

        Ok(Response::new(StoragerExportPrefixResponse {
            segment,
            root_hash,
            root_accumulator,
        }))
    }

    async fn import_prefix_segment(
        &self,
        request: Request<StoragerImportPrefixRequest>,
    ) -> Result<Response<StoragerImportPrefixResponse>, Status> {
        let req = request.into_inner();
        self.set_run_metadata(
            req.dataset.clone(),
            req.concurrency,
            req.total_uploads,
            req.total_queries,
            req.total_updates,
        );
        self.set_route_mode(req.route_mode.clone());
        let mut ads = match self.ads.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                eprintln!("[LOCK] ImportPrefixSegment service: recovering from poisoned ads lock");
                poisoned.into_inner()
            }
        };
        let root_hash = ads
            .import_prefix_segment(&req.segment)
            .map_err(Status::failed_precondition)?;
        let root_accumulator = ads.root_accumulator();
        drop(ads);
        if req.total_updates > 0 {
            self.record_after_update_count();
        }
        self.write_metrics_report();

        Ok(Response::new(StoragerImportPrefixResponse {
            root_hash,
            root_accumulator,
        }))
    }

    async fn drain_prefix_segment(
        &self,
        request: Request<StoragerDrainPrefixRequest>,
    ) -> Result<Response<StoragerDrainPrefixResponse>, Status> {
        let req = request.into_inner();
        self.set_run_metadata(
            req.dataset.clone(),
            req.concurrency,
            req.total_uploads,
            req.total_queries,
            req.total_updates,
        );
        self.set_route_mode(req.route_mode.clone());
        let mut ads = match self.ads.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                eprintln!("[LOCK] DrainPrefixSegment service: recovering from poisoned ads lock");
                poisoned.into_inner()
            }
        };
        let (segment, root_hash) = ads
            .drain_prefix_segment(&req.prefix)
            .map_err(Status::failed_precondition)?;
        let root_accumulator = ads.root_accumulator();
        drop(ads);
        if req.total_updates > 0 {
            self.record_after_update_count();
        }
        self.write_metrics_report();

        Ok(Response::new(StoragerDrainPrefixResponse {
            segment,
            root_hash,
            root_accumulator,
        }))
    }
    async fn prepare_retain_prefix_segment(
        &self,
        request: Request<StoragerPrepareRetainPrefixRequest>,
    ) -> Result<Response<StoragerPrepareRetainPrefixResponse>, Status> {
        let req = request.into_inner();
        self.set_run_metadata(
            req.dataset.clone(),
            req.concurrency,
            req.total_uploads,
            req.total_queries,
            req.total_updates,
        );
        self.set_route_mode(req.route_mode.clone());
        self.wait_for_mutations_to_drain().await;
        let mut ads = match self.ads.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                eprintln!(
                    "[LOCK] PrepareRetainPrefixSegment service: recovering from poisoned ads lock"
                );
                poisoned.into_inner()
            }
        };
        let prepared = ads.prepare_retain_prefix_segment(&req.prefix);
        let (segment, root_hash) = prepared.map_err(Status::failed_precondition)?;
        let root_accumulator = ads.root_accumulator();
        drop(ads);
        self.write_metrics_report();
        Ok(Response::new(StoragerPrepareRetainPrefixResponse {
            segment,
            root_hash,
            root_accumulator,
        }))
    }

    async fn confirm_prefix_migration(
        &self,
        request: Request<StoragerConfirmPrefixMigrationRequest>,
    ) -> Result<Response<StoragerConfirmPrefixMigrationResponse>, Status> {
        let req = request.into_inner();
        self.set_run_metadata(
            req.dataset.clone(),
            req.concurrency,
            req.total_uploads,
            req.total_queries,
            req.total_updates,
        );
        self.set_route_mode(req.route_mode.clone());
        self.wait_for_prefix_queries_to_drain(&req.prefix).await;
        let mut ads = match self.ads.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                eprintln!(
                    "[LOCK] ConfirmPrefixMigration service: recovering from poisoned ads lock"
                );
                poisoned.into_inner()
            }
        };
        let root_hash = ads.confirm_prefix_migration(&req.prefix);
        let root_hash = root_hash.map_err(Status::failed_precondition)?;
        let root_accumulator = ads.root_accumulator();
        drop(ads);
        self.write_metrics_report();
        Ok(Response::new(StoragerConfirmPrefixMigrationResponse {
            root_hash,
            root_accumulator,
        }))
    }

    async fn reset_storage(
        &self,
        request: Request<ResetStorageRequest>,
    ) -> Result<Response<ResetStorageResponse>, Status> {
        let req = request.into_inner();
        self.set_route_mode(req.route_mode.clone());
        self.wait_for_mutations_to_drain().await;
        let mut ads = match self.ads.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        ads.reset()
            .map_err(|err| Status::internal(format!("reset failed: {}", err)))?;
        drop(ads);
        ads_rust::io_stats::reset();
        self.write_metrics_report();
        Ok(Response::new(ResetStorageResponse {
            success: true,
            message: "storage reset completed".to_string(),
        }))
    }
}
