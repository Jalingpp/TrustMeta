//! System-level helper library for the distributed-storage-system workspace.
//!
//! 提供整个分布式存储系统的初始化与配置读写工具：
//! - `initialize` 用于根据参数构造 `SystemConfig`
//! - `load_config` / `save_config` 用于从文件加载和保存配置

use common::{AdsMode, SetProofMode, SystemConfig};
use std::error::Error;

/// Initialize the distributed storage system
///
/// 当前实现仅构造并返回 `SystemConfig`，不负责真正拉起各个进程。
/// 可以在测试或实验脚本中复用。
pub async fn initialize(
    num_clients: usize,
    num_storagers: usize,
    ads_mode: AdsMode,
    manager_addr: String,
    storager_addrs: Vec<String>,
    client_addrs: Vec<String>,
) -> Result<SystemConfig, Box<dyn Error>> {
    println!("Initializing distributed storage system...");
    println!("  Clients: {}", num_clients);
    println!("  Storagers: {}", num_storagers);
    println!("  ADS Mode: {:?}", ads_mode);
    println!("  Manager Address: {}", manager_addr);

    // Validate configuration
    if storager_addrs.len() != num_storagers {
        return Err("Number of storager addresses must match num_storagers".into());
    }

    if !client_addrs.is_empty() && client_addrs.len() != num_clients {
        return Err("Number of client addresses must match num_clients or be empty".into());
    }

    let config = SystemConfig {
        num_clients,
        num_storagers,
        ads_mode,
        set_proof_mode: SetProofMode::Polynomial,
        manager_addr,
        storager_addrs,
        client_addrs,
    };

    println!("System initialized successfully!");

    Ok(config)
}

/// Load system configuration from a file
pub fn load_config(path: &str) -> Result<SystemConfig, Box<dyn Error>> {
    let content = std::fs::read_to_string(path)?;
    let config: SystemConfig = serde_json::from_str(&content)?;
    Ok(config)
}

/// Save system configuration to a file
pub fn save_config(config: &SystemConfig, path: &str) -> Result<(), Box<dyn Error>> {
    let content = serde_json::to_string_pretty(config)?;
    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::rpc::manager_service_client::ManagerServiceClient;
    use common::rpc::manager_service_server::ManagerServiceServer;
    use common::rpc::query_request::QueryType;
    use common::rpc::storager_service_server::StoragerServiceServer;
    use common::rpc::{AddRequest, BatchAddRecord, BatchAddRequest, QueryRequest};
    use common::{AdsMode, ProofVerifier, SetProofMode};
    use manager::Manager;
    use storager::Storager;
    use tokio::net::TcpListener;
    use tokio::time::{sleep, Duration};
    use tonic::transport::{Channel, Server};

    async fn spawn_storager(addr: std::net::SocketAddr, storager: Storager) {
        tokio::spawn(async move {
            Server::builder()
                .add_service(StoragerServiceServer::new(storager))
                .serve(addr)
                .await
                .unwrap();
        });
    }

    async fn spawn_manager(addr: std::net::SocketAddr, manager: Manager) {
        tokio::spawn(async move {
            Server::builder()
                .add_service(ManagerServiceServer::new(manager))
                .serve(addr)
                .await
                .unwrap();
        });
    }

    async fn connect_manager(addr: std::net::SocketAddr) -> ManagerServiceClient<Channel> {
        let endpoint = [
            bytes_to_string(&[104, 116, 116, 112, 58, 47, 47]),
            addr.to_string(),
        ]
        .concat();
        for _ in 0..20 {
            if let Ok(client) = ManagerServiceClient::connect(endpoint.clone()).await {
                return client;
            }
            sleep(Duration::from_millis(100)).await;
        }
        panic!()
    }

    fn bytes_to_string(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn build_storager(mode: AdsMode) -> Storager {
        match mode {
            AdsMode::Mpt => Storager::with_mpt(),
            AdsMode::Mest => Storager::with_mest(),
            AdsMode::AccTrie => Storager::with_acctrie(),
            AdsMode::AccTree => Storager::with_acctree(),
        }
    }

    async fn run_manager_query_end_to_end(mode: AdsMode) {
        let bind_addr = bytes_to_string(&[49, 50, 55, 46, 48, 46, 48, 46, 49, 58, 48]);
        let http_prefix = bytes_to_string(&[104, 116, 116, 112, 58, 47, 47]);
        let file_1 = bytes_to_string(&[102, 105, 108, 101, 45, 49]);
        let alpha = bytes_to_string(&[97, 108, 112, 104, 97]);
        let beta = bytes_to_string(&[98, 101, 116, 97]);
        let alpha_and_beta =
            bytes_to_string(&[97, 108, 112, 104, 97, 32, 65, 78, 68, 32, 98, 101, 116, 97]);

        let storager_listener = TcpListener::bind(bind_addr.clone()).await.unwrap();
        let storager_addr = storager_listener.local_addr().unwrap();
        drop(storager_listener);
        spawn_storager(storager_addr, build_storager(mode)).await;

        let manager_listener = TcpListener::bind(bind_addr).await.unwrap();
        let manager_addr = manager_listener.local_addr().unwrap();
        drop(manager_listener);
        spawn_manager(
            manager_addr,
            Manager::new(
                vec![[http_prefix.clone(), storager_addr.to_string()].concat()],
                mode,
                SetProofMode::Polynomial,
                150,
            ),
        )
        .await;

        let mut client = connect_manager(manager_addr).await;

        let add_response = client
            .add(AddRequest {
                fid: file_1.clone(),
                keywords: vec![alpha.clone(), beta.clone()],
                total_upload_kv_pairs: 2,
                dataset: "test".to_string(),
                concurrency: 1,
                total_uploads: 1,
                total_queries: 1,
                total_updates: 1,
            })
            .await
            .unwrap()
            .into_inner();
        assert!(add_response.success);

        let query_alpha = client
            .query(QueryRequest {
                query_type: Some(QueryType::Keyword(alpha.clone())),
                dataset: "test".to_string(),
                concurrency: 1,
                total_uploads: 1,
                total_queries: 1,
                total_updates: 1,
            })
            .await
            .unwrap()
            .into_inner();

        assert!(query_alpha.verified);
        assert_eq!(query_alpha.fids, vec![file_1.clone()]);

        let verifier = ProofVerifier::new(mode);
        assert!(verifier.verify(&query_alpha.proof, &query_alpha.root_hash));
        assert!(verifier.verify_query_result_fids(&query_alpha.proof, &query_alpha.fids));

        let query_boolean = client
            .query(QueryRequest {
                query_type: Some(QueryType::BooleanFunction(alpha_and_beta)),
                dataset: "test".to_string(),
                concurrency: 1,
                total_uploads: 1,
                total_queries: 1,
                total_updates: 1,
            })
            .await
            .unwrap()
            .into_inner();

        assert!(query_boolean.verified);
        assert_eq!(query_boolean.fids, vec![file_1]);
        assert!(verifier.verify(&query_boolean.proof, &query_boolean.root_hash));
        assert!(verifier.verify_query_result_fids(&query_boolean.proof, &query_boolean.fids));
    }

    async fn run_multi_node_boolean_query_end_to_end(mode: AdsMode) {
        let bind_addr = bytes_to_string(&[49, 50, 55, 46, 48, 46, 48, 46, 49, 58, 48]);
        let http_prefix = bytes_to_string(&[104, 116, 116, 112, 58, 47, 47]);
        let shared_fid = bytes_to_string(&[115, 104, 97, 114, 101, 100, 45, 102, 105, 108, 101]);
        let and_sep = bytes_to_string(&[32, 65, 78, 68, 32]);

        let storager_listener_1 = TcpListener::bind(bind_addr.clone()).await.unwrap();
        let storager_addr_1 = storager_listener_1.local_addr().unwrap();
        drop(storager_listener_1);
        spawn_storager(storager_addr_1, build_storager(mode)).await;

        let storager_listener_2 = TcpListener::bind(bind_addr.clone()).await.unwrap();
        let storager_addr_2 = storager_listener_2.local_addr().unwrap();
        drop(storager_listener_2);
        spawn_storager(storager_addr_2, build_storager(mode)).await;

        let manager_listener = TcpListener::bind(bind_addr).await.unwrap();
        let manager_addr = manager_listener.local_addr().unwrap();
        drop(manager_listener);
        spawn_manager(
            manager_addr,
            Manager::new(
                vec![
                    [http_prefix.clone(), storager_addr_1.to_string()].concat(),
                    [http_prefix.clone(), storager_addr_2.to_string()].concat(),
                ],
                mode,
                SetProofMode::Polynomial,
                150,
            ),
        )
        .await;

        let mut client = connect_manager(manager_addr).await;
        let candidate_keywords: Vec<String> = (0..32)
            .map(|index| [bytes_to_string(&[107, 119, 45]), index.to_string()].concat())
            .collect();
        let mut node_keywords: Vec<(String, String)> = Vec::new();

        for keyword in &candidate_keywords {
            let probe_fid = [
                bytes_to_string(&[112, 114, 111, 98, 101, 45]),
                keyword.clone(),
            ]
            .concat();
            let add_response = client
                .add(AddRequest {
                    fid: probe_fid,
                    keywords: vec![keyword.clone()],
                    total_upload_kv_pairs: 1,
                    dataset: "test".to_string(),
                    concurrency: 1,
                    total_uploads: 1,
                    total_queries: 1,
                    total_updates: 1,
                })
                .await
                .unwrap()
                .into_inner();
            assert!(add_response.success);

            let query = client
                .query(QueryRequest {
                    query_type: Some(QueryType::Keyword(keyword.clone())),
                    dataset: "test".to_string(),
                    concurrency: 1,
                    total_uploads: 1,
                    total_queries: 1,
                    total_updates: 1,
                })
                .await
                .unwrap()
                .into_inner();
            assert!(query.verified);

            let node_name = query.node_root_hashes.keys().next().cloned().unwrap();
            if node_keywords
                .iter()
                .all(|(existing, _)| existing != &node_name)
            {
                node_keywords.push((node_name, keyword.clone()));
            }
            if node_keywords.len() == 2 {
                break;
            }
        }

        assert_eq!(node_keywords.len(), 2);
        let left_keyword = node_keywords[0].1.clone();
        let right_keyword = node_keywords[1].1.clone();

        let add_shared = client
            .add(AddRequest {
                fid: shared_fid.clone(),
                keywords: vec![left_keyword.clone(), right_keyword.clone()],
                total_upload_kv_pairs: 2,
                dataset: "test".to_string(),
                concurrency: 1,
                total_uploads: 1,
                total_queries: 1,
                total_updates: 1,
            })
            .await
            .unwrap()
            .into_inner();
        assert!(add_shared.success);

        let boolean_query = [left_keyword, and_sep, right_keyword].concat();
        let response = client
            .query(QueryRequest {
                query_type: Some(QueryType::BooleanFunction(boolean_query)),
                dataset: "test".to_string(),
                concurrency: 1,
                total_uploads: 1,
                total_queries: 1,
                total_updates: 1,
            })
            .await
            .unwrap()
            .into_inner();

        assert!(response.verified);
        assert_eq!(response.fids, vec![shared_fid]);
        assert!(response.node_root_hashes.len() >= 2);

        let verifier = ProofVerifier::new(mode);
        assert!(verifier.verify(&response.proof, &response.root_hash));
        assert!(verifier.verify_query_result_fids(&response.proof, &response.fids));
    }

    async fn run_manager_batch_add_end_to_end(mode: AdsMode) {
        let bind_addr = bytes_to_string(&[49, 50, 55, 46, 48, 46, 48, 46, 49, 58, 48]);
        let http_prefix = bytes_to_string(&[104, 116, 116, 112, 58, 47, 47]);
        let batch_file_1 = bytes_to_string(&[98, 97, 116, 99, 104, 45, 102, 105, 108, 101, 45, 49]);
        let batch_file_2 = bytes_to_string(&[98, 97, 116, 99, 104, 45, 102, 105, 108, 101, 45, 50]);
        let alpha = bytes_to_string(&[97, 108, 112, 104, 97]);
        let beta = bytes_to_string(&[98, 101, 116, 97]);
        let gamma = bytes_to_string(&[103, 97, 109, 109, 97]);

        let storager_listener = TcpListener::bind(bind_addr.clone()).await.unwrap();
        let storager_addr = storager_listener.local_addr().unwrap();
        drop(storager_listener);
        spawn_storager(storager_addr, build_storager(mode)).await;

        let manager_listener = TcpListener::bind(bind_addr).await.unwrap();
        let manager_addr = manager_listener.local_addr().unwrap();
        drop(manager_listener);
        spawn_manager(
            manager_addr,
            Manager::new(
                vec![[http_prefix.clone(), storager_addr.to_string()].concat()],
                mode,
                SetProofMode::Polynomial,
                150,
            ),
        )
        .await;

        let mut client = connect_manager(manager_addr).await;
        let resp = client
            .batch_add(BatchAddRequest {
                records: vec![
                    BatchAddRecord {
                        fid: batch_file_1.clone(),
                        keywords: vec![alpha.to_string(), beta.to_string()],
                    },
                    BatchAddRecord {
                        fid: batch_file_2.clone(),
                        keywords: vec![beta.to_string(), gamma.to_string()],
                    },
                ],
                total_upload_kv_pairs: 4,
                dataset: "test".to_string(),
                concurrency: 1,
                total_uploads: 1,
                total_queries: 1,
                total_updates: 1,
            })
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.record_count, 2);
        assert_eq!(resp.keyword_pair_count, 4);

        let query_alpha = client
            .query(QueryRequest {
                query_type: Some(QueryType::Keyword(alpha.to_string())),
                dataset: "test".to_string(),
                concurrency: 1,
                total_uploads: 1,
                total_queries: 1,
                total_updates: 1,
            })
            .await
            .unwrap()
            .into_inner();
        assert!(query_alpha.verified);
        assert_eq!(query_alpha.fids, vec![batch_file_1]);

        let query_beta = client
            .query(QueryRequest {
                query_type: Some(QueryType::Keyword(beta.to_string())),
                dataset: "test".to_string(),
                concurrency: 1,
                total_uploads: 1,
                total_queries: 1,
                total_updates: 1,
            })
            .await
            .unwrap()
            .into_inner();
        assert!(query_beta.verified);
        assert_eq!(query_beta.fids.len(), 2);
    }

    async fn run_acctree_prefix_migration_end_to_end() {
        let bind_addr = bytes_to_string(&[49, 50, 55, 46, 48, 46, 48, 46, 49, 58, 48]);
        let http_prefix = bytes_to_string(&[104, 116, 116, 112, 58, 47, 47]);
        std::env::set_var("EPRING_SPLIT_THRESHOLD", "1");

        let storager_listener_1 = TcpListener::bind(bind_addr.clone()).await.unwrap();
        let storager_addr_1 = storager_listener_1.local_addr().unwrap();
        drop(storager_listener_1);
        spawn_storager(storager_addr_1, build_storager(AdsMode::AccTree)).await;

        let storager_listener_2 = TcpListener::bind(bind_addr.clone()).await.unwrap();
        let storager_addr_2 = storager_listener_2.local_addr().unwrap();
        drop(storager_listener_2);
        spawn_storager(storager_addr_2, build_storager(AdsMode::AccTree)).await;

        let manager_listener = TcpListener::bind(bind_addr).await.unwrap();
        let manager_addr = manager_listener.local_addr().unwrap();
        drop(manager_listener);
        spawn_manager(
            manager_addr,
            Manager::new(
                vec![
                    [http_prefix.clone(), storager_addr_1.to_string()].concat(),
                    [http_prefix.clone(), storager_addr_2.to_string()].concat(),
                ],
                AdsMode::AccTree,
                SetProofMode::Polynomial,
                1,
            ),
        )
        .await;

        let mut client = connect_manager(manager_addr).await;
        let shared_fid = bytes_to_string(&[
            97, 99, 99, 116, 114, 101, 101, 45, 109, 105, 103, 114, 97, 116, 101, 100,
        ]);
        let keywords = vec![
            bytes_to_string(&[97, 99, 99, 45, 107, 48]),
            bytes_to_string(&[97, 99, 99, 45, 107, 49]),
            bytes_to_string(&[97, 99, 99, 45, 107, 50]),
            bytes_to_string(&[97, 99, 99, 45, 107, 51]),
        ];

        for keyword in &keywords {
            let add = client
                .add(AddRequest {
                    fid: shared_fid.clone(),
                    keywords: vec![keyword.clone()],
                    total_upload_kv_pairs: 1,
                    dataset: "test".to_string(),
                    concurrency: 1,
                    total_uploads: 1,
                    total_queries: 1,
                    total_updates: 1,
                })
                .await
                .unwrap()
                .into_inner();
            assert!(add.success);
        }

        sleep(Duration::from_millis(300)).await;

        for keyword in &keywords {
            let response = client
                .query(QueryRequest {
                    query_type: Some(QueryType::Keyword(keyword.clone())),
                    dataset: "test".to_string(),
                    concurrency: 1,
                    total_uploads: 1,
                    total_queries: 1,
                    total_updates: 1,
                })
                .await
                .unwrap()
                .into_inner();

            assert!(response.verified);
            assert_eq!(response.fids, vec![shared_fid.clone()]);
        }

        std::env::remove_var("EPRING_SPLIT_THRESHOLD");
    }

    #[tokio::test]
    async fn test_acctree_manager_query_end_to_end() {
        run_manager_query_end_to_end(AdsMode::AccTree).await;
    }

    #[tokio::test]
    async fn test_acctree_prefix_migration_end_to_end() {
        run_acctree_prefix_migration_end_to_end().await;
    }

    #[tokio::test]
    async fn test_all_ads_manager_query_end_to_end() {
        for mode in [
            AdsMode::Mpt,
            AdsMode::Mest,
            AdsMode::AccTrie,
            AdsMode::AccTree,
        ] {
            run_manager_query_end_to_end(mode).await;
        }
    }

    #[tokio::test]
    async fn test_all_ads_multi_node_boolean_query_end_to_end() {
        for mode in [
            AdsMode::Mpt,
            AdsMode::Mest,
            AdsMode::AccTrie,
            AdsMode::AccTree,
        ] {
            run_multi_node_boolean_query_end_to_end(mode).await;
        }
    }

    #[tokio::test]
    async fn test_all_ads_manager_batch_add_end_to_end() {
        for mode in [
            AdsMode::Mpt,
            AdsMode::Mest,
            AdsMode::AccTrie,
            AdsMode::AccTree,
        ] {
            run_manager_batch_add_end_to_end(mode).await;
        }
    }

    #[tokio::test]
    async fn test_initialize() {
        let config = initialize(
            2,
            3,
            AdsMode::Mest,
            "http://[::1]:50051".to_string(),
            vec![
                "http://[::1]:50052".to_string(),
                "http://[::1]:50053".to_string(),
                "http://[::1]:50054".to_string(),
            ],
            vec![],
        )
        .await;

        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.num_clients, 2);
        assert_eq!(config.num_storagers, 3);
    }
}
