use crate::core::{Manager, PendingOperation};
use crate::core::manager::ProcessIoStats;
use accumulator_ads::{digest_set_from_set, Fr, IntersectionProof, Set, UnionProof};
use common::parse_boolean_expr;
use common::rpc::{
    manager_service_server::ManagerService, AddRequest, AddResponse, BatchAddRequest,
    BatchAddResponse, DeleteRequest, DeleteResponse, QueryRequest, QueryResponse,
    ResetStorageRequest, ResetSystemRequest, ResetSystemResponse, StoragerAddRequest,
    StoragerBatchAddItem, StoragerBatchAddRequest, StoragerConfirmPrefixMigrationRequest,
    StoragerDeleteRequest, StoragerImportPrefixRequest, StoragerIoStatsRequest,
    StoragerPrepareRetainPrefixRequest, StoragerQueryRequest, UpdateRequest, UpdateResponse,
};
use common::{
    build_characteristic_polynomial, build_polynomial_intersection_node,
    build_polynomial_union_node, encode_accumulator_set_operation_proof,
    encode_polynomial_intersection_proof, polynomial_intersection_root_hash,
    AccumulatorIntersectionNodeProof, AccumulatorSetOperationAggregateProof,
    AccumulatorSetOperationLeafProof, AccumulatorSetOperationProofNode, AccumulatorUnionNodeProof,
    AdsMode, BooleanExpr, PolynomialIntersectionAggregateProof, PolynomialIntersectionLeafProof,
    PolynomialSetProofNode, SetProofLeaf, SetProofMode,
};
use futures::future::join_all;
use futures::stream::{self, StreamExt};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use tonic::{Request, Response, Status};

#[derive(Clone)]
struct KeywordQueryLeafData {
    keyword: String,
    fids: Vec<String>,
    proof: Vec<u8>,
    node_name: String,
    ads_mode: AdsMode,
    root_hash: Vec<u8>,
}

fn sorted_vec_from_set(values: &HashSet<String>) -> Vec<String> {
    let mut items: Vec<String> = values.iter().cloned().collect();
    items.sort();
    items
}

fn set_from_vec(values: Vec<String>) -> HashSet<String> {
    values.into_iter().collect()
}

fn digest_set_from_strings(values: &[String]) -> Vec<Fr> {
    let set = Set::from_vec(values.to_vec());
    digest_set_from_set(&set)
}

fn build_set_proof_leaf(leaf: &KeywordQueryLeafData) -> SetProofLeaf {
    SetProofLeaf::new(
        leaf.keyword.clone(),
        leaf.node_name.clone(),
        leaf.ads_mode,
        leaf.root_hash.clone(),
        leaf.proof.clone(),
        leaf.fids.clone(),
    )
}

#[allow(dead_code)]
fn build_polynomial_intersection_leaf(
    leaf: &KeywordQueryLeafData,
) -> Result<PolynomialIntersectionLeafProof, Status> {
    Ok(PolynomialIntersectionLeafProof {
        leaf: build_set_proof_leaf(leaf),
        set_polynomial: build_characteristic_polynomial(&leaf.fids).map_err(Status::internal)?,
    })
}

fn build_accumulator_leaf(leaf: &KeywordQueryLeafData) -> AccumulatorSetOperationLeafProof {
    AccumulatorSetOperationLeafProof {
        leaf: build_set_proof_leaf(leaf),
    }
}

#[allow(dead_code)]
fn build_polynomial_proof_tree(
    expr: &BooleanExpr,
    leaf_data: &HashMap<String, KeywordQueryLeafData>,
) -> Result<(PolynomialSetProofNode, HashSet<String>), Status> {
    match expr {
        BooleanExpr::Keyword(keyword) => {
            let leaf = leaf_data.get(keyword).ok_or_else(|| {
                Status::internal(format!("Missing query leaf data for {}", keyword))
            })?;
            Ok((
                PolynomialSetProofNode::Leaf(build_polynomial_intersection_leaf(leaf)?),
                set_from_vec(leaf.fids.clone()),
            ))
        }
        BooleanExpr::And(left, right) => {
            let (left_node, left_set) = build_polynomial_proof_tree(left, leaf_data)?;
            let (right_node, right_set) = build_polynomial_proof_tree(right, leaf_data)?;
            let intersection: HashSet<String> =
                left_set.intersection(&right_set).cloned().collect();
            let node = PolynomialSetProofNode::And(Box::new(
                build_polynomial_intersection_node(
                    left_node,
                    right_node,
                    sorted_vec_from_set(&intersection),
                )
                .map_err(Status::internal)?,
            ));
            Ok((node, intersection))
        }
        BooleanExpr::Or(left, right) => {
            let (left_node, left_set) = build_polynomial_proof_tree(left, leaf_data)?;
            let (right_node, right_set) = build_polynomial_proof_tree(right, leaf_data)?;
            let intersection: HashSet<String> =
                left_set.intersection(&right_set).cloned().collect();
            let union: HashSet<String> = left_set.union(&right_set).cloned().collect();
            let node = PolynomialSetProofNode::Or(Box::new(
                build_polynomial_union_node(
                    left_node,
                    right_node,
                    sorted_vec_from_set(&intersection),
                    sorted_vec_from_set(&union),
                )
                .map_err(Status::internal)?,
            ));
            Ok((node, union))
        }
        BooleanExpr::Not(_) => Err(Status::invalid_argument(
            "NOT boolean queries are not supported by the polynomial proof layer",
        )),
    }
}

fn build_accumulator_proof_tree(
    expr: &BooleanExpr,
    leaf_data: &HashMap<String, KeywordQueryLeafData>,
) -> Result<(AccumulatorSetOperationProofNode, HashSet<String>), Status> {
    match expr {
        BooleanExpr::Keyword(keyword) => {
            let leaf = leaf_data.get(keyword).ok_or_else(|| {
                Status::internal(format!("Missing query leaf data for {}", keyword))
            })?;
            Ok((
                AccumulatorSetOperationProofNode::Leaf(build_accumulator_leaf(leaf)),
                set_from_vec(leaf.fids.clone()),
            ))
        }
        BooleanExpr::And(left, right) => {
            let (left_node, left_set) = build_accumulator_proof_tree(left, leaf_data)?;
            let (right_node, right_set) = build_accumulator_proof_tree(right, leaf_data)?;
            let intersection: HashSet<String> =
                left_set.intersection(&right_set).cloned().collect();
            let left_values = sorted_vec_from_set(&left_set);
            let right_values = sorted_vec_from_set(&right_set);
            let result_fids = sorted_vec_from_set(&intersection);
            let left_digest = digest_set_from_strings(&left_values);
            let right_digest = digest_set_from_strings(&right_values);
            let intersection_digest = digest_set_from_strings(&result_fids);
            let (_, proof) =
                IntersectionProof::new(&left_digest, &right_digest, &intersection_digest)
                    .map_err(|e| Status::internal(e.to_string()))?;
            Ok((
                AccumulatorSetOperationProofNode::And(Box::new(AccumulatorIntersectionNodeProof {
                    left: left_node,
                    right: right_node,
                    result_fids,
                    proof,
                })),
                intersection,
            ))
        }
        BooleanExpr::Or(left, right) => {
            let (left_node, left_set) = build_accumulator_proof_tree(left, leaf_data)?;
            let (right_node, right_set) = build_accumulator_proof_tree(right, leaf_data)?;
            let intersection: HashSet<String> =
                left_set.intersection(&right_set).cloned().collect();
            let union: HashSet<String> = left_set.union(&right_set).cloned().collect();
            let left_values = sorted_vec_from_set(&left_set);
            let right_values = sorted_vec_from_set(&right_set);
            let intersection_values = sorted_vec_from_set(&intersection);
            let result_fids = sorted_vec_from_set(&union);
            let left_digest = digest_set_from_strings(&left_values);
            let right_digest = digest_set_from_strings(&right_values);
            let intersection_digest = digest_set_from_strings(&intersection_values);
            let union_digest = digest_set_from_strings(&result_fids);
            let (intersection_acc, intersection_proof) =
                IntersectionProof::new(&left_digest, &right_digest, &intersection_digest)
                    .map_err(|e| Status::internal(e.to_string()))?;
            let (_, proof) = UnionProof::new(&intersection_acc, intersection_proof, &union_digest)
                .map_err(|e| Status::internal(e.to_string()))?;
            Ok((
                AccumulatorSetOperationProofNode::Or(Box::new(AccumulatorUnionNodeProof {
                    left: left_node,
                    right: right_node,
                    result_fids,
                    proof,
                })),
                union,
            ))
        }
        BooleanExpr::Not(_) => Err(Status::invalid_argument(
            "NOT boolean queries are not supported by the accumulator proof layer",
        )),
    }
}

#[tonic::async_trait]
impl ManagerService for Manager {
    async fn add(&self, request: Request<AddRequest>) -> Result<Response<AddResponse>, Status> {
        let _gate = self.reset_lock.clone().read_owned().await;
        let req = request.into_inner();
        let dataset = req.dataset.clone();
        let concurrency = req.concurrency;
        let total_uploads = req.total_uploads;
        let total_queries = req.total_queries;
        let total_updates = req.total_updates;
        self.set_run_metadata(
            dataset.clone(),
            concurrency,
            total_uploads,
            total_queries,
            total_updates,
        );
        let route_mode = self.route_mode().as_str().to_string();
        println!("Manager received Add request for fid: {}", req.fid);

        // Deduplicate keywords to avoid adding the same element twice
        let unique_keywords: HashSet<String> = req.keywords.into_iter().collect();
        let keyword_count = unique_keywords.len();

        if keyword_count == 0 {
            let persistence_mode = self.persistence_mode_snapshot();
            return Ok(Response::new(AddResponse {
                success: false,
                message: "No keywords provided".to_string(),
                combined_proof: vec![],
                combined_root_hash: vec![],
                combined_root_accumulator: vec![],
                route_mode,
                persistence_mode,
            }));
        }

        println!("  Processing {} unique keyword(s)", keyword_count);

        let mut ordered_keywords: Vec<String> = unique_keywords.into_iter().collect();
        ordered_keywords.sort();

        let mut combined_proof = Vec::new();
        let mut combined_root_hash = Vec::new();
        let mut combined_root_accumulator = Vec::new();
        let mut migration_happened = false;

        let start = Instant::now();
        for keyword in ordered_keywords {
            // Check if keyword is in active migration - buffer operation if so
            if self.active_prefix_migration_for_keyword(&keyword).is_some() {
                let operation = PendingOperation::Add {
                    keyword: keyword.clone(),
                    fid: req.fid.clone(),
                };
                if let Err(e) = self.buffer_operation_during_migration(operation).await {
                    println!("鈿狅笍  Failed to buffer Add operation: {}", e);
                }
                println!(
                    "馃摝 Buffered Add operation for keyword: {} during migration",
                    keyword
                );
                continue;
            }

            let route = self
                .write_route_keyword(&keyword)
                .ok_or_else(|| Status::internal("No storager available for keyword"))?;

            if self.active_prefix_migration_for_keyword(&keyword).is_some() {
                let operation = PendingOperation::Add {
                    keyword: keyword.clone(),
                    fid: req.fid.clone(),
                };
                if let Err(e) = self.buffer_operation_during_migration(operation).await {
                    println!("鈿狅笍  Failed to buffer Add operation: {}", e);
                }
                println!(
                    "馃摝 Buffered Add operation for keyword: {} during migration",
                    keyword
                );
                continue;
            }

            let mut client = self
                .get_storager_client(&route.addr)
                .await
                .map_err(|e| Status::internal(format!("Failed to connect: {}", e)))?;

            let storager_req = StoragerAddRequest {
                keyword: keyword.clone(),
                fid: req.fid.clone(),
                total_upload_kv_pairs: req.total_upload_kv_pairs,
                route_mode: route_mode.clone(),
                dataset: dataset.clone(),
                concurrency,
                total_uploads,
                total_queries,
                total_updates,
            };

            let response = client
                .add(storager_req)
                .await
                .map_err(|e| Status::internal(format!("Storager Add failed: {}", e)))?;
            let resp = response.into_inner();
            self.record_persistence_mode(&resp.persistence_mode);

            let root_summary =
                self.root_summary_for_values(&resp.root_hash, &resp.root_accumulator);
            self.update_root_hash(route.node_name.clone(), resp.root_hash.clone());
            self.update_root_accumulator(route.node_name.clone(), resp.root_accumulator.clone());

            let start = Instant::now();
            let verified = self
                .verify_proof_blocking(resp.proof.clone(), resp.root_hash.clone())
                .await
                .map_err(Status::internal)?;
            let duration = start.elapsed();
            println!("[METRIC] Proof Verification (Add): {:?}", duration);

            if !verified {
                return Err(Status::internal(format!(
                    "Add proof verification failed for keyword: {}",
                    keyword
                )));
            }

            if let Some(split_plan) =
                self.record_prefix_insert(&keyword, &route.prefix, &route.node_name, root_summary)
            {
                migration_happened = true;
                self.schedule_split_migration(split_plan);
            }
            self.record_upload_prefix_import(&route.node_name, &route.prefix, 1);

            combined_proof = resp.proof;
            combined_root_hash = resp.root_hash;
            combined_root_accumulator = resp.root_accumulator;
        }
        let duration = start.elapsed();
        println!(
            "[METRIC] Sequential Add RPCs ({} items): {:?}",
            keyword_count, duration
        );

        if migration_happened {
            combined_proof.clear();
        }

        self.write_run_report();
        self.write_upload_prefix_import_report();

        let persistence_mode = self.persistence_mode_snapshot();
        Ok(Response::new(AddResponse {
            success: true,
            message: "Add completed successfully".to_string(),
            combined_proof,
            combined_root_hash,
            combined_root_accumulator,
            route_mode,
            persistence_mode,
        }))
    }

    async fn batch_add(
        &self,
        request: Request<BatchAddRequest>,
    ) -> Result<Response<BatchAddResponse>, Status> {
        let _gate = self.reset_lock.clone().read_owned().await;
        let req = request.into_inner();
        let dataset = req.dataset.clone();
        let concurrency = req.concurrency;
        let total_uploads = req.total_uploads;
        let total_queries = req.total_queries;
        let total_updates = req.total_updates;
        self.set_run_metadata(
            dataset.clone(),
            concurrency,
            total_uploads,
            total_queries,
            total_updates,
        );
        let route_mode = self.route_mode().as_str().to_string();
        let total_upload_kv_pairs = req.total_upload_kv_pairs;
        let record_count = req.records.len() as u32;
        let mut keyword_pair_count = 0u32;
        let mut pending_items = Vec::new();

        for record in req.records {
            let unique_keywords: HashSet<String> = record.keywords.into_iter().collect();
            for keyword in unique_keywords {
                pending_items.push((keyword, record.fid.clone()));
                keyword_pair_count += 1;
            }
        }

        loop {
            let mut prefix_counts: HashMap<String, usize> = HashMap::new();
            for (keyword, _) in &pending_items {
                let route = self
                    .write_route_keyword(keyword)
                    .ok_or_else(|| Status::internal(String::new()))?;
                *prefix_counts.entry(route.prefix).or_insert(0) += 1;
            }

            let pre_splits = self.presplit_empty_prefixes(&prefix_counts);
            if pre_splits.is_empty() {
                break;
            }

            for split_plan in pre_splits {
                println!(
                    "[BATCH] pre-splitting empty prefix '{}' before batch load",
                    split_plan.parent_prefix
                );
                for child in &split_plan.children {
                    self.update_prefix_summary(&child.prefix, Vec::new());
                }
            }
        }

        let mut grouped: HashMap<(String, String), (String, Vec<StoragerBatchAddItem>)> =
            HashMap::new();
        for (keyword, fid) in pending_items {
            if self.active_prefix_migration_for_keyword(&keyword).is_some() {
                let operation = PendingOperation::Add {
                    keyword: keyword.clone(),
                    fid: fid.clone(),
                };
                if let Err(e) = self.buffer_operation_during_migration(operation).await {
                    println!("鈿狅笍  Failed to buffer Add operation: {}", e);
                    continue;
                }
                println!(
                    "馃摝 Buffered Add operation for keyword: {} during migration",
                    keyword
                );
                continue;
            }
            let route = self
                .write_route_keyword(&keyword)
                .ok_or_else(|| Status::internal(String::new()))?;
            let entry = grouped
                .entry((route.node_name.clone(), route.prefix.clone()))
                .or_insert_with(|| (route.addr.clone(), Vec::new()));
            entry.1.push(StoragerBatchAddItem { keyword, fid });
        }
        let batch_results = join_all(grouped.into_iter().map(
            |((node_name, prefix), (addr, items))| {
                let manager = self.clone();
                let route_mode = route_mode.clone();
                let dataset = dataset.clone();
                async move {
                    let mut client = manager
                        .get_storager_client(&addr)
                        .await
                        .map_err(|e| Status::internal(e.to_string()))?;
                    let resp = client
                        .batch_add(StoragerBatchAddRequest {
                            items: items.clone(),
                            total_upload_kv_pairs,
                            route_mode: route_mode.clone(),
                            dataset: dataset.clone(),
                            concurrency,
                            total_uploads,
                            total_queries,
                            total_updates,
                        })
                        .await
                        .map_err(|e| Status::internal(e.to_string()))?
                        .into_inner();
                    Ok::<_, Status>((node_name, prefix, items, resp))
                }
            },
        ))
        .await;

        for result in batch_results {
            let (node_name, prefix, items, resp) = result?;
            let item_count = items.len() as u64;
            self.record_persistence_mode(&resp.persistence_mode);
            self.update_root_hash(node_name.clone(), resp.root_hash.clone());
            self.update_root_accumulator(node_name.clone(), resp.root_accumulator.clone());
            let root_summary =
                self.root_summary_for_values(&resp.root_hash, &resp.root_accumulator);
            for item in items {
                if let Some(split_plan) = self.record_prefix_insert(
                    &item.keyword,
                    &prefix,
                    &node_name,
                    root_summary.clone(),
                ) {
                    self.schedule_split_migration(split_plan);
                }
            }
            self.update_prefix_summary(&prefix, root_summary);
            self.record_upload_prefix_import(&node_name, &prefix, item_count);
        }
        self.write_run_report();
        self.write_upload_prefix_import_report();
        let persistence_mode = self.persistence_mode_snapshot();
        Ok(Response::new(BatchAddResponse {
            success: true,
            message: String::new(),
            record_count,
            keyword_pair_count,
            route_mode,
            persistence_mode,
        }))
    }

    async fn query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let _gate = self.reset_lock.clone().read_owned().await;
        let req = request.into_inner();
        let dataset = req.dataset.clone();
        let concurrency = req.concurrency;
        let total_uploads = req.total_uploads;
        let total_queries = req.total_queries;
        let total_updates = req.total_updates;
        self.set_run_metadata(
            dataset.clone(),
            concurrency,
            total_uploads,
            total_queries,
            total_updates,
        );
        println!("Manager received Query request");

        match req.query_type {
            Some(common::rpc::query_request::QueryType::Keyword(keyword)) => {
                // 閸楁洖鍙ч柨顔跨槤閺屻儴顕?
                self.query_single_keyword(&keyword).await
            }
            Some(common::rpc::query_request::QueryType::BooleanFunction(func)) => {
                // 鐢啫鐨甸崙鑺ユ殶閺屻儴顕?
                self.query_boolean_function(&func).await
            }
            None => Err(Status::invalid_argument("No query type specified")),
        }
    }

    async fn delete(
        &self,
        request: Request<DeleteRequest>,
    ) -> Result<Response<DeleteResponse>, Status> {
        let _gate = self.reset_lock.clone().read_owned().await;
        let req = request.into_inner();
        let dataset = req.dataset.clone();
        let concurrency = req.concurrency;
        let total_uploads = req.total_uploads;
        let total_queries = req.total_queries;
        let total_updates = req.total_updates;
        self.set_run_metadata(
            dataset.clone(),
            concurrency,
            total_uploads,
            total_queries,
            total_updates,
        );
        let route_mode = self.route_mode().as_str().to_string();
        println!("Manager received Delete request for fid: {}", req.fid);

        // Deduplicate keywords to avoid deleting the same element twice
        let unique_keywords: HashSet<String> = req.keywords.into_iter().collect();
        let keyword_count = unique_keywords.len();

        if keyword_count == 0 {
            let persistence_mode = self.persistence_mode_snapshot();
            return Ok(Response::new(DeleteResponse {
                success: false,
                message: "No keywords provided".to_string(),
                combined_proof: vec![],
                combined_root_hash: vec![],
                combined_root_accumulator: vec![],
                route_mode,
                persistence_mode,
            }));
        }

        println!("  Processing {} unique keyword(s)", keyword_count);

        // 楠炴儼顢戞径鍕倞閹碘偓閺堝鍙ч柨顔跨槤
        let max_inflight = self.max_inflight_subrequests();
        let results = stream::iter(unique_keywords.into_iter().map(|keyword| {
            let manager = self.clone();
            let fid = req.fid.clone();
            let route_mode = route_mode.clone();
            let dataset = dataset.clone();
            async move {
                // Check if keyword is in active migration - buffer operation if so
                if manager
                    .active_prefix_migration_for_keyword(&keyword)
                    .is_some()
                {
                    let operation = PendingOperation::Delete {
                        keyword: keyword.clone(),
                        fid: fid.clone(),
                    };
                    if let Err(e) = manager.buffer_operation_during_migration(operation).await {
                        println!("鈿狅笍  Failed to buffer Delete operation: {}", e);
                        return Err(Status::internal(format!(
                            "Failed to buffer operation: {}",
                            e
                        )));
                    }
                    println!(
                        "馃摝 Buffered Delete operation for keyword: {} during migration",
                        keyword
                    );
                    return Ok((
                        keyword,
                        String::new(),
                        String::new(),
                        vec![],
                        vec![],
                        vec![],
                        String::new(),
                    ));
                }

                let route = manager
                    .write_route_keyword(&keyword)
                    .ok_or_else(|| Status::internal("No storager available"))?;

                if manager
                    .active_prefix_migration_for_keyword(&keyword)
                    .is_some()
                {
                    let operation = PendingOperation::Delete {
                        keyword: keyword.clone(),
                        fid: fid.clone(),
                    };
                    if let Err(e) = manager.buffer_operation_during_migration(operation).await {
                        println!("鈿狅笍  Failed to buffer Delete operation: {}", e);
                        return Err(Status::internal(format!(
                            "Failed to buffer operation: {}",
                            e
                        )));
                    }
                    println!(
                        "馃摝 Buffered Delete operation for keyword: {} during migration",
                        keyword
                    );
                    return Ok((
                        keyword,
                        String::new(),
                        String::new(),
                        vec![],
                        vec![],
                        vec![],
                        String::new(),
                    ));
                }

                // 娴ｈ法鏁ゆ潻鐐村复濮圭姾骞忛崣鏍ь吂閹撮顏?
                let _subrequest_permit = manager
                    .acquire_subrequest_permits(&route.addr)
                    .await
                    .map_err(Status::internal)?;
                let mut client = manager
                    .get_storager_client(&route.addr)
                    .await
                    .map_err(|e| {
                        Status::internal(format!("Failed to connect to storager: {}", e))
                    })?;

                let storager_req = StoragerDeleteRequest {
                    keyword: keyword.clone(),
                    fid,
                    route_mode: route_mode.clone(),
                    dataset: dataset.clone(),
                    concurrency,
                    total_uploads,
                    total_queries,
                    total_updates,
                };

                let response = client
                    .delete(storager_req)
                    .await
                    .map_err(|e| Status::internal(format!("Storager Delete failed: {}", e)))?;

                let resp = response.into_inner();

                // Verify proof with returned root hash
                let start = Instant::now();
                let verified = manager
                    .verify_proof_blocking(resp.proof.clone(), resp.root_hash.clone())
                    .await
                    .map_err(Status::internal)?;
                let duration = start.elapsed();
                println!("[METRIC] Proof Verification (Delete): {:?}", duration);

                if verified {
                    Ok((
                        keyword,
                        route.node_name,
                        route.prefix,
                        resp.proof,
                        resp.root_hash,
                        resp.root_accumulator,
                        resp.persistence_mode,
                    ))
                } else {
                    Err(Status::internal(format!(
                        "Proof verification failed for keyword: {}",
                        keyword
                    )))
                }
            }
        }))
        .buffer_unordered(max_inflight)
        .collect::<Vec<_>>()
        .await;

        // Collect all proofs and root hashes
        let mut proofs = Vec::new();
        let mut root_hashes = Vec::new();
        let mut root_accumulators = Vec::new();
        let mut storager_current_roots: HashMap<String, (Vec<u8>, Vec<u8>)> = HashMap::new();
        let mut persistence_modes = Vec::new();

        for result in results {
            match result {
                Ok((keyword, node_name, prefix, proof, root_hash, root_accumulator, persistence_mode)) => {
                    let root_summary = self.root_summary_for_values(&root_hash, &root_accumulator);
                    storager_current_roots
                        .insert(node_name, (root_hash.clone(), root_accumulator.clone()));
                    self.record_prefix_delete(&keyword, &prefix, root_summary);
                    proofs.push(proof);
                    root_hashes.push(root_hash);
                    root_accumulators.push(root_accumulator);
                    persistence_modes.push(persistence_mode);
                }
                Err(e) => return Err(e),
            }
        }

        self.record_persistence_modes(persistence_modes);

        // Commit all root hash updates atomically
        self.apply_root_state_updates(storager_current_roots.into_iter().map(
            |(node_name, (final_root, final_root_accumulator))| {
                (node_name, final_root, final_root_accumulator)
            },
        ));

        // 閸氬牆鑻熼幍鈧張澶庣槈閺?        println!("棣冩敵 Delete proof閸氬牆鑻? {} proofs", proofs.len());
        // Select representative proof/root aligned by index: prefer last non-empty root
        let rep_index = root_hashes.iter().rposition(|h| !h.is_empty()).unwrap_or(0);
        let combined_proof = proofs.get(rep_index).cloned().unwrap_or_default();
        let combined_root_hash = root_hashes.get(rep_index).cloned().unwrap_or_default();
        let combined_root_accumulator = root_accumulators
            .get(rep_index)
            .cloned()
            .unwrap_or_default();

        println!("閴?Delete combined_proof: {} bytes", combined_proof.len());

        let persistence_mode = self.persistence_mode_snapshot();
        Ok(Response::new(DeleteResponse {
            success: true,
            message: "Delete operation completed successfully".to_string(),
            combined_proof,
            combined_root_hash,
            combined_root_accumulator,
            route_mode,
            persistence_mode,
        }))
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        let _gate = self.reset_lock.clone().read_owned().await;
        let req = request.into_inner();
        let dataset = req.dataset.clone();
        let concurrency = req.concurrency;
        let total_uploads = req.total_uploads;
        let total_queries = req.total_queries;
        let total_updates = req.total_updates;
        self.set_run_metadata(
            dataset.clone(),
            concurrency,
            total_uploads,
            total_queries,
            total_updates,
        );
        let route_mode = self.route_mode().as_str().to_string();
        println!("Manager received Update request for fid: {}", req.fid);

        // Deduplicate old and new keywords
        let unique_old_keywords: HashSet<String> = req.old_keywords.clone().into_iter().collect();
        let unique_new_keywords: HashSet<String> = req.new_keywords.clone().into_iter().collect();

        println!(
            "  Deleting {} unique old keyword(s)",
            unique_old_keywords.len()
        );
        println!(
            "  Adding {} unique new keyword(s)",
            unique_new_keywords.len()
        );

        if unique_old_keywords.is_empty() && unique_new_keywords.is_empty() {
            let persistence_mode = self.persistence_mode_snapshot();
            return Ok(Response::new(UpdateResponse {
                success: true,
                message: "No keywords to update".to_string(),
                combined_proof: Vec::new(),
                combined_root_hash: Vec::new(),
                combined_root_accumulator: Vec::new(),
                route_mode,
                persistence_mode,
            }));
        }

        // Phase 1: Delete old keywords (Parallel)
        let max_inflight = self.max_inflight_subrequests();
        let delete_results = stream::iter(unique_old_keywords.into_iter().map(|keyword| {
            let manager = self.clone();
            let fid = req.fid.clone();
            let route_mode = route_mode.clone();
            let dataset = dataset.clone();
            async move {
                // Check if keyword is in active migration - buffer operation if so
                if manager
                    .active_prefix_migration_for_keyword(&keyword)
                    .is_some()
                {
                    let operation = PendingOperation::Delete {
                        keyword: keyword.clone(),
                        fid: fid.clone(),
                    };
                    if let Err(e) = manager.buffer_operation_during_migration(operation).await {
                        println!("鈿狅笍  Failed to buffer Delete operation: {}", e);
                        return Err(Status::internal(format!(
                            "Failed to buffer operation: {}",
                            e
                        )));
                    }
                    println!(
                        "馃摝 Buffered Delete operation for keyword: {} during migration",
                        keyword
                    );
                    return Ok((
                        keyword,
                        String::new(),
                        String::new(),
                        String::new(),
                        vec![],
                        vec![],
                        vec![],
                        vec![],
                        String::new(),
                    ));
                }

                let route = manager
                    .write_route_keyword(&keyword)
                    .ok_or_else(|| Status::internal("No storager available"))?;

                if manager
                    .active_prefix_migration_for_keyword(&keyword)
                    .is_some()
                {
                    let operation = PendingOperation::Delete {
                        keyword: keyword.clone(),
                        fid: fid.clone(),
                    };
                    if let Err(e) = manager.buffer_operation_during_migration(operation).await {
                        println!("鈿狅笍  Failed to buffer Delete operation: {}", e);
                        return Err(Status::internal(format!(
                            "Failed to buffer operation: {}",
                            e
                        )));
                    }
                    println!(
                        "馃摝 Buffered Delete operation for keyword: {} during migration",
                        keyword
                    );
                    return Ok((
                        keyword,
                        String::new(),
                        String::new(),
                        String::new(),
                        vec![],
                        vec![],
                        vec![],
                        vec![],
                        String::new(),
                    ));
                }

                let _subrequest_permit = manager
                    .acquire_subrequest_permits(&route.addr)
                    .await
                    .map_err(Status::internal)?;
                let old_root_hash = manager
                    .root_hashes
                    .read()
                    .expect("Failed to acquire read lock on root_hashes")
                    .get(&route.node_name)
                    .cloned()
                    .unwrap_or_default();

                let mut client = manager
                    .get_storager_client(&route.addr)
                    .await
                    .map_err(|e| {
                        Status::internal(format!("Failed to connect to storager: {}", e))
                    })?;

                    let storager_req = StoragerDeleteRequest {
                        keyword: keyword.clone(),
                        fid,
                        route_mode: route_mode.clone(),
                        dataset: dataset.clone(),
                        concurrency,
                        total_uploads,
                        total_queries,
                        total_updates,
                    };

                let response = client
                    .delete(storager_req)
                    .await
                    .map_err(|e| Status::internal(format!("Storager Delete failed: {}", e)))?;

                let resp = response.into_inner();

                let start = Instant::now();
                let verified = manager
                    .verify_proof_blocking(resp.proof.clone(), resp.root_hash.clone())
                    .await
                    .map_err(Status::internal)?;
                let duration = start.elapsed();
                println!(
                    "[METRIC] Proof Verification (Update-Delete): {:?}",
                    duration
                );

                if verified {
                    Ok((
                        keyword,
                        route.node_name,
                        route.prefix,
                        route.addr,
                        old_root_hash,
                        resp.proof,
                        resp.root_hash,
                        resp.root_accumulator,
                        resp.persistence_mode,
                    ))
                } else {
                    Err(Status::internal(format!(
                        "Delete proof verification failed for keyword: {}",
                        keyword
                    )))
                }
            }
        }))
        .buffer_unordered(max_inflight)
        .collect::<Vec<_>>()
        .await;

        let mut deleted_operations = Vec::new();
        let mut delete_proofs = Vec::new();
        let mut delete_root_hashes = Vec::new();
        let mut delete_root_accumulators = Vec::new();
        let mut delete_root_state_updates: HashMap<String, (Vec<u8>, Vec<u8>)> = HashMap::new();
        let mut delete_persistence_modes = Vec::new();

        for result in delete_results {
            match result {
                Ok((
                    keyword,
                    node_name,
                    prefix,
                    storager_addr,
                    old_root_hash,
                    proof,
                    root_hash,
                    root_accumulator,
                    persistence_mode,
                )) => {
                    let root_summary = self.root_summary_for_values(&root_hash, &root_accumulator);
                    delete_root_state_updates
                        .insert(node_name.clone(), (root_hash.clone(), root_accumulator.clone()));
                    self.record_prefix_delete(&keyword, &prefix, root_summary);
                    deleted_operations.push((
                        keyword,
                        prefix,
                        node_name,
                        storager_addr,
                        old_root_hash,
                    ));
                    delete_proofs.push(proof);
                    delete_root_hashes.push(root_hash);
                    delete_root_accumulators.push(root_accumulator);
                    delete_persistence_modes.push(persistence_mode);
                }
                Err(e) => return Err(e),
            }
        }

        self.record_persistence_modes(delete_persistence_modes);
        self.apply_root_state_updates(delete_root_state_updates.into_iter().map(
            |(node_name, (root_hash, root_accumulator))| (node_name, root_hash, root_accumulator),
        ));

        // Phase 2: Add new keywords (Parallel)
        let add_results = stream::iter(unique_new_keywords.into_iter().map(|keyword| {
            let manager = self.clone();
            let fid = req.fid.clone();
            let route_mode = route_mode.clone();
            let dataset = dataset.clone();
            async move {
                // Check if keyword is in active migration - buffer operation if so
                if manager
                    .active_prefix_migration_for_keyword(&keyword)
                    .is_some()
                {
                    let operation = PendingOperation::Add {
                        keyword: keyword.clone(),
                        fid: fid.clone(),
                    };
                    if let Err(e) = manager.buffer_operation_during_migration(operation).await {
                        println!("鈿狅笍  Failed to buffer Delete operation: {}", e);
                        return Err(Status::internal(format!(
                            "Failed to buffer operation: {}",
                            e
                        )));
                    }
                    println!(
                        "馃摝 Buffered Delete operation for keyword: {} during migration",
                        keyword
                    );
                    return Ok((
                        keyword,
                        String::new(),
                        String::new(),
                        vec![],
                        vec![],
                        vec![],
                        String::new(),
                    ));
                }

                let route = manager
                    .write_route_keyword(&keyword)
                    .ok_or_else(|| Status::internal("No storager available"))?;

                if manager
                    .active_prefix_migration_for_keyword(&keyword)
                    .is_some()
                {
                    let operation = PendingOperation::Add {
                        keyword: keyword.clone(),
                        fid: fid.clone(),
                    };
                    if let Err(e) = manager.buffer_operation_during_migration(operation).await {
                        println!("鈿狅笍  Failed to buffer Add operation: {}", e);
                        return Err(Status::internal(format!(
                            "Failed to buffer operation: {}",
                            e
                        )));
                    }
                    println!(
                        "馃摝 Buffered Add operation for keyword: {} during migration",
                        keyword
                    );
                    return Ok((
                        keyword,
                        String::new(),
                        String::new(),
                        vec![],
                        vec![],
                        vec![],
                        String::new(),
                    ));
                }

                let _subrequest_permit = manager
                    .acquire_subrequest_permits(&route.addr)
                    .await
                    .map_err(Status::internal)?;
                let mut client = manager
                    .get_storager_client(&route.addr)
                    .await
                    .map_err(|e| {
                        Status::internal(format!("Failed to connect to storager: {}", e))
                    })?;

                let storager_req = StoragerAddRequest {
                    keyword: keyword.clone(),
                    fid,
                    total_upload_kv_pairs: 0,
                    route_mode: route_mode.clone(),
                    dataset: dataset.clone(),
                    concurrency,
                    total_uploads,
                    total_queries,
                    total_updates,
                };

                let response = client
                    .add(storager_req)
                    .await
                    .map_err(|e| Status::internal(format!("Storager Add failed: {}", e)))?;

                let resp = response.into_inner();
                let persistence_mode = resp.persistence_mode.clone();

                let start = Instant::now();
                let verified = manager
                    .verify_proof_blocking(resp.proof.clone(), resp.root_hash.clone())
                    .await
                    .map_err(Status::internal)?;
                let duration = start.elapsed();
                println!("[METRIC] Proof Verification (Update-Add): {:?}", duration);

                if verified {
                    Ok((
                        keyword,
                        route.node_name,
                        route.prefix,
                        resp.proof,
                        resp.root_hash,
                        resp.root_accumulator,
                        persistence_mode,
                    ))
                } else {
                    Err(Status::internal(format!(
                        "Add proof verification failed for keyword: {}",
                        keyword
                    )))
                }
            }
        }))
        .buffer_unordered(max_inflight)
        .collect::<Vec<_>>()
        .await;

        let mut added_keywords: Vec<String> = Vec::new();
        let mut add_proofs: Vec<Vec<u8>> = Vec::new();
        let mut add_root_hashes: Vec<Vec<u8>> = Vec::new();
        let mut add_root_accumulators: Vec<Vec<u8>> = Vec::new();
        let mut rollback_needed = false;
        let mut error_message = String::new();
        let mut migration_happened = false;
        let mut pending_add_results: Vec<(
            String,
            String,
            String,
            Vec<u8>,
            Vec<u8>,
            Vec<u8>,
        )> = Vec::new();
        let mut add_root_state_updates: HashMap<String, (Vec<u8>, Vec<u8>)> = HashMap::new();
        let mut add_persistence_modes = Vec::new();

        for result in add_results {
            match result {
                Ok((keyword, node_name, prefix, proof, root_hash, root_accumulator, persistence_mode)) => {
                    add_root_state_updates
                        .insert(node_name.clone(), (root_hash.clone(), root_accumulator.clone()));
                    add_persistence_modes.push(persistence_mode);
                    pending_add_results.push((
                        keyword,
                        node_name,
                        prefix,
                        proof,
                        root_hash,
                        root_accumulator,
                    ));
                }
                Err(e) => {
                    rollback_needed = true;
                    error_message = e.message().to_string();
                    break;
                }
            }
        }

        self.record_persistence_modes(add_persistence_modes);
        self.apply_root_state_updates(add_root_state_updates.into_iter().map(
            |(node_name, (root_hash, root_accumulator))| (node_name, root_hash, root_accumulator),
        ));

        if rollback_needed {
            let rollback_added_keywords: Vec<String> = pending_add_results
                .iter()
                .map(|(keyword, _, _, _, _, _)| keyword.clone())
                .collect();
            println!(
                "閳跨媴绗? Add operation failed: {}, rolling back...",
                error_message
            );
            self.rollback_update(&req.fid, &deleted_operations, &rollback_added_keywords)
                .await;
            return Err(Status::internal(format!(
                "Update failed during add phase: {}",
                error_message
            )));
        }

        for (keyword, node_name, prefix, proof, root_hash, root_accumulator) in pending_add_results {
            let root_summary = self.root_summary_for_values(&root_hash, &root_accumulator);
            if let Some(split_plan) =
                self.record_prefix_insert(&keyword, &prefix, &node_name, root_summary)
            {
                migration_happened = true;
                self.schedule_split_migration(split_plan);
            }
            let final_route = self.route_keyword(&keyword);
            let final_node_name = final_route
                .as_ref()
                .map(|route| route.node_name.as_str())
                .unwrap_or(node_name.as_str());
            added_keywords.push(keyword);
            add_proofs.push(proof);
            add_root_hashes.push(self.get_root_hash(final_node_name).unwrap_or(root_hash));
            add_root_accumulators.push(
                self.get_root_accumulator(final_node_name)
                    .unwrap_or(root_accumulator),
            );
        }

        // Merge all proofs
        let mut all_proofs = delete_proofs;
        all_proofs.extend(add_proofs);
        let mut all_root_hashes = delete_root_hashes;
        all_root_hashes.extend(add_root_hashes);
        let mut all_root_accumulators = delete_root_accumulators;
        all_root_accumulators.extend(add_root_accumulators);

        // Select representative proof/root aligned by index (prefer last non-empty root)
        let rep_index = all_root_hashes
            .iter()
            .rposition(|h| !h.is_empty())
            .unwrap_or(0);
        let combined_proof = all_proofs.get(rep_index).cloned().unwrap_or_default();
        let combined_root_hash = all_root_hashes.get(rep_index).cloned().unwrap_or_default();
        let combined_root_accumulator = all_root_accumulators
            .get(rep_index)
            .cloned()
            .unwrap_or_default();
        let combined_proof = if migration_happened {
            Vec::new()
        } else {
            combined_proof
        };

        self.write_run_report();

        let persistence_mode = self.persistence_mode_snapshot();
        Ok(Response::new(UpdateResponse {
            success: true,
            message: "Update operation completed successfully".to_string(),
            combined_proof,
            combined_root_hash,
            combined_root_accumulator,
            route_mode,
            persistence_mode,
        }))
    }

    async fn reset_system(
        &self,
        _request: Request<ResetSystemRequest>,
    ) -> Result<Response<ResetSystemResponse>, Status> {
        let _guard = self.begin_reset().await;
        let _migration_guard = self.migration_lock.clone().lock_owned().await;
        let storagers = self.get_storagers();

        for (_node_name, addr) in storagers {
            let mut client = self
                .get_storager_client(&addr)
                .await
                .map_err(|e| Status::internal(format!("Failed to connect to storager: {}", e)))?;
            client
                .reset_storage(ResetStorageRequest {
                    route_mode: self.route_mode().as_str().to_string(),
                })
                .await
                .map_err(|e| Status::internal(format!("Storager reset failed: {}", e)))?;
        }

        self.root_hashes.write().unwrap().clear();
        self.root_accumulators.write().unwrap().clear();
        self.prefix_migrations.write().unwrap().clear();
        {
            let mut stats = self.boolean_query_stats.write().unwrap();
            stats.query_count = 0;
            stats.storager_visits = 0;
        }
        {
            let mut stats = self.split_migration_stats.write().unwrap();
            *stats = Default::default();
        }
        self.router.reset(self.split_threshold);

        Ok(Response::new(ResetSystemResponse {
            success: true,
            message: "system reset completed".to_string(),
        }))
    }
}

impl Manager {
    // split-aware helpers
    fn schedule_split_migration(&self, split_plan: crate::core::PrefixSplitPlan) {
        {
            let mut migrations = self.prefix_migrations.write().unwrap();
            let already_pending = migrations
                .get(&split_plan.parent_prefix)
                .map(|state| !state.confirmed)
                .unwrap_or(false);
            if already_pending {
                println!(
                    "[MIGRATE] background migration already pending for parent_prefix='{}'",
                    split_plan.parent_prefix
                );
                return;
            }

            migrations.insert(
                split_plan.parent_prefix.clone(),
                crate::core::manager::PrefixMigrationState {
                    parent_prefix: split_plan.parent_prefix.clone(),
                    source_node: split_plan.source.node_name.clone(),
                    source_addr: split_plan.source.addr.clone(),
                    child_prefixes: split_plan
                        .children
                        .iter()
                        .map(|child| child.prefix.clone())
                        .collect(),
                    target_nodes: split_plan
                        .children
                        .iter()
                        .map(|child| (child.prefix.clone(), child.node_name.clone()))
                        .collect(),
                    confirmed: false,
                    pending_operations: Vec::new(),
                },
            );
        }

        let manager = self.clone();
        let parent_prefix = split_plan.parent_prefix.clone();
        println!(
            "[MIGRATE] queued background migration for parent_prefix='{}'",
            parent_prefix
        );

        let migration_start = Instant::now();
        tokio::spawn(async move {
            let _migration_guard = manager.migration_lock.clone().lock_owned().await;
            let result = manager.run_split_migration(split_plan).await;
            manager.record_split_migration_duration(migration_start.elapsed());
            manager.write_run_report();
            if let Err(err) = result {
                eprintln!(
                    "[MIGRATE] background migration failed for prefix '{}': {}",
                    parent_prefix, err
                );
            }
        });
    }

    async fn run_split_migration(
        &self,
        split_plan: crate::core::PrefixSplitPlan,
    ) -> Result<(), Status> {
        let (dataset, concurrency, total_uploads, total_queries, total_updates) =
            self.run_metadata_snapshot_u32();
        println!(
            "=== EPRing Split Triggered: parent_prefix='{}', source_node='{}', source_addr='{}' ===",
            split_plan.parent_prefix,
            split_plan.source.node_name,
            split_plan.source.addr,
        );
        let route_mode = self.route_mode().as_str().to_string();
        println!(
            "[EPRING] nodes={} split_threshold summary_only",
            self.router.storager_count()
        );

        let mut source_client = self
            .get_storager_client(&split_plan.source.addr)
            .await
            .map_err(|e| {
                Status::internal(format!("Failed to connect to source storager: {}", e))
            })?;

        let mut payload_bytes_total: u64 = 0;

        let src_io_start = {
            let _subrequest_permit = self
                .acquire_subrequest_permits(&split_plan.source.addr)
                .await
                .map_err(Status::internal)?;
            let resp = source_client
                .get_io_stats(StoragerIoStatsRequest {})
                .await
                .map_err(|e| Status::internal(format!("GetIoStats failed for source: {}", e)))?
                .into_inner();
            ProcessIoStats {
                read_bytes: resp.read_bytes,
                write_bytes: resp.write_bytes,
                read_ops: resp.read_ops,
                write_ops: resp.write_ops,
            }
        };

        let mut target_io_starts: HashMap<String, ProcessIoStats> = HashMap::new();
        let mut seen_target_addrs: HashSet<String> = HashSet::new();
        for child in &split_plan.children {
            if child.node_name == split_plan.source.node_name {
                continue;
            }
            if !seen_target_addrs.insert(child.addr.clone()) {
                continue;
            }
            let mut target_client = self.get_storager_client(&child.addr).await.map_err(|e| {
                Status::internal(format!("Failed to connect to target storager: {}", e))
            })?;
            let _subrequest_permit = self
                .acquire_subrequest_permits(&child.addr)
                .await
                .map_err(Status::internal)?;
            let resp = target_client
                .get_io_stats(StoragerIoStatsRequest {})
                .await
                .map_err(|e| Status::internal(format!("GetIoStats failed for target: {}", e)))?
                .into_inner();
            target_io_starts.insert(
                child.addr.clone(),
                ProcessIoStats {
                    read_bytes: resp.read_bytes,
                    write_bytes: resp.write_bytes,
                    read_ops: resp.read_ops,
                    write_ops: resp.write_ops,
                },
            );
        }

        let mut current_roots: HashMap<String, (Vec<u8>, Vec<u8>)> = HashMap::new();
        current_roots.insert(
            split_plan.source.node_name.clone(),
            (
                self.get_root_hash(&split_plan.source.node_name)
                    .unwrap_or_default(),
                self.get_root_accumulator(&split_plan.source.node_name)
                    .unwrap_or_default(),
            ),
        );

        for child in &split_plan.children {
            if child.node_name == split_plan.source.node_name {
                continue;
            }

            let child_start = std::time::Instant::now();
            println!(
                "[MIGRATE] prefix={} source={} -> target={} ({}) prepare start",
                child.prefix, split_plan.source.node_name, child.node_name, child.addr
            );

            let _subrequest_permit = self
                .acquire_subrequest_permits(&split_plan.source.addr)
                .await
                .map_err(Status::internal)?;
            let prepare_response = source_client
                .prepare_retain_prefix_segment(StoragerPrepareRetainPrefixRequest {
                    prefix: child.prefix.clone(),
                    route_mode: route_mode.clone(),
                    dataset: dataset.clone(),
                    concurrency,
                    total_uploads,
                    total_queries,
                    total_updates,
                })
                .await
                .map_err(|e| Status::internal(format!("PrepareRetainPrefixSegment failed: {}", e)))?
                .into_inner();
            payload_bytes_total =
                payload_bytes_total.saturating_add(prepare_response.segment.len() as u64);
            println!(
                "[MIGRATE] prefix={} prepare done in {:?}, segment_bytes={}",
                child.prefix,
                child_start.elapsed(),
                prepare_response.segment.len()
            );

            current_roots.insert(
                split_plan.source.node_name.clone(),
                (
                    prepare_response.root_hash.clone(),
                    prepare_response.root_accumulator.clone(),
                ),
            );

            let mut target_client = self.get_storager_client(&child.addr).await.map_err(|e| {
                Status::internal(format!("Failed to connect to target storager: {}", e))
            })?;
            let import_start = std::time::Instant::now();
            let _subrequest_permit = self
                .acquire_subrequest_permits(&child.addr)
                .await
                .map_err(Status::internal)?;
            let import_response = target_client
                .import_prefix_segment(StoragerImportPrefixRequest {
                    segment: prepare_response.segment,
                    route_mode: route_mode.clone(),
                    dataset: dataset.clone(),
                    concurrency,
                    total_uploads,
                    total_queries,
                    total_updates,
                })
                .await
                .map_err(|e| Status::internal(format!("ImportPrefixSegment failed: {}", e)))?
                .into_inner();
            println!(
                "[MIGRATE] prefix={} import done in {:?}",
                child.prefix,
                import_start.elapsed()
            );

            current_roots.insert(
                child.node_name.clone(),
                (import_response.root_hash, import_response.root_accumulator),
            );

            let confirm_start = std::time::Instant::now();
            let _subrequest_permit = self
                .acquire_subrequest_permits(&split_plan.source.addr)
                .await
                .map_err(Status::internal)?;
            let confirm_response = source_client
                .confirm_prefix_migration(StoragerConfirmPrefixMigrationRequest {
                    prefix: child.prefix.clone(),
                    route_mode: route_mode.clone(),
                    dataset: dataset.clone(),
                    concurrency,
                    total_uploads,
                    total_queries,
                    total_updates,
                })
                .await
                .map_err(|e| Status::internal(format!("ConfirmPrefixMigration failed: {}", e)))?
                .into_inner();
            println!(
                "[MIGRATE] prefix={} confirm done in {:?}, total {:?}",
                child.prefix,
                confirm_start.elapsed(),
                child_start.elapsed()
            );

            current_roots.insert(
                split_plan.source.node_name.clone(),
                (
                    confirm_response.root_hash,
                    confirm_response.root_accumulator,
                ),
            );
        }

        self.apply_root_state_updates(current_roots.iter().map(
            |(node_name, (root_hash, root_accumulator))| {
                (
                    node_name.clone(),
                    root_hash.clone(),
                    root_accumulator.clone(),
                )
            },
        ));

        let mut prefix_summary_updates = Vec::new();
        for child in &split_plan.children {
            let (root_hash, root_accumulator) = current_roots
                .get(&child.node_name)
                .cloned()
                .unwrap_or_else(|| {
                    (
                        self.get_root_hash(&child.node_name).unwrap_or_default(),
                        self.get_root_accumulator(&child.node_name)
                            .unwrap_or_default(),
                    )
                });
            let root_summary = self.root_summary_for_values(&root_hash, &root_accumulator);
            prefix_summary_updates.push((child.prefix.clone(), root_summary));
        }
        self.update_prefix_summaries(prefix_summary_updates);

        self.clear_keyword_overrides_for_prefix(&split_plan.parent_prefix);
        let replayed = self
            .replay_pending_operations(&split_plan.parent_prefix)
            .await
            .map_err(Status::internal)?;
        if !replayed.is_empty() {
            println!(
                "Replay completed for prefix '{}': {} operation(s)",
                split_plan.parent_prefix,
                replayed.len()
            );
        }
        self.write_upload_prefix_import_report();
        if let Some(state) = self
            .prefix_migrations
            .write()
            .unwrap()
            .get_mut(&split_plan.parent_prefix)
        {
            state.confirmed = true;
        }

        let src_io_end = {
            let _subrequest_permit = self
                .acquire_subrequest_permits(&split_plan.source.addr)
                .await
                .map_err(Status::internal)?;
            let resp = source_client
                .get_io_stats(StoragerIoStatsRequest {})
                .await
                .map_err(|e| Status::internal(format!("GetIoStats failed for source: {}", e)))?
                .into_inner();
            ProcessIoStats {
                read_bytes: resp.read_bytes,
                write_bytes: resp.write_bytes,
                read_ops: resp.read_ops,
                write_ops: resp.write_ops,
            }
        };

        let src_io_delta = ProcessIoStats {
            read_bytes: src_io_end.read_bytes.saturating_sub(src_io_start.read_bytes),
            write_bytes: src_io_end.write_bytes.saturating_sub(src_io_start.write_bytes),
            read_ops: src_io_end.read_ops.saturating_sub(src_io_start.read_ops),
            write_ops: src_io_end.write_ops.saturating_sub(src_io_start.write_ops),
        };

        let mut tgt_io_delta = ProcessIoStats::default();
        for (addr, start) in &target_io_starts {
            let mut target_client =
                self.get_storager_client(addr)
                    .await
                    .map_err(|e| Status::internal(format!("Failed to connect to target storager: {}", e)))?;
            let _subrequest_permit = self
                .acquire_subrequest_permits(addr)
                .await
                .map_err(Status::internal)?;
            let resp = target_client
                .get_io_stats(StoragerIoStatsRequest {})
                .await
                .map_err(|e| Status::internal(format!("GetIoStats failed for target: {}", e)))?
                .into_inner();
            let end = ProcessIoStats {
                read_bytes: resp.read_bytes,
                write_bytes: resp.write_bytes,
                read_ops: resp.read_ops,
                write_ops: resp.write_ops,
            };
            tgt_io_delta.read_bytes = tgt_io_delta
                .read_bytes
                .saturating_add(end.read_bytes.saturating_sub(start.read_bytes));
            tgt_io_delta.write_bytes = tgt_io_delta
                .write_bytes
                .saturating_add(end.write_bytes.saturating_sub(start.write_bytes));
            tgt_io_delta.read_ops = tgt_io_delta
                .read_ops
                .saturating_add(end.read_ops.saturating_sub(start.read_ops));
            tgt_io_delta.write_ops = tgt_io_delta
                .write_ops
                .saturating_add(end.write_ops.saturating_sub(start.write_ops));
        }

        self.record_split_migration_io(src_io_delta, tgt_io_delta, payload_bytes_total);
        Ok(())
    }

    async fn replay_pending_operations(&self, prefix: &str) -> Result<Vec<String>, String> {
        let (dataset, concurrency, total_uploads, total_queries, total_updates) =
            self.run_metadata_snapshot_u32();
        let route_mode = self.route_mode().as_str().to_string();
        let pending_operations = {
            let mut migrations = self.prefix_migrations.write().unwrap();
            let state = migrations
                .get_mut(prefix)
                .ok_or_else(|| format!("prefix migration not found: {}", prefix))?;
            std::mem::take(&mut state.pending_operations)
        };

        let mut replayed = Vec::with_capacity(pending_operations.len());
        let mut root_state_updates: HashMap<String, (Vec<u8>, Vec<u8>)> = HashMap::new();
        let mut prefix_summary_updates: HashMap<String, Vec<u8>> = HashMap::new();

        for operation in pending_operations {
            match operation {
                PendingOperation::Add { keyword, fid } => {
                    let route = self
                        .route_keyword(&keyword)
                        .ok_or_else(|| format!("no storager available for keyword: {}", keyword))?;

                    let _subrequest_permit = self.acquire_subrequest_permits(&route.addr).await?;
                    let mut client = self
                        .get_storager_client(&route.addr)
                        .await
                        .map_err(|e| format!("failed to connect to storager: {}", e))?;

                    let resp = client
                        .add(StoragerAddRequest {
                            keyword: keyword.clone(),
                            fid,
                            total_upload_kv_pairs: 0,
                            route_mode: route_mode.clone(),
                            dataset: dataset.clone(),
                            concurrency,
                            total_uploads,
                            total_queries,
                            total_updates,
                        })
                        .await
                        .map_err(|e| format!("storager add failed for {}: {}", keyword, e))?
                        .into_inner();
                    self.record_persistence_mode(&resp.persistence_mode);

                    let verified = self
                        .verify_proof_blocking(resp.proof.clone(), resp.root_hash.clone())
                        .await?;
                    if !verified {
                        return Err(format!(
                            "proof verification failed while replaying add for {}",
                            keyword
                        ));
                    }

                    let root_summary =
                        self.root_summary_for_values(&resp.root_hash, &resp.root_accumulator);
                    root_state_updates.insert(
                        route.node_name.clone(),
                        (resp.root_hash.clone(), resp.root_accumulator.clone()),
                    );

                    if let Some(_split_plan) = self.record_prefix_insert(
                        &keyword,
                        &route.prefix,
                        &route.node_name,
                        root_summary,
                    ) {
                        println!(
                            "Skipping nested split migration during replay for keyword: {}",
                            keyword
                        );
                    }
                    self.record_upload_prefix_import(&route.node_name, &route.prefix, 1);

                    let final_route = self.route_keyword(&keyword).unwrap_or(route);
                    let final_summary = root_state_updates
                        .get(&final_route.node_name)
                        .map(|(root_hash, root_accumulator)| {
                            self.root_summary_for_values(root_hash, root_accumulator)
                        })
                        .unwrap_or_else(|| self.root_summary_for_storager(&final_route.node_name));
                    prefix_summary_updates.insert(final_route.prefix.clone(), final_summary);
                    replayed.push(format!("add:{}", keyword));
                }
                PendingOperation::Delete { keyword, fid } => {
                    let route = self
                        .route_keyword(&keyword)
                        .ok_or_else(|| format!("no storager available for keyword: {}", keyword))?;

                    let _subrequest_permit = self.acquire_subrequest_permits(&route.addr).await?;
                    let mut client = self
                        .get_storager_client(&route.addr)
                        .await
                        .map_err(|e| format!("failed to connect to storager: {}", e))?;

                    let resp = client
                        .delete(StoragerDeleteRequest {
                            keyword: keyword.clone(),
                            fid,
                            route_mode: route_mode.clone(),
                            dataset: dataset.clone(),
                            concurrency,
                            total_uploads,
                            total_queries,
                            total_updates,
                        })
                        .await
                        .map_err(|e| format!("storager delete failed for {}: {}", keyword, e))?
                        .into_inner();

                    let verified = self
                        .verify_proof_blocking(resp.proof.clone(), resp.root_hash.clone())
                        .await?;
                    if !verified {
                        return Err(format!(
                            "proof verification failed while replaying delete for {}",
                            keyword
                        ));
                    }

                    let root_summary =
                        self.root_summary_for_values(&resp.root_hash, &resp.root_accumulator);
                    root_state_updates.insert(
                        route.node_name.clone(),
                        (resp.root_hash.clone(), resp.root_accumulator.clone()),
                    );
                    self.record_prefix_delete(&keyword, &route.prefix, root_summary.clone());
                    prefix_summary_updates.insert(route.prefix.clone(), root_summary);
                    replayed.push(format!("delete:{}", keyword));
                }
                PendingOperation::Update {
                    keyword,
                    old_fid,
                    new_fid,
                } => {
                    if old_fid != new_fid {
                        return Err(format!(
                            "replay for update with differing fid values is unsupported: {}",
                            keyword
                        ));
                    }

                    let route = self
                        .route_keyword(&keyword)
                        .ok_or_else(|| format!("no storager available for keyword: {}", keyword))?;

                    let _subrequest_permit = self.acquire_subrequest_permits(&route.addr).await?;
                    let mut client = self
                        .get_storager_client(&route.addr)
                        .await
                        .map_err(|e| format!("failed to connect to storager: {}", e))?;

                    let delete_resp = client
                        .delete(StoragerDeleteRequest {
                            keyword: keyword.clone(),
                            fid: old_fid,
                            route_mode: route_mode.clone(),
                            dataset: dataset.clone(),
                            concurrency,
                            total_uploads,
                            total_queries,
                            total_updates,
                        })
                        .await
                        .map_err(|e| format!("storager delete failed for {}: {}", keyword, e))?
                        .into_inner();
                    self.record_persistence_mode(&delete_resp.persistence_mode);

                    let delete_verified = self
                        .verify_proof_blocking(
                            delete_resp.proof.clone(),
                            delete_resp.root_hash.clone(),
                        )
                        .await?;
                    if !delete_verified {
                        return Err(format!(
                            "proof verification failed while replaying update-delete for {}",
                            keyword
                        ));
                    }

                    let delete_summary = self.root_summary_for_values(
                        &delete_resp.root_hash,
                        &delete_resp.root_accumulator,
                    );
                    root_state_updates.insert(
                        route.node_name.clone(),
                        (
                            delete_resp.root_hash.clone(),
                            delete_resp.root_accumulator.clone(),
                        ),
                    );
                    self.record_prefix_delete(&keyword, &route.prefix, delete_summary);

                    let add_resp = client
                        .add(StoragerAddRequest {
                            keyword: keyword.clone(),
                            fid: new_fid,
                            total_upload_kv_pairs: 0,
                            route_mode: route_mode.clone(),
                            dataset: dataset.clone(),
                            concurrency,
                            total_uploads,
                            total_queries,
                            total_updates,
                        })
                        .await
                        .map_err(|e| format!("storager add failed for {}: {}", keyword, e))?
                        .into_inner();
                    self.record_persistence_mode(&add_resp.persistence_mode);

                    let add_verified = self
                        .verify_proof_blocking(
                            add_resp.proof.clone(),
                            add_resp.root_hash.clone(),
                        )
                        .await?;
                    if !add_verified {
                        return Err(format!(
                            "proof verification failed while replaying update-add for {}",
                            keyword
                        ));
                    }

                    let root_summary = self
                        .root_summary_for_values(&add_resp.root_hash, &add_resp.root_accumulator);
                    root_state_updates.insert(
                        route.node_name.clone(),
                        (add_resp.root_hash.clone(), add_resp.root_accumulator.clone()),
                    );

                    if let Some(_split_plan) = self.record_prefix_insert(
                        &keyword,
                        &route.prefix,
                        &route.node_name,
                        root_summary,
                    ) {
                        println!(
                            "Skipping nested split migration during replay for keyword: {}",
                            keyword
                        );
                    }

                    let final_route = self.route_keyword(&keyword).unwrap_or(route);
                    let final_summary = root_state_updates
                        .get(&final_route.node_name)
                        .map(|(root_hash, root_accumulator)| {
                            self.root_summary_for_values(root_hash, root_accumulator)
                        })
                        .unwrap_or_else(|| self.root_summary_for_storager(&final_route.node_name));
                    prefix_summary_updates.insert(final_route.prefix.clone(), final_summary);
                    replayed.push(format!("update:{}", keyword));
                }
            }
        }

        self.apply_root_state_updates(
            root_state_updates
                .into_iter()
                .map(|(node_name, (root_hash, root_accumulator))| {
                    (node_name, root_hash, root_accumulator)
                }),
        );
        self.update_prefix_summaries(prefix_summary_updates.into_iter());

        Ok(replayed)
    }
    /// 閸楁洖鍙ч柨顔跨槤閺屻儴顕?
    pub(crate) async fn query_single_keyword(
        &self,
        keyword: &str,
    ) -> Result<Response<QueryResponse>, Status> {
        let (dataset, concurrency, total_uploads, total_queries, total_updates) =
            self.run_metadata_snapshot_u32();
        let route_mode = self.route_mode().as_str().to_string();
        println!("  Query type: Single keyword '{}'", keyword);

        let route = self
            .query_route_keyword(keyword)
            .ok_or_else(|| Status::internal("No storager available"))?;

        // 娴ｈ法鏁ゆ潻鐐村复濮圭姾骞忛崣鏍ь吂閹撮顏?
        let mut client = self
            .get_storager_client(&route.addr)
            .await
            .map_err(|e| Status::internal(format!("Failed to connect to storager: {}", e)))?;

        let storager_req = StoragerQueryRequest {
            keyword: keyword.to_string(),
            route_mode: route_mode.clone(),
            dataset: dataset.clone(),
            concurrency,
            total_uploads,
            total_queries,
            total_updates,
        };

        let response = client
            .query(storager_req)
            .await
            .map_err(|e| Status::internal(format!("Storager Query failed: {}", e)))?;

        let resp = response.into_inner();
        self.record_persistence_mode(&resp.persistence_mode);
        let root_hash = resp.root_hash.clone();
        let root_summary = self.root_summary_for_values(&root_hash, &resp.root_accumulator);
        self.apply_root_state_updates(std::iter::once((
            route.node_name.clone(),
            resp.root_hash.clone(),
            resp.root_accumulator.clone(),
        )));
        self.update_prefix_summaries(std::iter::once((route.prefix.clone(), root_summary)));

        // Verify proof
        let start = Instant::now();
        let verified = self
            .verify_query_response_blocking(
                resp.proof.clone(),
                root_hash.clone(),
                resp.fids.clone(),
            )
            .await
            .map_err(Status::internal)?;
        let duration = start.elapsed();
        println!("[METRIC] Proof Verification (Query): {:?}", duration);

        let mut node_root_hashes = HashMap::new();
        node_root_hashes.insert(route.node_name.clone(), root_hash.clone());
        let mut node_root_accumulators = HashMap::new();
        node_root_accumulators.insert(route.node_name, resp.root_accumulator.clone());

        self.write_run_report();
        let persistence_mode = self.persistence_mode_snapshot();
        Ok(Response::new(QueryResponse {
            fids: resp.fids,
            proof: resp.proof,
            root_hash,
            verified,
            node_root_hashes,
            root_accumulator: resp.root_accumulator,
            node_root_accumulators,
            manager_proof_aggregation_ms: 0.0,
            manager_set_operation_proof_generation_ms: 0.0,
            route_mode,
            persistence_mode,
        }))
    }

    /// 鐢啫鐨甸崙鑺ユ殶閺屻儴顕?
    pub(crate) async fn query_boolean_function(
        &self,
        func: &str,
    ) -> Result<Response<QueryResponse>, Status> {
        let (dataset, concurrency, total_uploads, total_queries, total_updates) =
            self.run_metadata_snapshot_u32();
        let route_mode = self.route_mode().as_str().to_string();
        println!("  Query type: Boolean function '{}'", func);

        // 1. 瑙ｆ瀽甯冨皵琛ㄨ揪寮?
        let expr = parse_boolean_expr(func).map_err(|e| {
            Status::invalid_argument(format!("Failed to parse boolean expression: {}", e))
        })?;
        println!("  Parsed expression: {}", expr.to_string());

        // 2. 閼惧嘲褰囬幍鈧張澶婂彠闁款喛鐦?
        let keywords = expr.get_keywords();
        println!("  Keywords: {:?}", keywords);

        // 3. 楠炶泛褰傞弻銉嚄閹碘偓閺堝鍙ч柨顔跨槤
        let max_inflight = self.max_inflight_subrequests();
        let results = stream::iter(keywords.into_iter().map(|keyword| {
            let manager = self.clone();
            let route_mode = route_mode.clone();
            let dataset = dataset.clone();
            async move {
                let route = manager
                    .route_keyword(&keyword)
                    .ok_or_else(|| Status::internal("No storager available"))?;

                let _subrequest_permit = manager
                    .acquire_subrequest_permits(&route.addr)
                    .await
                    .map_err(Status::internal)?;
                let mut client = manager
                    .get_storager_client(&route.addr)
                    .await
                    .map_err(|e| {
                        Status::internal(format!("Failed to connect to storager: {}", e))
                    })?;

                let storager_req = StoragerQueryRequest {
                    keyword: keyword.clone(),
                    route_mode: route_mode.clone(),
                    dataset: dataset.clone(),
                    concurrency,
                    total_uploads,
                    total_queries,
                    total_updates,
                };

                let response = client
                    .query(storager_req)
                    .await
                    .map_err(|e| Status::internal(format!("Storager Query failed: {}", e)))?;

                let resp = response.into_inner();
                let root_hash = resp.root_hash.clone();

                let verified = manager
                    .verify_query_response_blocking(
                        resp.proof.clone(),
                        root_hash.clone(),
                        resp.fids.clone(),
                    )
                    .await
                    .map_err(Status::internal)?;
                if !verified {
                    return Err(Status::internal(format!(
                        "Proof verification failed for keyword: {}",
                        keyword
                    )));
                }

                Ok((
                    keyword,
                    resp.fids,
                    resp.proof,
                    route.node_name,
                    route.prefix,
                    root_hash,
                    resp.root_accumulator,
                    resp.persistence_mode,
                ))
            }
        }))
        .buffer_unordered(max_inflight)
        .collect::<Vec<_>>()
        .await;

        let aggregation_start = Instant::now();
        let mut keyword_results = HashMap::new();
        let mut keyword_leaf_data = HashMap::new();
        let mut node_root_hashes = HashMap::new();
        let mut node_root_accumulators = HashMap::new();
        let mut root_state_updates: HashMap<String, (Vec<u8>, Vec<u8>)> = HashMap::new();
        let mut prefix_summary_updates = Vec::new();
        let mut persistence_modes = Vec::new();

        for result in results {
            match result {
                Ok((
                    keyword,
                    fids,
                    proof,
                    node_name,
                    prefix,
                    root_hash,
                    root_accumulator,
                    persistence_mode,
                )) => {
                    keyword_results.insert(keyword.clone(), fids.iter().cloned().collect());
                    keyword_leaf_data.insert(
                        keyword.clone(),
                        KeywordQueryLeafData {
                            keyword,
                            fids,
                            proof,
                            node_name: node_name.clone(),
                            ads_mode: self.ads_mode(),
                            root_hash: root_hash.clone(),
                        },
                    );
                    let root_summary = self.root_summary_for_values(&root_hash, &root_accumulator);
                    root_state_updates
                        .insert(node_name.clone(), (root_hash.clone(), root_accumulator.clone()));
                    prefix_summary_updates.push((prefix, root_summary));
                    persistence_modes.push(persistence_mode);
                    node_root_hashes.insert(node_name.clone(), root_hash);
                    node_root_accumulators.insert(node_name, root_accumulator);
                }
                Err(e) => return Err(e),
            }
        }
        self.record_persistence_modes(persistence_modes);
        self.apply_root_state_updates(root_state_updates.into_iter().map(
            |(node_name, (root_hash, root_accumulator))| (node_name, root_hash, root_accumulator),
        ));
        self.update_prefix_summaries(prefix_summary_updates);
        let proof_aggregation_duration = aggregation_start.elapsed();

        // Build proof tree and evaluate expression
        let expected_result_fids = sorted_vec_from_set(&expr.evaluate(&keyword_results));
        let proof_generation_start = Instant::now();
        let expr_for_proof = expr.clone();
        let leaf_data_for_proof = keyword_leaf_data.clone();
        let func_for_proof = func.to_string();
        let set_proof_mode = self.set_proof_mode();
        let (combined_proof, root_hash, result_fids) = self
            .run_blocking_proof_task("boolean proof aggregation", move || {
                match set_proof_mode {
                    SetProofMode::Polynomial => {
                        let (tree, result_set) =
                            build_polynomial_proof_tree(&expr_for_proof, &leaf_data_for_proof)
                                .map_err(|e| e.message().to_string())?;
                        let result_fids = sorted_vec_from_set(&result_set);

                        if expected_result_fids != result_fids {
                            return Err(
                                "Polynomial proof tree result does not match boolean evaluation result"
                                    .to_string(),
                            );
                        }

                        let aggregate_proof = PolynomialIntersectionAggregateProof {
                            expr: func_for_proof.clone(),
                            result_fids: result_fids.clone(),
                            root: tree,
                        };

                        let combined_proof = encode_polynomial_intersection_proof(&aggregate_proof)
                            .map_err(|e| e.to_string())?;
                        let root_hash = polynomial_intersection_root_hash(&combined_proof);
                        Ok((combined_proof, root_hash, result_fids))
                    }
                    SetProofMode::Accumulator => {
                        let (tree, result_set) =
                            build_accumulator_proof_tree(&expr_for_proof, &leaf_data_for_proof)
                                .map_err(|e| e.message().to_string())?;
                        let result_fids = sorted_vec_from_set(&result_set);

                        if expected_result_fids != result_fids {
                            return Err(
                                "Accumulator proof tree result does not match boolean evaluation result"
                                    .to_string(),
                            );
                        }

                        let aggregate_proof = AccumulatorSetOperationAggregateProof {
                            expr: func_for_proof,
                            result_fids: result_fids.clone(),
                            root: tree,
                        };

                        let combined_proof =
                            encode_accumulator_set_operation_proof(&aggregate_proof)
                                .map_err(|e| e.to_string())?;
                        let root_hash =
                            common::accumulator_set_operation_root_hash(&combined_proof);
                        Ok((combined_proof, root_hash, result_fids))
                    }
                }
            })
            .await
            .map_err(Status::internal)?;
        let proof_generation_duration = proof_generation_start.elapsed();
        let proof_aggregation_ms = proof_aggregation_duration.as_secs_f64() * 1000.0;
        let proof_generation_ms = proof_generation_duration.as_secs_f64() * 1000.0;
        self.record_boolean_query_proof_generation(
            proof_aggregation_duration + proof_generation_duration,
        );

        self.record_boolean_query(node_root_hashes.len());
        self.write_run_report();

        let persistence_mode = self.persistence_mode_snapshot();
        return Ok(Response::new(QueryResponse {
            fids: result_fids,
            proof: combined_proof,
            root_hash,
            verified: true,
            node_root_hashes: node_root_hashes.clone(),
            root_accumulator: Vec::new(),
            node_root_accumulators,
            manager_proof_aggregation_ms: proof_aggregation_ms,
            manager_set_operation_proof_generation_ms: proof_generation_ms,
            route_mode,
            persistence_mode,
        }));

        // 5. 閻㈢喐鍨氱紒鍕値鐠囦焦妲?

        // 6. 娴ｈ法鏁ょ粭顑跨娑?storager 閻?root hash 娴ｆ粈璐熸禒锝堛€?(Deprecated)
    }
    /// 閸ョ偞绮?Update 閹垮秳缍?
    ///
    /// 瑜?Update 閹垮秳缍旀径杈Е閺?闂団偓鐟?
    /// 1. 闁插秵鏌婂ǎ璇插瀹告彃鍨归梽銈囨畱閸忔娊鏁拠?    /// 2. 閸掔娀娅庡鍙夊潑閸旂姷娈戦弬鏉垮彠闁款喛鐦?
    async fn rollback_update(
        &self,
        fid: &str,
        deleted_operations: &[(String, String, String, String, Vec<u8>)], // (keyword, prefix, node_name, storager_addr, old_root_hash)
        added_keywords: &[String],
    ) {
        let (dataset, concurrency, total_uploads, total_queries, total_updates) =
            self.run_metadata_snapshot_u32();
        let route_mode = self.route_mode().as_str().to_string();
        println!("棣冩敡 Rolling back update operation for fid: {}", fid);
        let mut root_state_updates: HashMap<String, (Vec<u8>, Vec<u8>)> = HashMap::new();
        let mut prefix_summary_updates: HashMap<String, Vec<u8>> = HashMap::new();

        // Rollback Phase 1: Re-add deleted keywords
        for (keyword, prefix, node_name, storager_addr, _old_root_hash) in deleted_operations {
            println!("  Re-adding deleted keyword: {}", keyword);

            if let Ok(mut client) = self.get_storager_client(storager_addr).await {
                let _subrequest_permit = match self.acquire_subrequest_permits(storager_addr).await {
                    Ok(permit) => permit,
                    Err(err) => {
                        println!("  Failed to acquire re-add permit for {}: {}", keyword, err);
                        continue;
                    }
                };
                let storager_req = StoragerAddRequest {
                    keyword: keyword.clone(),
                    fid: fid.to_string(),
                    total_upload_kv_pairs: 0,
                    route_mode: route_mode.clone(),
                    dataset: dataset.clone(),
                    concurrency,
                    total_uploads,
                    total_queries,
                    total_updates,
                };

                match client.add(storager_req).await {
                    Ok(response) => {
                        let resp = response.into_inner();
                        let verified = match self
                            .verify_proof_blocking(resp.proof.clone(), resp.root_hash.clone())
                            .await
                        {
                            Ok(verified) => verified,
                            Err(err) => {
                                println!("  閴?Failed to verify re-add {}: {}", keyword, err);
                                false
                            }
                        };
                        if verified {
                            let root_summary = self
                                .root_summary_for_values(&resp.root_hash, &resp.root_accumulator);
                            root_state_updates.insert(
                                node_name.clone(),
                                (resp.root_hash.clone(), resp.root_accumulator.clone()),
                            );
                            if let Some(split_plan) =
                                self.record_prefix_insert(keyword, prefix, node_name, root_summary)
                            {
                                self.schedule_split_migration(split_plan);
                            }
                            if let Some(final_route) = self.route_keyword(keyword) {
                                let final_summary = root_state_updates
                                    .get(&final_route.node_name)
                                    .map(|(root_hash, root_accumulator)| {
                                        self.root_summary_for_values(root_hash, root_accumulator)
                                    })
                                    .unwrap_or_else(|| {
                                        self.root_summary_for_storager(&final_route.node_name)
                                    });
                                prefix_summary_updates
                                    .insert(final_route.prefix.clone(), final_summary);
                            }
                            println!("  閴?Re-added: {}", keyword);
                        }
                    }
                    Err(e) => {
                        println!("  閴?Failed to re-add {}: {}", keyword, e);
                    }
                }
            }
        }

        // Rollback Phase 2: Remove added keywords
        for keyword in added_keywords {
            println!("  Removing added keyword: {}", keyword);

            if let Some(route) = self.route_keyword(keyword) {
                if let Ok(mut client) = self.get_storager_client(&route.addr).await {
                    let _subrequest_permit =
                        match self.acquire_subrequest_permits(&route.addr).await {
                            Ok(permit) => permit,
                            Err(err) => {
                                println!(
                                    "  Failed to acquire remove permit for {}: {}",
                                    keyword, err
                                );
                                continue;
                            }
                        };
                    let storager_req = StoragerDeleteRequest {
                        keyword: keyword.clone(),
                        fid: fid.to_string(),
                        route_mode: route_mode.clone(),
                        dataset: dataset.clone(),
                        concurrency,
                        total_uploads,
                        total_queries,
                        total_updates,
                    };

                    match client.delete(storager_req).await {
                        Ok(response) => {
                            let resp = response.into_inner();
                            let verified = match self
                                .verify_proof_blocking(resp.proof.clone(), resp.root_hash.clone())
                                .await
                            {
                                Ok(verified) => verified,
                                Err(err) => {
                                    println!("  閴?Failed to verify remove {}: {}", keyword, err);
                                    false
                                }
                            };
                            if verified {
                                let root_summary = self.root_summary_for_values(
                                    &resp.root_hash,
                                    &resp.root_accumulator,
                                );
                                root_state_updates.insert(
                                    route.node_name.clone(),
                                    (resp.root_hash, resp.root_accumulator),
                                );
                                self.record_prefix_delete(keyword, &route.prefix, root_summary.clone());
                                prefix_summary_updates.insert(route.prefix.clone(), root_summary);
                                println!("  閴?Removed: {}", keyword);
                            }
                        }
                        Err(e) => {
                            println!("  閴?Failed to remove {}: {}", keyword, e);
                        }
                    }
                }
            }
        }

        self.apply_root_state_updates(
            root_state_updates
                .into_iter()
                .map(|(node_name, (root_hash, root_accumulator))| {
                    (node_name, root_hash, root_accumulator)
                }),
        );
        self.update_prefix_summaries(prefix_summary_updates.into_iter());
        println!("棣冩敡 Rollback completed");
    }
}
