use crate::core::{Manager, PendingOperation};
use accumulator_ads::{digest_set_from_set, Fr, IntersectionProof, Set, UnionProof};
use common::parse_boolean_expr;
use common::rpc::{
    manager_service_server::ManagerService, AddRequest, AddResponse, BatchAddRequest,
    BatchAddResponse, DeleteRequest, DeleteResponse, QueryRequest, QueryResponse, ResetSystemRequest,
    ResetSystemResponse, ResetStorageRequest,
    StoragerAddRequest, StoragerBatchAddItem, StoragerBatchAddRequest,
    StoragerConfirmPrefixMigrationRequest, StoragerDeleteRequest, StoragerImportPrefixRequest,
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
        let _gate = self.reset_lock.clone().lock_owned().await;
        let req = request.into_inner();
        println!("Manager received Add request for fid: {}", req.fid);

        // Deduplicate keywords to avoid adding the same element twice
        let unique_keywords: HashSet<String> = req.keywords.into_iter().collect();
        let keyword_count = unique_keywords.len();

        if keyword_count == 0 {
            return Ok(Response::new(AddResponse {
                success: false,
                message: "No keywords provided".to_string(),
                combined_proof: vec![],
                combined_root_hash: vec![],
                combined_root_accumulator: vec![],
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
            };

            let response = client
                .add(storager_req)
                .await
                .map_err(|e| Status::internal(format!("Storager Add failed: {}", e)))?;
            let resp = response.into_inner();

            let root_summary =
                self.root_summary_for_values(&resp.root_hash, &resp.root_accumulator);
            self.update_root_hash(route.node_name.clone(), resp.root_hash.clone());
            self.update_root_accumulator(route.node_name.clone(), resp.root_accumulator.clone());

            let start = Instant::now();
            let verified = self.verify_proof(&resp.proof, &resp.root_hash);
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

        Ok(Response::new(AddResponse {
            success: true,
            message: "Add completed successfully".to_string(),
            combined_proof,
            combined_root_hash,
            combined_root_accumulator,
        }))
    }

    async fn batch_add(
        &self,
        request: Request<BatchAddRequest>,
    ) -> Result<Response<BatchAddResponse>, Status> {
        let _gate = self.reset_lock.clone().lock_owned().await;
        let req = request.into_inner();
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
                async move {
                    let mut client = manager
                        .get_storager_client(&addr)
                        .await
                        .map_err(|e| Status::internal(e.to_string()))?;
                    let resp = client
                        .batch_add(StoragerBatchAddRequest {
                            items: items.clone(),
                            total_upload_kv_pairs,
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
        }
        Ok(Response::new(BatchAddResponse {
            success: true,
            message: String::new(),
            record_count,
            keyword_pair_count,
        }))
    }

    async fn query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let _gate = self.reset_lock.clone().lock_owned().await;
        let req = request.into_inner();
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
        let _gate = self.reset_lock.clone().lock_owned().await;
        let req = request.into_inner();
        println!("Manager received Delete request for fid: {}", req.fid);

        // Deduplicate keywords to avoid deleting the same element twice
        let unique_keywords: HashSet<String> = req.keywords.into_iter().collect();
        let keyword_count = unique_keywords.len();

        if keyword_count == 0 {
            return Ok(Response::new(DeleteResponse {
                success: false,
                message: "No keywords provided".to_string(),
                combined_proof: vec![],
                combined_root_hash: vec![],
                combined_root_accumulator: vec![],
            }));
        }

        println!("  Processing {} unique keyword(s)", keyword_count);

        // 楠炴儼顢戞径鍕倞閹碘偓閺堝鍙ч柨顔跨槤
        let mut futures = Vec::new();
        for keyword in unique_keywords {
            let manager = self.clone();
            let fid = req.fid.clone();
            futures.push(tokio::spawn(async move {
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
                    ));
                }

                let route = manager
                    .write_route_keyword(&keyword)
                    .ok_or_else(|| Status::internal("No storager available"))?;

                if manager.active_prefix_migration_for_keyword(&keyword).is_some() {
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
                    ));
                }

                // 娴ｈ法鏁ゆ潻鐐村复濮圭姾骞忛崣鏍ь吂閹撮顏?
                let mut client = manager
                    .get_storager_client(&route.addr)
                    .await
                    .map_err(|e| {
                        Status::internal(format!("Failed to connect to storager: {}", e))
                    })?;

                let storager_req = StoragerDeleteRequest {
                    keyword: keyword.clone(),
                    fid,
                };

                let response = client
                    .delete(storager_req)
                    .await
                    .map_err(|e| Status::internal(format!("Storager Delete failed: {}", e)))?;

                let resp = response.into_inner();

                // Verify proof with returned root hash
                let start = Instant::now();
                let verified = manager.verify_proof(&resp.proof, &resp.root_hash);
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
                    ))
                } else {
                    Err(Status::internal(format!(
                        "Proof verification failed for keyword: {}",
                        keyword
                    )))
                }
            }));
        }

        let results = join_all(futures).await;

        // Collect all proofs and root hashes
        let mut proofs = Vec::new();
        let mut root_hashes = Vec::new();
        let mut root_accumulators = Vec::new();
        let mut storager_current_roots: HashMap<String, (Vec<u8>, Vec<u8>)> = HashMap::new();

        for result in results {
            match result {
                Ok(Ok((keyword, node_name, prefix, proof, root_hash, root_accumulator))) => {
                    let root_summary = self.root_summary_for_values(&root_hash, &root_accumulator);
                    storager_current_roots
                        .insert(node_name, (root_hash.clone(), root_accumulator.clone()));
                    self.record_prefix_delete(&keyword, &prefix, root_summary);
                    proofs.push(proof);
                    root_hashes.push(root_hash);
                    root_accumulators.push(root_accumulator);
                }
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(Status::internal(format!("Task join error: {}", e))),
            }
        }

        // Commit all root hash updates atomically
        for (node_name, (final_root, final_root_accumulator)) in storager_current_roots {
            self.update_root_hash(node_name.clone(), final_root);
            self.update_root_accumulator(node_name, final_root_accumulator);
        }

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

        Ok(Response::new(DeleteResponse {
            success: true,
            message: "Delete operation completed successfully".to_string(),
            combined_proof,
            combined_root_hash,
            combined_root_accumulator,
        }))
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        let _gate = self.reset_lock.clone().lock_owned().await;
        let req = request.into_inner();
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
            return Ok(Response::new(UpdateResponse {
                success: true,
                message: "No keywords to update".to_string(),
                combined_proof: Vec::new(),
                combined_root_hash: Vec::new(),
                combined_root_accumulator: Vec::new(),
            }));
        }

        // Phase 1: Delete old keywords (Parallel)
        let mut delete_futures = Vec::new();
        for keyword in unique_old_keywords {
            let manager = self.clone();
            let fid = req.fid.clone();
            delete_futures.push(tokio::spawn(async move {
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
                    ));
                }

                let route = manager
                    .write_route_keyword(&keyword)
                    .ok_or_else(|| Status::internal("No storager available"))?;

                if manager.active_prefix_migration_for_keyword(&keyword).is_some() {
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
                    ));
                }

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
                };

                let response = client
                    .delete(storager_req)
                    .await
                    .map_err(|e| Status::internal(format!("Storager Delete failed: {}", e)))?;

                let resp = response.into_inner();

                let start = Instant::now();
                let verified = manager.verify_proof(&resp.proof, &resp.root_hash);
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
                    ))
                } else {
                    Err(Status::internal(format!(
                        "Delete proof verification failed for keyword: {}",
                        keyword
                    )))
                }
            }));
        }

        let delete_results = join_all(delete_futures).await;

        let mut deleted_operations = Vec::new();
        let mut delete_proofs = Vec::new();
        let mut delete_root_hashes = Vec::new();
        let mut delete_root_accumulators = Vec::new();

        for result in delete_results {
            match result {
                Ok(Ok((
                    keyword,
                    node_name,
                    prefix,
                    storager_addr,
                    old_root_hash,
                    proof,
                    root_hash,
                    root_accumulator,
                ))) => {
                    let root_summary = self.root_summary_for_values(&root_hash, &root_accumulator);
                    self.update_root_hash(node_name.clone(), root_hash.clone());
                    self.update_root_accumulator(node_name.clone(), root_accumulator.clone());
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
                }
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(Status::internal(format!("Task join error: {}", e))),
            }
        }

        // Phase 2: Add new keywords (Parallel)
        let mut add_futures = Vec::new();
        for keyword in unique_new_keywords {
            let manager = self.clone();
            let fid = req.fid.clone();
            add_futures.push(tokio::spawn(async move {
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
                    ));
                }

                let route = manager
                    .write_route_keyword(&keyword)
                    .ok_or_else(|| Status::internal("No storager available"))?;

                if manager.active_prefix_migration_for_keyword(&keyword).is_some() {
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
                    ));
                }

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
                };

                let response = client
                    .add(storager_req)
                    .await
                    .map_err(|e| Status::internal(format!("Storager Add failed: {}", e)))?;

                let resp = response.into_inner();

                let start = Instant::now();
                let verified = manager.verify_proof(&resp.proof, &resp.root_hash);
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
                    ))
                } else {
                    Err(Status::internal(format!(
                        "Add proof verification failed for keyword: {}",
                        keyword
                    )))
                }
            }));
        }

        let add_results = join_all(add_futures).await;

        let mut added_keywords = Vec::new();
        let mut add_proofs = Vec::new();
        let mut add_root_hashes = Vec::new();
        let mut add_root_accumulators = Vec::new();
        let mut rollback_needed = false;
        let mut error_message = String::new();
        let mut migration_happened = false;

        for result in add_results {
            match result {
                Ok(Ok((keyword, node_name, prefix, proof, root_hash, root_accumulator))) => {
                    let root_summary = self.root_summary_for_values(&root_hash, &root_accumulator);
                    self.update_root_hash(node_name.clone(), root_hash.clone());
                    self.update_root_accumulator(node_name.clone(), root_accumulator.clone());
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
                Ok(Err(e)) => {
                    rollback_needed = true;
                    error_message = e.message().to_string();
                    break;
                }
                Err(e) => {
                    rollback_needed = true;
                    error_message = format!("Task join error: {}", e);
                    break;
                }
            }
        }

        if rollback_needed {
            println!(
                "閳跨媴绗? Add operation failed: {}, rolling back...",
                error_message
            );
            self.rollback_update(&req.fid, &deleted_operations, &added_keywords)
                .await;
            return Err(Status::internal(format!(
                "Update failed during add phase: {}",
                error_message
            )));
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

        Ok(Response::new(UpdateResponse {
            success: true,
            message: "Update operation completed successfully".to_string(),
            combined_proof,
            combined_root_hash,
            combined_root_accumulator,
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
                .reset_storage(ResetStorageRequest {})
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

        tokio::spawn(async move {
            let _migration_guard = manager.migration_lock.clone().lock_owned().await;
            if let Err(err) = manager.run_split_migration(split_plan).await {
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
        println!(
            "=== EPRing Split Triggered: parent_prefix='{}', source_node='{}', source_addr='{}' ===",
            split_plan.parent_prefix,
            split_plan.source.node_name,
            split_plan.source.addr,
        );
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

            let prepare_response = source_client
                .prepare_retain_prefix_segment(StoragerPrepareRetainPrefixRequest {
                    prefix: child.prefix.clone(),
                })
                .await
                .map_err(|e| Status::internal(format!("PrepareRetainPrefixSegment failed: {}", e)))?
                .into_inner();
            println!(
                "[MIGRATE] prefix={} prepare done in {:?}, segment_bytes={}",
                child.prefix,
                child_start.elapsed(),
                prepare_response.segment.len()
            );

            self.update_root_hash(
                split_plan.source.node_name.clone(),
                prepare_response.root_hash.clone(),
            );
            self.update_root_accumulator(
                split_plan.source.node_name.clone(),
                prepare_response.root_accumulator.clone(),
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
            let import_response = target_client
                .import_prefix_segment(StoragerImportPrefixRequest {
                    segment: prepare_response.segment,
                })
                .await
                .map_err(|e| Status::internal(format!("ImportPrefixSegment failed: {}", e)))?
                .into_inner();
            println!(
                "[MIGRATE] prefix={} import done in {:?}",
                child.prefix,
                import_start.elapsed()
            );

            self.update_root_hash(child.node_name.clone(), import_response.root_hash.clone());
            self.update_root_accumulator(
                child.node_name.clone(),
                import_response.root_accumulator.clone(),
            );
            current_roots.insert(
                child.node_name.clone(),
                (import_response.root_hash, import_response.root_accumulator),
            );

            let confirm_start = std::time::Instant::now();
            let confirm_response = source_client
                .confirm_prefix_migration(StoragerConfirmPrefixMigrationRequest {
                    prefix: child.prefix.clone(),
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

            self.update_root_hash(
                split_plan.source.node_name.clone(),
                confirm_response.root_hash.clone(),
            );
            self.update_root_accumulator(
                split_plan.source.node_name.clone(),
                confirm_response.root_accumulator.clone(),
            );
            current_roots.insert(
                split_plan.source.node_name.clone(),
                (
                    confirm_response.root_hash,
                    confirm_response.root_accumulator,
                ),
            );
        }

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
            self.update_prefix_summary(&child.prefix, root_summary);
        }

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
        if let Some(state) = self
            .prefix_migrations
            .write()
            .unwrap()
            .get_mut(&split_plan.parent_prefix)
        {
            state.confirmed = true;
        }
        Ok(())
    }

    async fn replay_pending_operations(&self, prefix: &str) -> Result<Vec<String>, String> {
        let pending_operations = {
            let mut migrations = self.prefix_migrations.write().unwrap();
            let state = migrations
                .get_mut(prefix)
                .ok_or_else(|| format!("prefix migration not found: {}", prefix))?;
            std::mem::take(&mut state.pending_operations)
        };

        let mut replayed = Vec::with_capacity(pending_operations.len());

        for operation in pending_operations {
            match operation {
                PendingOperation::Add { keyword, fid } => {
                    let route = self
                        .route_keyword(&keyword)
                        .ok_or_else(|| format!("no storager available for keyword: {}", keyword))?;

                    let mut client = self
                        .get_storager_client(&route.addr)
                        .await
                        .map_err(|e| format!("failed to connect to storager: {}", e))?;

                    let resp = client
                        .add(StoragerAddRequest {
                            keyword: keyword.clone(),
                            fid,
                            total_upload_kv_pairs: 0,
                        })
                        .await
                        .map_err(|e| format!("storager add failed for {}: {}", keyword, e))?
                        .into_inner();

                    if !self.verify_proof(&resp.proof, &resp.root_hash) {
                        return Err(format!(
                            "proof verification failed while replaying add for {}",
                            keyword
                        ));
                    }

                    let root_summary =
                        self.root_summary_for_values(&resp.root_hash, &resp.root_accumulator);
                    self.update_root_hash(route.node_name.clone(), resp.root_hash.clone());
                    self.update_root_accumulator(
                        route.node_name.clone(),
                        resp.root_accumulator.clone(),
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
                    let final_root_hash = self
                        .get_root_hash(&final_route.node_name)
                        .unwrap_or_default();
                    let final_root_accumulator = self
                        .get_root_accumulator(&final_route.node_name)
                        .unwrap_or_default();
                    self.update_prefix_summary(
                        &final_route.prefix,
                        self.root_summary_for_values(&final_root_hash, &final_root_accumulator),
                    );
                    replayed.push(format!("add:{}", keyword));
                }
                PendingOperation::Delete { keyword, fid } => {
                    let route = self
                        .route_keyword(&keyword)
                        .ok_or_else(|| format!("no storager available for keyword: {}", keyword))?;

                    let mut client = self
                        .get_storager_client(&route.addr)
                        .await
                        .map_err(|e| format!("failed to connect to storager: {}", e))?;

                    let resp = client
                        .delete(StoragerDeleteRequest {
                            keyword: keyword.clone(),
                            fid,
                        })
                        .await
                        .map_err(|e| format!("storager delete failed for {}: {}", keyword, e))?
                        .into_inner();

                    if !self.verify_proof(&resp.proof, &resp.root_hash) {
                        return Err(format!(
                            "proof verification failed while replaying delete for {}",
                            keyword
                        ));
                    }

                    let root_summary =
                        self.root_summary_for_values(&resp.root_hash, &resp.root_accumulator);
                    self.update_root_hash(route.node_name.clone(), resp.root_hash.clone());
                    self.update_root_accumulator(
                        route.node_name.clone(),
                        resp.root_accumulator.clone(),
                    );
                    self.record_prefix_delete(&keyword, &route.prefix, root_summary.clone());
                    self.update_prefix_summary(&route.prefix, root_summary);
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

                    let mut client = self
                        .get_storager_client(&route.addr)
                        .await
                        .map_err(|e| format!("failed to connect to storager: {}", e))?;

                    let delete_resp = client
                        .delete(StoragerDeleteRequest {
                            keyword: keyword.clone(),
                            fid: old_fid,
                        })
                        .await
                        .map_err(|e| format!("storager delete failed for {}: {}", keyword, e))?
                        .into_inner();

                    if !self.verify_proof(&delete_resp.proof, &delete_resp.root_hash) {
                        return Err(format!(
                            "proof verification failed while replaying update-delete for {}",
                            keyword
                        ));
                    }

                    let delete_summary = self.root_summary_for_values(
                        &delete_resp.root_hash,
                        &delete_resp.root_accumulator,
                    );
                    self.update_root_hash(route.node_name.clone(), delete_resp.root_hash.clone());
                    self.update_root_accumulator(
                        route.node_name.clone(),
                        delete_resp.root_accumulator.clone(),
                    );
                    self.record_prefix_delete(&keyword, &route.prefix, delete_summary);

                    let add_resp = client
                        .add(StoragerAddRequest {
                            keyword: keyword.clone(),
                            fid: new_fid,
                            total_upload_kv_pairs: 0,
                        })
                        .await
                        .map_err(|e| format!("storager add failed for {}: {}", keyword, e))?
                        .into_inner();

                    if !self.verify_proof(&add_resp.proof, &add_resp.root_hash) {
                        return Err(format!(
                            "proof verification failed while replaying update-add for {}",
                            keyword
                        ));
                    }

                    let root_summary = self
                        .root_summary_for_values(&add_resp.root_hash, &add_resp.root_accumulator);
                    self.update_root_hash(route.node_name.clone(), add_resp.root_hash.clone());
                    self.update_root_accumulator(
                        route.node_name.clone(),
                        add_resp.root_accumulator.clone(),
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
                    let final_root_hash = self
                        .get_root_hash(&final_route.node_name)
                        .unwrap_or_default();
                    let final_root_accumulator = self
                        .get_root_accumulator(&final_route.node_name)
                        .unwrap_or_default();
                    self.update_prefix_summary(
                        &final_route.prefix,
                        self.root_summary_for_values(&final_root_hash, &final_root_accumulator),
                    );
                    replayed.push(format!("update:{}", keyword));
                }
            }
        }

        Ok(replayed)
    }
    /// 閸楁洖鍙ч柨顔跨槤閺屻儴顕?
    pub(crate) async fn query_single_keyword(
        &self,
        keyword: &str,
    ) -> Result<Response<QueryResponse>, Status> {
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
        };

        let response = client
            .query(storager_req)
            .await
            .map_err(|e| Status::internal(format!("Storager Query failed: {}", e)))?;

        let resp = response.into_inner();
        self.update_root_accumulator(route.node_name.clone(), resp.root_accumulator.clone());
        self.update_root_hash(route.node_name.clone(), resp.root_hash.clone());
        let root_hash = resp.root_hash.clone();
        let root_summary = self.root_summary_for_values(&root_hash, &resp.root_accumulator);
        self.update_prefix_summary(&route.prefix, root_summary);

        // Verify proof
        let start = Instant::now();
        let verified = self.verify_proof(&resp.proof, &root_hash)
            && self
                .verifier
                .verify_query_result_fids(&resp.proof, &resp.fids);
        let duration = start.elapsed();
        println!("[METRIC] Proof Verification (Query): {:?}", duration);

        let mut node_root_hashes = HashMap::new();
        node_root_hashes.insert(route.node_name.clone(), root_hash.clone());
        let mut node_root_accumulators = HashMap::new();
        node_root_accumulators.insert(route.node_name, resp.root_accumulator.clone());

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
        }))
    }

    /// 鐢啫鐨甸崙鑺ユ殶閺屻儴顕?
    pub(crate) async fn query_boolean_function(
        &self,
        func: &str,
    ) -> Result<Response<QueryResponse>, Status> {
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
        let mut futures = Vec::new();
        for keyword in keywords {
            let manager = self.clone();
            futures.push(tokio::spawn(async move {
                let route = manager
                    .route_keyword(&keyword)
                    .ok_or_else(|| Status::internal("No storager available"))?;

                let mut client = manager
                    .get_storager_client(&route.addr)
                    .await
                    .map_err(|e| {
                        Status::internal(format!("Failed to connect to storager: {}", e))
                    })?;

                let storager_req = StoragerQueryRequest {
                    keyword: keyword.clone(),
                };

                let response = client
                    .query(storager_req)
                    .await
                    .map_err(|e| Status::internal(format!("Storager Query failed: {}", e)))?;

                let resp = response.into_inner();
                manager.update_root_hash(route.node_name.clone(), resp.root_hash.clone());
                let root_hash = resp.root_hash.clone();

                if !manager.verify_proof(&resp.proof, &root_hash)
                    || !manager
                        .verifier
                        .verify_query_result_fids(&resp.proof, &resp.fids)
                {
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
                ))
            }));
        }

        let results = join_all(futures).await;

        let aggregation_start = Instant::now();
        let mut keyword_results = HashMap::new();
        let mut keyword_leaf_data = HashMap::new();
        let mut node_root_hashes = HashMap::new();
        let mut node_root_accumulators = HashMap::new();

        for result in results {
            match result {
                Ok(Ok((keyword, fids, proof, node_name, prefix, root_hash, root_accumulator))) => {
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
                    self.update_root_accumulator(node_name.clone(), root_accumulator.clone());
                    let root_summary = self.root_summary_for_values(&root_hash, &root_accumulator);
                    self.update_prefix_summary(&prefix, root_summary);
                    node_root_hashes.insert(node_name.clone(), root_hash);
                    node_root_accumulators.insert(node_name, root_accumulator);
                }
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(Status::internal(format!("Task join error: {}", e))),
            }
        }
        let proof_aggregation_duration = aggregation_start.elapsed();

        // Build proof tree and evaluate expression
        let expected_result_fids = sorted_vec_from_set(&expr.evaluate(&keyword_results));
        let proof_generation_start = Instant::now();
        let (combined_proof, root_hash, result_fids) = match self.set_proof_mode() {
            SetProofMode::Polynomial => {
                let (tree, result_set) = build_polynomial_proof_tree(&expr, &keyword_leaf_data)?;
                let result_fids = sorted_vec_from_set(&result_set);

                if expected_result_fids != result_fids {
                    return Err(Status::internal(
                        "Polynomial proof tree result does not match boolean evaluation result",
                    ));
                }

                let aggregate_proof = PolynomialIntersectionAggregateProof {
                    expr: func.to_string(),
                    result_fids: result_fids.clone(),
                    root: tree.clone(),
                };

                let combined_proof = encode_polynomial_intersection_proof(&aggregate_proof)
                    .map_err(Status::internal)?;
                let root_hash = polynomial_intersection_root_hash(&combined_proof);
                (combined_proof, root_hash, result_fids)
            }
            SetProofMode::Accumulator => {
                let (tree, result_set) = build_accumulator_proof_tree(&expr, &keyword_leaf_data)?;
                let result_fids = sorted_vec_from_set(&result_set);

                if expected_result_fids != result_fids {
                    return Err(Status::internal(
                        "Accumulator proof tree result does not match boolean evaluation result",
                    ));
                }

                let aggregate_proof = AccumulatorSetOperationAggregateProof {
                    expr: func.to_string(),
                    result_fids: result_fids.clone(),
                    root: tree,
                };

                let combined_proof = encode_accumulator_set_operation_proof(&aggregate_proof)
                    .map_err(Status::internal)?;
                let root_hash = common::accumulator_set_operation_root_hash(&combined_proof);
                (combined_proof, root_hash, result_fids)
            }
        };
        let proof_generation_duration = proof_generation_start.elapsed();
        let proof_aggregation_ms = proof_aggregation_duration.as_secs_f64() * 1000.0;
        let proof_generation_ms = proof_generation_duration.as_secs_f64() * 1000.0;
        self.record_boolean_query_proof_generation(
            proof_aggregation_duration + proof_generation_duration,
        );

        self.record_boolean_query(node_root_hashes.len());

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
        println!("棣冩敡 Rolling back update operation for fid: {}", fid);

        // Rollback Phase 1: Re-add deleted keywords
        for (keyword, prefix, node_name, storager_addr, _old_root_hash) in deleted_operations {
            println!("  Re-adding deleted keyword: {}", keyword);

            if let Ok(mut client) = self.get_storager_client(storager_addr).await {
                let storager_req = StoragerAddRequest {
                    keyword: keyword.clone(),
                    fid: fid.to_string(),
                    total_upload_kv_pairs: 0,
                };

                match client.add(storager_req).await {
                    Ok(response) => {
                        let resp = response.into_inner();
                        if self.verify_proof(&resp.proof, &resp.root_hash) {
                            let root_summary = self
                                .root_summary_for_values(&resp.root_hash, &resp.root_accumulator);
                            self.update_root_hash(node_name.clone(), resp.root_hash);
                            self.update_root_accumulator(node_name.clone(), resp.root_accumulator);
                            if let Some(split_plan) =
                                self.record_prefix_insert(keyword, prefix, node_name, root_summary)
                            {
                                self.schedule_split_migration(split_plan);
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
                    let storager_req = StoragerDeleteRequest {
                        keyword: keyword.clone(),
                        fid: fid.to_string(),
                    };

                    match client.delete(storager_req).await {
                        Ok(response) => {
                            let resp = response.into_inner();
                            if self.verify_proof(&resp.proof, &resp.root_hash) {
                                let root_summary = self.root_summary_for_values(
                                    &resp.root_hash,
                                    &resp.root_accumulator,
                                );
                                self.update_root_hash(route.node_name.clone(), resp.root_hash);
                                self.update_root_accumulator(
                                    route.node_name.clone(),
                                    resp.root_accumulator,
                                );
                                self.record_prefix_delete(keyword, &route.prefix, root_summary);
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

        println!("棣冩敡 Rollback completed");
    }
}
