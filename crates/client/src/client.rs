use common::rpc::{
    manager_service_client::ManagerServiceClient, AddRequest, BatchAddRecord, BatchAddRequest,
    DeleteRequest, QueryRequest, ResetSystemRequest, UpdateRequest,
};
use common::{
    is_accumulator_set_operation_proof, is_polynomial_intersection_proof, parse_boolean_expr,
    AdsMode, BooleanExpr, ProofVerifier, SetProofMode,
};
use std::time::Duration;
use tokio::sync::OnceCell;
use tonic::transport::{Channel, Endpoint};
use xxhash_rust::xxh3::xxh3_128;

fn env_duration_secs(key: &str, default_secs: u64) -> Duration {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default_secs))
}

#[derive(Debug, Clone)]
pub struct QueryKeywordMetrics {
    pub result_count: usize,
    pub proof_size_bytes: usize,
    pub verification_latency: Duration,
    pub manager_proof_aggregation_latency: Duration,
    pub manager_set_operation_proof_generation_latency: Duration,
    pub route_mode: String,
    pub persistence_mode: String,
}

#[derive(Debug, Clone)]
pub struct RunMetadata {
    pub dataset: String,
    pub concurrency: u32,
    pub total_uploads: u32,
    pub total_queries: u32,
    pub total_updates: u32,
}

impl RunMetadata {
    pub fn new(
        dataset: impl Into<String>,
        concurrency: u32,
        total_uploads: u32,
        total_queries: u32,
        total_updates: u32,
    ) -> Self {
        Self {
            dataset: dataset.into(),
            concurrency,
            total_uploads,
            total_queries,
            total_updates,
        }
    }
}

/// Client 结构，封装与 Manager 的交互
pub struct Client {
    manager_addr: String,
    channel: OnceCell<Channel>,
    verifier: ProofVerifier,
    set_proof_mode: SetProofMode,
}

impl Client {
    /// 创建新的 Client
    pub fn new(manager_addr: String, ads_mode: AdsMode, set_proof_mode: SetProofMode) -> Self {
        Client {
            manager_addr,
            channel: OnceCell::new(),
            verifier: ProofVerifier::new(ads_mode),
            set_proof_mode,
        }
    }

    fn verify_expected_set_proof_mode(&self, proof: &[u8]) -> bool {
        match self.set_proof_mode {
            SetProofMode::Polynomial => is_polynomial_intersection_proof(proof),
            SetProofMode::Accumulator => is_accumulator_set_operation_proof(proof),
        }
    }

    fn digest_from_keyword(keyword: &str) -> [u8; 16] {
        xxh3_128(keyword.as_bytes()).to_le_bytes()
    }

    fn keyword_to_hex(keyword: &str) -> String {
        let digest = Self::digest_from_keyword(keyword);
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            out.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
            out.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
        }
        out
    }

    fn encode_boolean_expr(expr: &BooleanExpr) -> BooleanExpr {
        match expr {
            BooleanExpr::Keyword(keyword) => BooleanExpr::Keyword(Self::keyword_to_hex(keyword)),
            BooleanExpr::And(left, right) => BooleanExpr::And(
                Box::new(Self::encode_boolean_expr(left)),
                Box::new(Self::encode_boolean_expr(right)),
            ),
            BooleanExpr::Or(left, right) => BooleanExpr::Or(
                Box::new(Self::encode_boolean_expr(left)),
                Box::new(Self::encode_boolean_expr(right)),
            ),
            BooleanExpr::Not(expr) => BooleanExpr::Not(Box::new(Self::encode_boolean_expr(expr))),
        }
    }

    pub fn encode_upload_keywords(&self, keywords: Vec<String>) -> Vec<String> {
        keywords
            .into_iter()
            .map(|keyword| Self::keyword_to_hex(&keyword))
            .collect()
    }

    pub fn encode_update_keywords(&self, keywords: Vec<String>) -> Vec<String> {
        self.encode_upload_keywords(keywords)
    }

    pub fn encode_query_expression(&self, query: &str) -> Result<String, String> {
        let expr = parse_boolean_expr(query)?;
        Ok(Self::encode_boolean_expr(&expr).to_string())
    }

    pub fn encode_boolean_query_expression(&self, expr: &BooleanExpr) -> String {
        Self::encode_boolean_expr(expr).to_string()
    }

    async fn put_file_raw(
        &self,
        fid: String,
        keywords: Vec<String>,
        total_upload_kv_pairs: u32,
        metadata: &RunMetadata,
    ) -> Result<(Duration, String, String), Box<dyn std::error::Error>> {
        let channel = self.get_channel().await?;
        let mut client = ManagerServiceClient::new(channel);

        let request = AddRequest {
            fid,
            keywords,
            total_upload_kv_pairs,
            dataset: metadata.dataset.clone(),
            concurrency: metadata.concurrency,
            total_uploads: metadata.total_uploads,
            total_queries: metadata.total_queries,
            total_updates: metadata.total_updates,
        };

        let response = client.add(request).await?;
        let resp = response.into_inner();
        let mut verification_latency = Duration::from_secs(0);

        if resp.success {
            if !resp.combined_proof.is_empty() {
                let verification_start = std::time::Instant::now();
                if self
                    .verifier
                    .verify(&resp.combined_proof, &resp.combined_root_hash)
                {
                    verification_latency = verification_start.elapsed();
                    println!("✅ Client verification passed (Add)");
                    println!("Put file succeeded: {}", resp.message);
                } else {
                    println!("❌ Client verification failed (Add)");
                    return Err("Client verification failed".into());
                }
            } else {
                println!("⚠️  No proof returned for Add (maybe batch add optimization?)");
                println!("Put file succeeded: {}", resp.message);
            }
        } else {
            println!("Put file failed: {}", resp.message);
        }

        Ok((verification_latency, resp.route_mode, resp.persistence_mode))
    }

    async fn build_channel(&self) -> Result<Channel, Box<dyn std::error::Error>> {
        let use_heavy_profile =
            matches!(self.verifier.ads_mode(), AdsMode::Mpt | AdsMode::AccTree | AdsMode::AccTrie);
        let request_timeout = if use_heavy_profile {
            env_duration_secs("CLIENT_HEAVY_RPC_TIMEOUT_SECS", 3600)
        } else {
            env_duration_secs("CLIENT_RPC_TIMEOUT_SECS", 600)
        };
        let connect_timeout = if use_heavy_profile {
            env_duration_secs("CLIENT_HEAVY_CONNECT_TIMEOUT_SECS", 30)
        } else {
            env_duration_secs("CLIENT_CONNECT_TIMEOUT_SECS", 10)
        };
        let tcp_keepalive = if use_heavy_profile {
            env_duration_secs("CLIENT_HEAVY_TCP_KEEPALIVE_SECS", 300)
        } else {
            env_duration_secs("CLIENT_TCP_KEEPALIVE_SECS", 30)
        };
        let endpoint = Endpoint::from_shared(self.manager_addr.clone())?
            .timeout(request_timeout)
            .connect_timeout(connect_timeout)
            .tcp_keepalive(Some(tcp_keepalive));

        Ok(endpoint.connect().await?)
    }

    /// 获取或创建共享gRPC连接
    async fn get_channel(&self) -> Result<Channel, Box<dyn std::error::Error>> {
        let channel = self
            .channel
            .get_or_try_init(|| self.build_channel())
            .await?;
        Ok(channel.clone())
    }

    /// Put file: add (fid, keywords) to the system
    pub async fn put_file(
        &self,
        fid: String,
        keywords: Vec<String>,
        total_upload_kv_pairs: u32,
        metadata: &RunMetadata,
    ) -> Result<(Duration, String, String), Box<dyn std::error::Error>> {
        self.put_file_raw(fid, keywords, total_upload_kv_pairs, metadata)
            .await
    }

    pub async fn put_file_hex(
        &self,
        fid: String,
        keywords_hex: Vec<String>,
        total_upload_kv_pairs: u32,
        metadata: &RunMetadata,
    ) -> Result<(Duration, String, String), Box<dyn std::error::Error>> {
        self.put_file_raw(fid, keywords_hex, total_upload_kv_pairs, metadata)
            .await
    }

    pub async fn batch_put_files(
        &self,
        records: Vec<(String, Vec<String>)>,
        total_upload_kv_pairs: u32,
        metadata: &RunMetadata,
    ) -> Result<(String, String), Box<dyn std::error::Error>> {
        let channel = self.get_channel().await?;
        let mut client = ManagerServiceClient::new(channel);
        let request = BatchAddRequest {
            total_upload_kv_pairs,
            dataset: metadata.dataset.clone(),
            concurrency: metadata.concurrency,
            total_uploads: metadata.total_uploads,
            total_queries: metadata.total_queries,
            total_updates: metadata.total_updates,
            records: records
                .into_iter()
                .map(|(fid, keywords)| BatchAddRecord { fid, keywords })
                .collect(),
        };
        let response = client.batch_add(request).await?;
        let resp = response.into_inner();
        if resp.success {
            Ok((resp.route_mode, resp.persistence_mode))
        } else {
            Err(resp.message.into())
        }
    }

    /// Query by keyword
    pub async fn query_by_keyword(
        &self,
        keyword: String,
        metadata: &RunMetadata,
    ) -> Result<QueryKeywordMetrics, Box<dyn std::error::Error>> {
        self.query_by_keyword_hex(Self::keyword_to_hex(&keyword), metadata)
            .await
    }

    pub async fn query_by_keyword_hex(
        &self,
        keyword_hex: String,
        metadata: &RunMetadata,
    ) -> Result<QueryKeywordMetrics, Box<dyn std::error::Error>> {
        let channel = self.get_channel().await?;
        let mut client = ManagerServiceClient::new(channel);

        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::Keyword(keyword_hex)),
            dataset: metadata.dataset.clone(),
            concurrency: metadata.concurrency,
            total_uploads: metadata.total_uploads,
            total_queries: metadata.total_queries,
            total_updates: metadata.total_updates,
        };

        let response = client.query(request).await?;
        let resp = response.into_inner();

        // Client-side verification
        let verification_start = std::time::Instant::now();
        if self.verifier.verify(&resp.proof, &resp.root_hash)
            && self
                .verifier
                .verify_query_result_fids(&resp.proof, &resp.fids)
        {
            println!("✅ Client verification passed (Query)");
            println!("Query succeeded, found {} files:", resp.fids.len());
            for fid in &resp.fids {
                println!("  - {}", fid);
            }
            if !resp.root_accumulator.is_empty() {
                println!(
                    "Current AccTrie root accumulator size: {} bytes",
                    resp.root_accumulator.len()
                );
            }
        } else {
            println!("❌ Client verification failed (Query)");
            return Err("Client verification failed".into());
        }

        Ok(QueryKeywordMetrics {
            result_count: resp.fids.len(),
            proof_size_bytes: resp.proof.len(),
            verification_latency: verification_start.elapsed(),
            manager_proof_aggregation_latency: Duration::from_secs_f64(
                resp.manager_proof_aggregation_ms / 1000.0,
            ),
            manager_set_operation_proof_generation_latency: Duration::from_secs_f64(
                resp.manager_set_operation_proof_generation_ms / 1000.0,
            ),
            route_mode: resp.route_mode,
            persistence_mode: resp.persistence_mode,
        })
    }

    /// Query by boolean function
    pub async fn query_by_func(
        &self,
        boolean_func: String,
        metadata: &RunMetadata,
    ) -> Result<QueryKeywordMetrics, Box<dyn std::error::Error>> {
        let encoded = self
            .encode_query_expression(&boolean_func)
            .map_err(|err| format!("failed to encode query expression: {err}"))?;
        self.query_by_func_hex(encoded, metadata).await
    }

    pub async fn query_by_func_hex(
        &self,
        boolean_func_hex: String,
        metadata: &RunMetadata,
    ) -> Result<QueryKeywordMetrics, Box<dyn std::error::Error>> {
        let channel = self.get_channel().await?;
        let mut client = ManagerServiceClient::new(channel);

        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::BooleanFunction(
                boolean_func_hex,
            )),
            dataset: metadata.dataset.clone(),
            concurrency: metadata.concurrency,
            total_uploads: metadata.total_uploads,
            total_queries: metadata.total_queries,
            total_updates: metadata.total_updates,
        };

        let response = client.query(request).await?;
        let resp = response.into_inner();

        if !self.verify_expected_set_proof_mode(&resp.proof) {
            return Err(format!(
                "Unexpected boolean proof type returned by manager; expected {}",
                self.set_proof_mode
            )
            .into());
        }

        // Client-side verification
        let verification_start = std::time::Instant::now();
        if self.verifier.verify(&resp.proof, &resp.root_hash)
            && self
                .verifier
                .verify_query_result_fids(&resp.proof, &resp.fids)
        {
            println!("✅ Client verification passed (Boolean Query)");
            println!("Query succeeded, found {} files:", resp.fids.len());
            for fid in &resp.fids {
                println!("  - {}", fid);
            }

            if !resp.node_root_hashes.is_empty() {
                println!(
                    "Verified against root hashes from {} nodes:",
                    resp.node_root_hashes.len()
                );
                for (node, hash) in resp.node_root_hashes {
                    println!("  - Node {}: {:?}", node, hash);
                }
            }

            if !resp.node_root_accumulators.is_empty() {
                println!(
                    "AccTrie root accumulators from {} nodes:",
                    resp.node_root_accumulators.len()
                );
                for (node, acc) in resp.node_root_accumulators {
                    println!("  - Node {}: {} bytes", node, acc.len());
                }
            }
        } else {
            println!("❌ Client verification failed (Boolean Query)");
            return Err("Client verification failed".into());
        }

        Ok(QueryKeywordMetrics {
            result_count: resp.fids.len(),
            proof_size_bytes: resp.proof.len(),
            verification_latency: verification_start.elapsed(),
            manager_proof_aggregation_latency: Duration::from_secs_f64(
                resp.manager_proof_aggregation_ms / 1000.0,
            ),
            manager_set_operation_proof_generation_latency: Duration::from_secs_f64(
                resp.manager_set_operation_proof_generation_ms / 1000.0,
            ),
            route_mode: resp.route_mode,
            persistence_mode: resp.persistence_mode,
        })
    }

    /// Delete file: remove (fid, keywords) from the system
    pub async fn delete_file(
        &self,
        fid: String,
        keywords: Vec<String>,
        metadata: &RunMetadata,
    ) -> Result<(Duration, String), Box<dyn std::error::Error>> {
        let channel = self.get_channel().await?;
        let mut client = ManagerServiceClient::new(channel);

        let request = DeleteRequest {
            fid,
            keywords,
            dataset: metadata.dataset.clone(),
            concurrency: metadata.concurrency,
            total_uploads: metadata.total_uploads,
            total_queries: metadata.total_queries,
            total_updates: metadata.total_updates,
        };

        let response = client.delete(request).await?;
        let resp = response.into_inner();
        let mut verification_latency = Duration::from_secs(0);

        if resp.success {
            // Client-side verification
            let verification_start = std::time::Instant::now();
            if self
                .verifier
                .verify(&resp.combined_proof, &resp.combined_root_hash)
            {
                verification_latency = verification_start.elapsed();
                println!("✅ Client verification passed (Delete)");
                println!("Delete file succeeded: {}", resp.message);
            } else {
                println!("❌ Client verification failed (Delete)");
                return Err("Client verification failed".into());
            }
        } else {
            println!("Delete file failed: {}", resp.message);
        }

        Ok((verification_latency, resp.route_mode))
    }

    /// Update file: change (fid, old_keywords) to (fid, new_keywords)
    pub async fn update_file(
        &self,
        fid: String,
        old_keywords: Vec<String>,
        new_keywords: Vec<String>,
        metadata: &RunMetadata,
    ) -> Result<(Duration, String, String), Box<dyn std::error::Error>> {
        self.update_file_raw(fid, old_keywords, new_keywords, metadata)
            .await
    }

    async fn update_file_raw(
        &self,
        fid: String,
        old_keywords: Vec<String>,
        new_keywords: Vec<String>,
        metadata: &RunMetadata,
    ) -> Result<(Duration, String, String), Box<dyn std::error::Error>> {
        let channel = self.get_channel().await?;
        let mut client = ManagerServiceClient::new(channel);

        let request = UpdateRequest {
            fid,
            old_keywords,
            new_keywords,
            dataset: metadata.dataset.clone(),
            concurrency: metadata.concurrency,
            total_uploads: metadata.total_uploads,
            total_queries: metadata.total_queries,
            total_updates: metadata.total_updates,
        };

        let response = client.update(request).await?;
        let resp = response.into_inner();
        let mut verification_latency = Duration::from_secs(0);

        if resp.success {
            // Client-side verification
            let verification_start = std::time::Instant::now();
            if self
                .verifier
                .verify(&resp.combined_proof, &resp.combined_root_hash)
            {
                verification_latency = verification_start.elapsed();
                println!("✅ Client verification passed (Update)");
                println!("Update file succeeded: {}", resp.message);
            } else {
                println!("❌ Client verification failed (Update)");
                return Err("Client verification failed".into());
            }
        } else {
            println!("Update file failed: {}", resp.message);
        }

        Ok((verification_latency, resp.route_mode, resp.persistence_mode))
    }

    pub async fn update_file_hex(
        &self,
        fid: String,
        old_keywords_hex: Vec<String>,
        new_keywords_hex: Vec<String>,
        metadata: &RunMetadata,
    ) -> Result<(Duration, String, String), Box<dyn std::error::Error>> {
        self.update_file_raw(fid, old_keywords_hex, new_keywords_hex, metadata)
            .await
    }

    pub async fn reset_system(&self) -> Result<(), Box<dyn std::error::Error>> {
        let channel = self.get_channel().await?;
        let mut client = ManagerServiceClient::new(channel);
        let response = client.reset_system(ResetSystemRequest {}).await?;
        let resp = response.into_inner();
        if resp.success {
            println!("{}", resp.message);
            Ok(())
        } else {
            Err(resp.message.into())
        }
    }
}
