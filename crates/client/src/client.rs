use common::rpc::{
    manager_service_client::ManagerServiceClient, AddRequest, BatchAddRecord, BatchAddRequest,
    DeleteRequest, QueryRequest, ResetSystemRequest, UpdateRequest,
};
use common::{
    is_accumulator_set_operation_proof, is_polynomial_intersection_proof, AdsMode, ProofVerifier,
    SetProofMode,
};
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};

fn env_duration_secs(key: &str, default_secs: u64) -> Duration {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default_secs))
}

#[derive(Debug, Clone, Copy)]
pub struct QueryKeywordMetrics {
    pub result_count: usize,
    pub proof_size_bytes: usize,
    pub verification_latency: Duration,
    pub manager_proof_aggregation_latency: Duration,
    pub manager_set_operation_proof_generation_latency: Duration,
}

/// Client 结构，封装与 Manager 的交互
pub struct Client {
    manager_addr: String,
    client: Option<ManagerServiceClient<Channel>>,
    verifier: ProofVerifier,
    set_proof_mode: SetProofMode,
}

impl Client {
    /// 创建新的 Client
    pub fn new(manager_addr: String, ads_mode: AdsMode, set_proof_mode: SetProofMode) -> Self {
        Client {
            manager_addr,
            client: None,
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

    /// 获取或创建gRPC client连接
    async fn get_client(
        &mut self,
    ) -> Result<&mut ManagerServiceClient<Channel>, Box<dyn std::error::Error>> {
        if self.client.is_none() {
            let use_heavy_profile = matches!(self.verifier.ads_mode(), AdsMode::Mpt | AdsMode::AccTree);
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

            let channel = endpoint.connect().await?;
            self.client = Some(ManagerServiceClient::new(channel));
        }
        Ok(self.client.as_mut().unwrap())
    }

    /// Put file: add (fid, keywords) to the system
    pub async fn put_file(
        &mut self,
        fid: String,
        keywords: Vec<String>,
        total_upload_kv_pairs: u32,
    ) -> Result<Duration, Box<dyn std::error::Error>> {
        let client = self.get_client().await?;

        let request = AddRequest {
            fid,
            keywords,
            total_upload_kv_pairs,
        };

        let response = client.add(request).await?;
        let resp = response.into_inner();
        let mut verification_latency = Duration::from_secs(0);

        if resp.success {
            // Client-side verification
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

        Ok(verification_latency)
    }

    pub async fn batch_put_files(
        &mut self,
        records: Vec<(String, Vec<String>)>,
        total_upload_kv_pairs: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let client = self.get_client().await?;
        let request = BatchAddRequest {
            total_upload_kv_pairs,
            records: records
                .into_iter()
                .map(|(fid, keywords)| BatchAddRecord { fid, keywords })
                .collect(),
        };
        let response = client.batch_add(request).await?;
        let resp = response.into_inner();
        if resp.success {
            Ok(())
        } else {
            Err(resp.message.into())
        }
    }

    /// Query by keyword
    pub async fn query_by_keyword(
        &mut self,
        keyword: String,
    ) -> Result<QueryKeywordMetrics, Box<dyn std::error::Error>> {
        let client = self.get_client().await?;

        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::Keyword(keyword)),
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
        })
    }

    /// Query by boolean function
    pub async fn query_by_func(
        &mut self,
        boolean_func: String,
    ) -> Result<QueryKeywordMetrics, Box<dyn std::error::Error>> {
        let client = self.get_client().await?;

        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::BooleanFunction(
                boolean_func,
            )),
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
        })
    }

    /// Delete file: remove (fid, keywords) from the system
    pub async fn delete_file(
        &mut self,
        fid: String,
        keywords: Vec<String>,
    ) -> Result<Duration, Box<dyn std::error::Error>> {
        let client = self.get_client().await?;

        let request = DeleteRequest { fid, keywords };

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

        Ok(verification_latency)
    }

    /// Update file: change (fid, old_keywords) to (fid, new_keywords)
    pub async fn update_file(
        &mut self,
        fid: String,
        old_keywords: Vec<String>,
        new_keywords: Vec<String>,
    ) -> Result<Duration, Box<dyn std::error::Error>> {
        let client = self.get_client().await?;

        let request = UpdateRequest {
            fid,
            old_keywords,
            new_keywords,
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

        Ok(verification_latency)
    }

    pub async fn reset_system(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let client = self.get_client().await?;
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
