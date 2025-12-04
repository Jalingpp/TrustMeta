//! Manager 核心结构
//!
//! 负责协调客户端请求和 storager 节点

use crate::core::{ProofVerifier, Router};
use common::rpc::storager_service_client::StoragerServiceClient;
use common::{AdsMode, RootHash};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::OnceCell;
use tonic::transport::Channel;

/// Manager 结构
///
/// 负责：
/// - 路由请求到对应的 storager 节点
/// - 验证来自 storager 的证明
/// - 维护系统状态（根哈希等）
#[derive(Clone)]
pub struct Manager {
    /// 路由器(管理一致性哈希和地址映射)
    pub(crate) router: Arc<Router>,
    /// 证明验证器
    pub(crate) verifier: Arc<ProofVerifier>,
    /// storager 名称到根哈希的映射
    pub(crate) root_hashes: Arc<RwLock<HashMap<String, RootHash>>>,
    /// storager 连接池(地址 -> 客户端)
    pub(crate) client_pool:
        Arc<RwLock<HashMap<String, Arc<OnceCell<StoragerServiceClient<Channel>>>>>>,
}

impl Manager {
    /// 创建新的 Manager 实例
    ///
    /// # Arguments
    /// * `storager_addrs` - storager 地址列表
    /// * `ads_mode` - ADS 模式
    pub fn new(storager_addrs: Vec<String>, ads_mode: AdsMode) -> Self {
        let router = Arc::new(Router::new(storager_addrs, 150)); // 每个节点 150 个虚拟节点
        let verifier = Arc::new(ProofVerifier::new(ads_mode));
        let root_hashes = Arc::new(RwLock::new(HashMap::new()));
        let client_pool = Arc::new(RwLock::new(HashMap::new()));

        Manager {
            router,
            verifier,
            root_hashes,
            client_pool,
        }
    }

    /// 使用一致性哈希环获取 keyword 对应的 storager
    pub(crate) fn get_storager_for_keyword(&self, keyword: &str) -> Option<(String, String)> {
        self.router.get_storager_for_keyword(keyword)
    }

    /// 验证证明
    pub(crate) fn verify_proof(&self, proof: &[u8], root_hash: &[u8]) -> bool {
        self.verifier.verify(proof, root_hash)
    }

    /// 更新 storager 的根哈希
    pub(crate) fn update_root_hash(&self, storager_name: String, root_hash: RootHash) {
        let mut hashes = self
            .root_hashes
            .write()
            .expect("Failed to acquire write lock on root_hashes");
        hashes.insert(storager_name, root_hash);
    }

    /// 获取 storager 的当前根哈希
    pub(crate) fn get_root_hash(&self, storager_name: &str) -> Option<RootHash> {
        let hashes = self
            .root_hashes
            .read()
            .expect("Failed to acquire read lock on root_hashes");
        hashes.get(storager_name).cloned()
    }

    /// 合并多个证明
    pub(crate) fn combine_proofs(&self, proofs: &[Vec<u8>]) -> Vec<u8> {
        self.verifier.combine_proofs(proofs)
    }

    /// 获取当前的 ADS 模式
    pub fn ads_mode(&self) -> AdsMode {
        self.verifier.ads_mode()
    }

    /// 获取所有 storager 节点信息
    pub fn get_storagers(&self) -> Vec<(String, String)> {
        self.router.get_all_storagers()
    }

    /// 获取或创建到指定storager的连接
    pub(crate) async fn get_storager_client(
        &self,
        storager_addr: &str,
    ) -> Result<StoragerServiceClient<Channel>, tonic::transport::Error> {
        // 确保地址有 http:// 前缀
        let addr_with_scheme =
            if storager_addr.starts_with("http://") || storager_addr.starts_with("https://") {
                storager_addr.to_string()
            } else {
                format!("http://{}", storager_addr)
            };

        // 获取或创建连接
        let cell = {
            let mut pool = self
                .client_pool
                .write()
                .expect("Failed to acquire write lock on client_pool");
            pool.entry(addr_with_scheme.clone())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };

        // 懒加载:只在第一次使用时创建连接
        let client = cell
            .get_or_try_init(|| async {
                // 配置优化的连接参数
                // 使用 expect 是因为地址已经在前面格式化过，这里不应该失败
                let endpoint = tonic::transport::Endpoint::from_shared(addr_with_scheme.clone())
                    .expect("Failed to create endpoint from validated address")
                    .timeout(std::time::Duration::from_secs(30)) // 请求超时30秒
                    .connect_timeout(std::time::Duration::from_secs(10)) // 连接超时10秒
                    .tcp_keepalive(Some(std::time::Duration::from_secs(60))) // TCP keepalive
                    .http2_keep_alive_interval(std::time::Duration::from_secs(30)) // HTTP2 keepalive
                    .keep_alive_timeout(std::time::Duration::from_secs(20)) // keepalive超时
                    .concurrency_limit(256); // 每个连接的并发限制

                StoragerServiceClient::connect(endpoint).await
            })
            .await?;

        Ok(client.clone())
    }
}
