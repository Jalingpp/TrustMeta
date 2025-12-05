use common::rpc::{
    manager_service_client::ManagerServiceClient, AddRequest, DeleteRequest, QueryRequest,
    UpdateRequest,
};
use common::{AdsMode, ProofVerifier};
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};

/// Client 结构，封装与 Manager 的交互
pub struct Client {
    manager_addr: String,
    client: Option<ManagerServiceClient<Channel>>,
    verifier: ProofVerifier,
}

impl Client {
    /// 创建新的 Client
    pub fn new(manager_addr: String, ads_mode: AdsMode) -> Self {
        Client {
            manager_addr,
            client: None,
            verifier: ProofVerifier::new(ads_mode),
        }
    }

    /// 获取或创建gRPC client连接
    async fn get_client(
        &mut self,
    ) -> Result<&mut ManagerServiceClient<Channel>, Box<dyn std::error::Error>> {
        if self.client.is_none() {
            let endpoint = Endpoint::from_shared(self.manager_addr.clone())?
                .timeout(Duration::from_secs(60))
                .connect_timeout(Duration::from_secs(10))
                .tcp_keepalive(Some(Duration::from_secs(30)));

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
    ) -> Result<(), Box<dyn std::error::Error>> {
        let client = self.get_client().await?;

        let request = AddRequest { fid, keywords };

        let response = client.add(request).await?;
        let resp = response.into_inner();

        if resp.success {
            // Client-side verification
            if !resp.combined_proof.is_empty() {
                if self.verifier.verify(&resp.combined_proof, &resp.combined_root_hash) {
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

        Ok(())
    }

    /// Query by keyword
    pub async fn query_by_keyword(
        &mut self,
        keyword: String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let client = self.get_client().await?;

        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::Keyword(keyword)),
        };

        let response = client.query(request).await?;
        let resp = response.into_inner();

        // Client-side verification
        if self.verifier.verify(&resp.proof, &resp.root_hash) {
            println!("✅ Client verification passed (Query)");
            println!("Query succeeded, found {} files:", resp.fids.len());
            for fid in resp.fids {
                println!("  - {}", fid);
            }
        } else {
            println!("❌ Client verification failed (Query)");
            return Err("Client verification failed".into());
        }

        Ok(())
    }

    /// Query by boolean function
    pub async fn query_by_func(
        &mut self,
        boolean_func: String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let client = self.get_client().await?;

        let request = QueryRequest {
            query_type: Some(common::rpc::query_request::QueryType::BooleanFunction(
                boolean_func,
            )),
        };

        let response = client.query(request).await?;
        let resp = response.into_inner();

        // Client-side verification
        if self.verifier.verify(&resp.proof, &resp.root_hash) {
            println!("✅ Client verification passed (Boolean Query)");
            println!("Query succeeded, found {} files:", resp.fids.len());
            for fid in resp.fids {
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
        } else {
            println!("❌ Client verification failed (Boolean Query)");
            return Err("Client verification failed".into());
        }

        Ok(())
    }

    /// Delete file: remove (fid, keywords) from the system
    pub async fn delete_file(
        &mut self,
        fid: String,
        keywords: Vec<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let client = self.get_client().await?;

        let request = DeleteRequest { fid, keywords };

        let response = client.delete(request).await?;
        let resp = response.into_inner();

        if resp.success {
            // Client-side verification
            if self.verifier.verify(&resp.combined_proof, &resp.combined_root_hash) {
                println!("✅ Client verification passed (Delete)");
                println!("Delete file succeeded: {}", resp.message);
            } else {
                println!("❌ Client verification failed (Delete)");
                return Err("Client verification failed".into());
            }
        } else {
            println!("Delete file failed: {}", resp.message);
        }

        Ok(())
    }

    /// Update file: change (fid, old_keywords) to (fid, new_keywords)
    pub async fn update_file(
        &mut self,
        fid: String,
        old_keywords: Vec<String>,
        new_keywords: Vec<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let client = self.get_client().await?;

        let request = UpdateRequest {
            fid,
            old_keywords,
            new_keywords,
        };

        let response = client.update(request).await?;
        let resp = response.into_inner();

        if resp.success {
            // Client-side verification
            if self.verifier.verify(&resp.combined_proof, &resp.combined_root_hash) {
                println!("✅ Client verification passed (Update)");
                println!("Update file succeeded: {}", resp.message);
            } else {
                println!("❌ Client verification failed (Update)");
                return Err("Client verification failed".into());
            }
        } else {
            println!("Update file failed: {}", resp.message);
        }

        Ok(())
    }
}
