use crate::manager::Manager;
use common::parse_boolean_expr;
use common::rpc::{
    manager_service_server::ManagerService, AddRequest, AddResponse, DeleteRequest, DeleteResponse,
    QueryRequest, QueryResponse, StoragerAddRequest, StoragerDeleteRequest, StoragerQueryRequest,
    UpdateRequest, UpdateResponse,
};
use std::collections::{HashMap, HashSet};
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl ManagerService for Manager {
    async fn add(&self, request: Request<AddRequest>) -> Result<Response<AddResponse>, Status> {
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
            }));
        }

        println!("  Processing {} unique keyword(s)", keyword_count);

        // 收集所有证明和根哈希
        let mut proofs = Vec::new();
        let mut root_hashes = Vec::new();

        // Process each unique keyword
        for keyword in &unique_keywords {
            let (node_name, storager_addr) = self
                .get_storager_for_keyword(keyword)
                .ok_or_else(|| Status::internal("No storager available"))?;

            // 使用连接池获取客户端
            let mut client = self
                .get_storager_client(&storager_addr)
                .await
                .map_err(|e| Status::internal(format!("Failed to connect to storager: {}", e)))?;

            let storager_req = StoragerAddRequest {
                keyword: keyword.clone(),
                fid: req.fid.clone(),
            };

            let response = client
                .add(storager_req)
                .await
                .map_err(|e| Status::internal(format!("Storager Add failed: {}", e)))?;

            let resp = response.into_inner();

            // Verify proof with returned root hash
            // The proof is based on the state AFTER adding this keyword
            if self.verify_proof(&resp.proof, &resp.root_hash) {
                self.update_root_hash(node_name, resp.root_hash.clone());
                proofs.push(resp.proof);
                root_hashes.push(resp.root_hash);
            } else {
                return Ok(Response::new(AddResponse {
                    success: false,
                    message: format!("Proof verification failed for keyword: {}", keyword),
                    combined_proof: vec![],
                    combined_root_hash: vec![],
                }));
            }
        }

        // 合并所有证明
        let combined_proof = self.combine_proofs(&proofs);
        let combined_root_hash = root_hashes.into_iter().flatten().collect();

        Ok(Response::new(AddResponse {
            success: true,
            message: "Add operation completed successfully".to_string(),
            combined_proof,
            combined_root_hash,
        }))
    }

    async fn query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let req = request.into_inner();
        println!("Manager received Query request");

        match req.query_type {
            Some(common::rpc::query_request::QueryType::Keyword(keyword)) => {
                // 单关键词查询
                self.query_single_keyword(&keyword).await
            }
            Some(common::rpc::query_request::QueryType::BooleanFunction(func)) => {
                // 布尔函数查询
                self.query_boolean_function(&func).await
            }
            None => Err(Status::invalid_argument("No query type specified")),
        }
    }

    async fn delete(
        &self,
        request: Request<DeleteRequest>,
    ) -> Result<Response<DeleteResponse>, Status> {
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
            }));
        }

        println!("  Processing {} unique keyword(s)", keyword_count);

        // 收集所有证明和根哈希
        let mut proofs = Vec::new();
        let mut root_hashes = Vec::new();

        // Track current root hash for each storager (updated after each delete)
        let mut storager_current_roots: HashMap<String, Vec<u8>> = HashMap::new();

        // Process each unique keyword
        for keyword in &unique_keywords {
            let (node_name, storager_addr) = self
                .get_storager_for_keyword(keyword)
                .ok_or_else(|| Status::internal("No storager available"))?;

            // 使用连接池获取客户端
            let mut client = self
                .get_storager_client(&storager_addr)
                .await
                .map_err(|e| Status::internal(format!("Failed to connect to storager: {}", e)))?;

            let storager_req = StoragerDeleteRequest {
                keyword: keyword.clone(),
                fid: req.fid.clone(),
            };

            let response = client
                .delete(storager_req)
                .await
                .map_err(|e| Status::internal(format!("Storager Delete failed: {}", e)))?;

            let resp = response.into_inner();

            // Verify proof with returned root hash (post-delete proof)
            if self.verify_proof(&resp.proof, &resp.root_hash) {
                // Proof verified: the key state is valid in the new tree
                // Update current root for this storager (storager's tree has changed)
                storager_current_roots.insert(node_name.clone(), resp.root_hash.clone());
                proofs.push(resp.proof);
                root_hashes.push(resp.root_hash);
            } else {
                return Ok(Response::new(DeleteResponse {
                    success: false,
                    message: format!("Proof verification failed for keyword: {}", keyword),
                    combined_proof: vec![],
                    combined_root_hash: vec![],
                }));
            }
        }

        // Commit all root hash updates atomically
        for (node_name, final_root) in storager_current_roots {
            self.update_root_hash(node_name, final_root);
        }

        // 合并所有证明
        println!("🔍 Delete proof合并: {} proofs", proofs.len());
        for (i, proof) in proofs.iter().enumerate() {
            println!("  - proof[{}]: {} bytes", i, proof.len());
        }

        let combined_proof = self.combine_proofs(&proofs);
        let combined_root_hash = root_hashes.into_iter().flatten().collect();

        println!("✅ Delete combined_proof: {} bytes", combined_proof.len());

        Ok(Response::new(DeleteResponse {
            success: true,
            message: "Delete operation completed successfully".to_string(),
            combined_proof,
            combined_root_hash,
        }))
    }

    async fn update(
        &self,
        request: Request<UpdateRequest>,
    ) -> Result<Response<UpdateResponse>, Status> {
        let req = request.into_inner();
        println!("Manager received Update request for fid: {}", req.fid);

        // Deduplicate old and new keywords
        let unique_old_keywords: HashSet<String> = req.old_keywords.into_iter().collect();
        let unique_new_keywords: HashSet<String> = req.new_keywords.into_iter().collect();

        println!(
            "  Deleting {} unique old keyword(s)",
            unique_old_keywords.len()
        );
        println!(
            "  Adding {} unique new keyword(s)",
            unique_new_keywords.len()
        );

        // 提前返回检查
        if unique_old_keywords.is_empty() && unique_new_keywords.is_empty() {
            println!("🔚 Update: No keywords to update, returning empty proof");
            return Ok(Response::new(UpdateResponse {
                success: true,
                message: "No keywords to update".to_string(),
                combined_proof: Vec::new(),
                combined_root_hash: Vec::new(),
            }));
        }

        // Phase 1: Delete old keywords and track deleted operations for rollback
        let mut deleted_operations: Vec<(String, String, Vec<u8>)> = Vec::new(); // (keyword, storager_addr, old_root_hash)
        let mut delete_proofs: Vec<Vec<u8>> = Vec::new();
        let mut delete_root_hashes: Vec<Vec<u8>> = Vec::new();

        for keyword in &unique_old_keywords {
            let (node_name, storager_addr) = self
                .get_storager_for_keyword(keyword)
                .ok_or_else(|| Status::internal("No storager available"))?;

            // Save old root hash for potential rollback
            let old_root_hash = self
                .root_hashes
                .read()
                .expect("Failed to acquire read lock on root_hashes")
                .get(&node_name)
                .cloned()
                .unwrap_or_default();

            let mut client = self
                .get_storager_client(&storager_addr)
                .await
                .map_err(|e| Status::internal(format!("Failed to connect to storager: {}", e)))?;

            let storager_req = StoragerDeleteRequest {
                keyword: keyword.clone(),
                fid: req.fid.clone(),
            };

            let response = client
                .delete(storager_req)
                .await
                .map_err(|e| Status::internal(format!("Storager Delete failed: {}", e)))?;

            let resp = response.into_inner();

            // Verify proof with returned root hash (post-delete proof)
            if self.verify_proof(&resp.proof, &resp.root_hash) {
                // Proof verified, update to post-delete root hash
                self.update_root_hash(node_name.clone(), resp.root_hash.clone());
                deleted_operations.push((keyword.clone(), storager_addr, old_root_hash));
                delete_proofs.push(resp.proof);
                delete_root_hashes.push(resp.root_hash);
            } else {
                println!(
                    "🔚 Update: Delete proof verification failed for keyword {}",
                    keyword
                );
                return Ok(Response::new(UpdateResponse {
                    success: false,
                    message: format!("Delete proof verification failed for keyword: {}", keyword),
                    combined_proof: Vec::new(),
                    combined_root_hash: Vec::new(),
                }));
            }
        }

        // Phase 2: Add new keywords with rollback on failure
        let mut added_keywords: Vec<String> = Vec::new();
        let mut add_proofs: Vec<Vec<u8>> = Vec::new();
        let mut add_root_hashes: Vec<Vec<u8>> = Vec::new();

        for keyword in &unique_new_keywords {
            let (node_name, storager_addr) = self
                .get_storager_for_keyword(keyword)
                .ok_or_else(|| Status::internal("No storager available"))?;

            let mut client = self
                .get_storager_client(&storager_addr)
                .await
                .map_err(|e| {
                    // Rollback: re-add deleted keywords
                    println!("⚠️  Add operation failed, initiating rollback...");
                    Status::internal(format!("Failed to connect to storager: {}", e))
                })?;

            let storager_req = StoragerAddRequest {
                keyword: keyword.clone(),
                fid: req.fid.clone(),
            };

            match client.add(storager_req).await {
                Ok(response) => {
                    let resp = response.into_inner();
                    // Strict verification: proof must be valid
                    let is_valid = self.verify_proof(&resp.proof, &resp.root_hash);

                    if is_valid {
                        self.update_root_hash(node_name, resp.root_hash.clone());
                        added_keywords.push(keyword.clone());
                        add_proofs.push(resp.proof);
                        add_root_hashes.push(resp.root_hash);
                    } else {
                        // Rollback on proof verification failure
                        println!(
                            "🔚 Update: Add proof verification failed for keyword {}, rolling back",
                            keyword
                        );
                        self.rollback_update(&req.fid, &deleted_operations, &added_keywords)
                            .await;
                        return Ok(Response::new(UpdateResponse {
                            success: false,
                            message: format!(
                                "Add proof verification failed for keyword: {}",
                                keyword
                            ),
                            combined_proof: Vec::new(),
                            combined_root_hash: Vec::new(),
                        }));
                    }
                }
                Err(e) => {
                    // Rollback on error
                    println!("⚠️  Add operation error, rolling back...");
                    self.rollback_update(&req.fid, &deleted_operations, &added_keywords)
                        .await;
                    return Err(Status::internal(format!("Storager Add failed: {}", e)));
                }
            }
        }

        // 合并所有证明（删除阶段 + 添加阶段）
        let delete_count = delete_proofs.len();
        let add_count = add_proofs.len();

        let mut all_proofs = delete_proofs;
        all_proofs.extend(add_proofs);
        let mut all_root_hashes = delete_root_hashes;
        all_root_hashes.extend(add_root_hashes);

        println!(
            "🔍 Update proof合并: delete_proofs={}, add_proofs={}, total_proofs={}",
            delete_count,
            add_count,
            all_proofs.len()
        );
        for (i, proof) in all_proofs.iter().enumerate() {
            println!("  - proof[{}]: {} bytes", i, proof.len());
        }

        let combined_proof = self.combine_proofs(&all_proofs);
        let combined_root_hash = all_root_hashes.into_iter().flatten().collect();

        println!("✅ Update combined_proof: {} bytes", combined_proof.len());

        Ok(Response::new(UpdateResponse {
            success: true,
            message: "Update operation completed successfully".to_string(),
            combined_proof,
            combined_root_hash,
        }))
    }
}

impl Manager {
    /// 单关键词查询
    pub(crate) async fn query_single_keyword(
        &self,
        keyword: &str,
    ) -> Result<Response<QueryResponse>, Status> {
        println!("  Query type: Single keyword '{}'", keyword);

        let (node_name, storager_addr) = self
            .get_storager_for_keyword(keyword)
            .ok_or_else(|| Status::internal("No storager available"))?;

        // 使用连接池获取客户端
        let mut client = self
            .get_storager_client(&storager_addr)
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

        // Get root hash for this storager
        let root_hash = self
            .root_hashes
            .read()
            .expect("Failed to acquire read lock on root_hashes")
            .get(&node_name)
            .cloned()
            .unwrap_or_default();

        // Verify proof
        let verified = self.verify_proof(&resp.proof, &root_hash);

        Ok(Response::new(QueryResponse {
            fids: resp.fids,
            proof: resp.proof,
            root_hash,
            verified,
        }))
    }

    /// 布尔函数查询
    pub(crate) async fn query_boolean_function(
        &self,
        func: &str,
    ) -> Result<Response<QueryResponse>, Status> {
        println!("  Query type: Boolean function '{}'", func);

        // 1. 解析布尔表达式
        let expr = parse_boolean_expr(func).map_err(|e| {
            Status::invalid_argument(format!("Failed to parse boolean expression: {}", e))
        })?;

        println!("  Parsed expression: {}", expr.to_string());

        // 2. 获取所有关键词
        let keywords = expr.get_keywords();
        println!("  Keywords: {:?}", keywords);

        // 3. 并发查询所有关键词
        let mut keyword_results = HashMap::new();
        let mut all_proofs = Vec::new();

        for keyword in keywords.iter() {
            let (node_name, storager_addr) = self
                .get_storager_for_keyword(keyword)
                .ok_or_else(|| Status::internal("No storager available"))?;

            // 使用连接池获取客户端
            let mut client = self
                .get_storager_client(&storager_addr)
                .await
                .map_err(|e| Status::internal(format!("Failed to connect to storager: {}", e)))?;

            let storager_req = StoragerQueryRequest {
                keyword: keyword.clone(),
            };

            let response = client
                .query(storager_req)
                .await
                .map_err(|e| Status::internal(format!("Storager Query failed: {}", e)))?;

            let resp = response.into_inner();

            // Get root hash for this storager
            let root_hash = self
                .root_hashes
                .read()
                .expect("Failed to acquire read lock on root_hashes")
                .get(&node_name)
                .cloned()
                .unwrap_or_default();

            // Verify individual proof
            if !self.verify_proof(&resp.proof, &root_hash) {
                return Err(Status::internal(format!(
                    "Proof verification failed for keyword: {}",
                    keyword
                )));
            }

            // 存储查询结果
            let fid_set: HashSet<String> = resp.fids.into_iter().collect();
            keyword_results.insert(keyword.clone(), fid_set);

            // 收集证明
            all_proofs.push(resp.proof);

            println!(
                "    '{}' -> {} files",
                keyword,
                keyword_results.get(keyword).map_or(0, |s| s.len())
            );
        }

        // 4. 对布尔表达式求值
        let result_set = expr.evaluate(&keyword_results);
        let result_fids: Vec<String> = result_set.into_iter().collect();

        println!("  Final result: {} files", result_fids.len());

        // 5. 生成组合证明
        let combined_proof = self.combine_proofs(&all_proofs);

        // 6. 使用第一个 storager 的 root hash 作为代表
        let root_hash = self
            .root_hashes
            .read()
            .expect("Failed to acquire read lock on root_hashes")
            .values()
            .next()
            .cloned()
            .unwrap_or_default();

        Ok(Response::new(QueryResponse {
            fids: result_fids,
            proof: combined_proof,
            root_hash,
            verified: true, // 已经验证过各个子查询的证明
        }))
    }

    /// 回滚 Update 操作
    ///
    /// 当 Update 操作失败时,需要:
    /// 1. 重新添加已删除的关键词
    /// 2. 删除已添加的新关键词
    async fn rollback_update(
        &self,
        fid: &str,
        deleted_operations: &[(String, String, Vec<u8>)], // (keyword, storager_addr, old_root_hash)
        added_keywords: &[String],
    ) {
        println!("🔄 Rolling back update operation for fid: {}", fid);

        // Rollback Phase 1: Re-add deleted keywords
        for (keyword, storager_addr, _old_root_hash) in deleted_operations {
            println!("  Re-adding deleted keyword: {}", keyword);

            if let Ok(mut client) = self.get_storager_client(storager_addr).await {
                let storager_req = StoragerAddRequest {
                    keyword: keyword.clone(),
                    fid: fid.to_string(),
                };

                match client.add(storager_req).await {
                    Ok(response) => {
                        let resp = response.into_inner();
                        if let Some((node_name, _)) = self.get_storager_for_keyword(keyword) {
                            if self.verify_proof(&resp.proof, &resp.root_hash) {
                                self.update_root_hash(node_name, resp.root_hash);
                                println!("  ✅ Re-added: {}", keyword);
                            }
                        }
                    }
                    Err(e) => {
                        println!("  ❌ Failed to re-add {}: {}", keyword, e);
                    }
                }
            }
        }

        // Rollback Phase 2: Remove added keywords
        for keyword in added_keywords {
            println!("  Removing added keyword: {}", keyword);

            if let Some((_node_name, storager_addr)) = self.get_storager_for_keyword(keyword) {
                if let Ok(mut client) = self.get_storager_client(&storager_addr).await {
                    let storager_req = StoragerDeleteRequest {
                        keyword: keyword.clone(),
                        fid: fid.to_string(),
                    };

                    match client.delete(storager_req).await {
                        Ok(response) => {
                            let resp = response.into_inner();
                            if let Some((node_name, _)) = self.get_storager_for_keyword(keyword) {
                                if self.verify_proof(&resp.proof, &resp.root_hash) {
                                    self.update_root_hash(node_name, resp.root_hash);
                                    println!("  ✅ Removed: {}", keyword);
                                }
                            }
                        }
                        Err(e) => {
                            println!("  ❌ Failed to remove {}: {}", keyword, e);
                        }
                    }
                }
            }
        }

        println!("🔄 Rollback completed");
    }
}
