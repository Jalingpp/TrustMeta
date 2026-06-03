//! 璁よ瘉鏁版嵁缁撴瀯 (Authenticated Data Structures) 妯″潡
//!
//! 璇ユā鍧楀畾涔変簡鎵€鏈?ADS 瀹炵幇蹇呴』閬靛畧鐨勯€氱敤鎺ュ彛锛?
//! 骞舵彁渚涗簡澶氱 ADS 鐨勫叿浣撳疄鐜般€?
//!
//! ## 鍙敤鐨?ADS 瀹炵幇
//! - **MptAds**: Merkle Patricia Trie (浠ュお鍧婇鏍?
//! - **MestAds**: Merkle-based Extendible Segmented Hash Tree
//! - **AccTrieAds**: Accumulator-based Trie with Cryptographic Accumulators
//!
//! ## 濡備綍閫夋嫨 ADS
//! 璇峰弬鑰?`ads/README.md` 鑾峰彇璇︾粏鐨勯€夋嫨鎸囧崡鍜屾€ц兘瀵规瘮銆?
//!
//! ## 娣诲姞鏂扮殑 ADS
//! 1. 瀹炵幇 `AdsOperations` trait
//! 2. 鍦?`mod.rs` 涓鍑?
//! 3. 鍦?`common::AdsMode` 涓坊鍔犳灇涓?
//! 4. 鍦?`Storager::from_config()` 涓坊鍔犲尮閰嶅垎鏀?
//! 5. 鍦?`ProofVerifier::verify()` 涓坊鍔犻獙璇侀€昏緫

use ads_rust::unified_ads::{AuthenticatedDataStructure, UnifiedKey, UnifiedValue};
use common::RootHash;

/// ADS 鎿嶄綔鐨勯€氱敤 trait
///
/// 鎵€鏈夎璇佹暟鎹粨鏋勯兘闇€瑕佸疄鐜拌繖涓?trait
pub trait AdsOperations: Send + Sync {
    /// 娣诲姞 (keyword, fid) 瀵瑰埌 ADS
    /// 杩斿洖: (proof, root_hash)
    fn add(&self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash);

    /// 鏌ヨ keyword 瀵瑰簲鐨勬墍鏈?fid
    /// 杩斿洖: (fids, proof)
    fn query(&self, keyword: &str) -> (Vec<String>, Vec<u8>);

    /// 浠?ADS 涓垹闄?(keyword, fid) 瀵?
    /// 杩斿洖: (proof, root_hash)
    fn delete(&self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash);

    /// 鎵归噺娣诲姞 (keyword, fid) 瀵瑰埌 ADS
    /// 杩斿洖: (proof, root_hash) - proof 鍙兘涓虹┖锛屽彇鍐充簬瀹炵幇
    fn add_batch(&self, kvs: Vec<(String, String)>) -> (Vec<u8>, RootHash) {
        // 榛樿瀹炵幇锛氬惊鐜皟鐢?add
        let mut last_root_hash = Vec::new();
        for (k, v) in kvs {
            let (_, root) = self.add(&k, &v);
            last_root_hash = root;
        }
        (Vec::new(), last_root_hash)
    }

    /// 褰撳墠 ADS 鐨勯檮鍔犳牴鐘舵€併€?
    /// 瀵?MPT/MEST 榛樿杩斿洖绌猴紱AccTrie 杩斿洖鏍圭疮鍔犲櫒鍊肩殑搴忓垪鍖栧瓧鑺傘€?
    fn root_accumulator(&self) -> Vec<u8> {
        Vec::new()
    }

    fn record_count(&self) -> usize {
        0
    }

    fn storage_bytes(&self) -> u64 {
        0
    }

    fn current_root_hash(&self) -> RootHash {
        Vec::new()
    }

    fn export_prefix_segment(&self, _prefix_hex: &str) -> Result<Vec<u8>, String> {
        Err("prefix-segment export is not supported by this ADS".to_string())
    }

    fn import_prefix_segment(&mut self, _segment: &[u8]) -> Result<RootHash, String> {
        Err("prefix-segment import is not supported by this ADS".to_string())
    }

    fn drain_prefix_segment(&mut self, _prefix_hex: &str) -> Result<(Vec<u8>, RootHash), String> {
        Err("prefix-segment drain is not supported by this ADS".to_string())
    }

    fn prepare_retain_prefix_segment(
        &mut self,
        _prefix_hex: &str,
    ) -> Result<(Vec<u8>, RootHash), String> {
        Err("prefix-segment retain prepare is not supported by this ADS".to_string())
    }

    fn confirm_prefix_migration(&mut self, _prefix_hex: &str) -> Result<RootHash, String> {
        Err("prefix migration confirmation is not supported by this ADS".to_string())
    }

    fn reset(&mut self) -> Result<(), String> {
        Err("reset is not supported by this ADS".to_string())
    }
}

// ADS 瀹炵幇妯″潡
pub mod acctree_ads;
pub mod acctrie_ads;
pub mod mest_ads;
pub mod mpt;

// 瀵煎嚭 ADS 瀹炵幇
pub use acctree_ads::AccTreeAds;
pub use acctrie_ads::AccTrieAds;
pub use mest_ads::MestAds;
pub use mpt::MptAds;

/// 閫傞厤鍣細鎶婄幇鏈?`AdsOperations` 瀹炵幇鍖呰涓?`AuthenticatedDataStructure`
pub struct AccTrieAdapter(pub AccTrieAds);

pub struct MestAdapter(pub MestAds);

pub struct MptAdapter(pub MptAds);

impl AuthenticatedDataStructure for AccTrieAdapter {
    type Key = UnifiedKey;
    type Value = UnifiedValue;
    type Proof = Vec<u8>;
    type Database = ();

    fn insert(
        &mut self,
        key: Self::Key,
        value: Self::Value,
        _db: Option<&mut Self::Database>,
    ) -> anyhow::Result<Self::Proof> {
        let keyword = String::from_utf8_lossy(key.as_bytes()).to_string();
        let fid = value.as_string();
        let (proof, _root) = self.0.add(&keyword, &fid);
        Ok(proof)
    }

    fn query(
        &mut self,
        key: &Self::Key,
        _db: Option<&mut Self::Database>,
    ) -> anyhow::Result<Option<(Self::Value, Self::Proof)>> {
        let keyword = String::from_utf8_lossy(key.as_bytes()).to_string();
        let (fids, proof) = self.0.query(&keyword);
        if fids.is_empty() {
            Ok(None)
        } else {
            // 灏嗘墍鏈?fids 鐢ㄩ€楀彿鎷兼帴涓轰竴涓€艰繑鍥?
            Ok(Some((Self::Value::String(fids.join(",")), proof)))
        }
    }

    fn delete(
        &mut self,
        key: &Self::Key,
        _db: Option<&mut Self::Database>,
    ) -> anyhow::Result<Option<Self::Proof>> {
        let keyword = String::from_utf8_lossy(key.as_bytes()).to_string();
        // 鍒犻櫎鏃堕渶瑕佹彁渚?fid锛岀粺涓€鎺ュ彛鎶?value 浣滀负鍗曚竴瀛楃涓诧紙閫楀彿鍒嗛殧锛夌殑鍦烘櫙澶嶆潅锛?
        // 杩欓噷绾﹀畾锛氬綋璋冪敤鑰呮兂鍒犻櫎鍗曚釜 fid锛岄渶鍦?`UnifiedKey` 涓妸 fid 鏀惧埌 key 鍚庯紙涓嶅父鐢ㄥ満鏅級銆?
        // 涓哄吋瀹圭幇鏈変唬鐮侊紝鎴戜滑灏濊瘯鎶?keyword 鏈韩浣滀负鍏抽敭瀛楀苟杩斿洖绌?闈炵┖ proof 浠ユ寚绀烘槸鍚︽湁鍙樺寲銆?
        // 榛樿涓嶄紶 fid锛屽垯鏃犳硶绮剧‘鍒犻櫎鏌愪釜 fid 鈥斺€?鐢变笂灞傚喅瀹氬浣曡皟鐢ㄩ€傞厤鍣ㄣ€?
        let (proof, _root) = self.0.delete(&keyword, "");
        if proof.is_empty() {
            Ok(None)
        } else {
            Ok(Some(proof))
        }
    }

    fn verify(&self, proof: &Self::Proof) -> bool {
        !proof.is_empty()
    }

    fn ads_type(&self) -> &'static str {
        "AccTrie"
    }

    fn estimate_proof_size(proof: &Self::Proof) -> usize {
        proof.len()
    }
}

impl AuthenticatedDataStructure for MestAdapter {
    type Key = UnifiedKey;
    type Value = UnifiedValue;
    type Proof = Vec<u8>;
    type Database = ();

    fn insert(
        &mut self,
        key: Self::Key,
        value: Self::Value,
        _db: Option<&mut Self::Database>,
    ) -> anyhow::Result<Self::Proof> {
        let keyword = String::from_utf8_lossy(key.as_bytes()).to_string();
        let fid = value.as_string();
        let (proof, _root) = self.0.add(&keyword, &fid);
        Ok(proof)
    }

    fn query(
        &mut self,
        key: &Self::Key,
        _db: Option<&mut Self::Database>,
    ) -> anyhow::Result<Option<(Self::Value, Self::Proof)>> {
        let keyword = String::from_utf8_lossy(key.as_bytes()).to_string();
        let (fids, proof) = self.0.query(&keyword);
        if fids.is_empty() {
            Ok(None)
        } else {
            Ok(Some((Self::Value::String(fids.join(",")), proof)))
        }
    }

    fn delete(
        &mut self,
        key: &Self::Key,
        _db: Option<&mut Self::Database>,
    ) -> anyhow::Result<Option<Self::Proof>> {
        let keyword = String::from_utf8_lossy(key.as_bytes()).to_string();
        // 鍚屾牱绾﹀畾锛氬垹闄ゅ崟涓?fid 闇€瑕佷笂灞備紶鍏?fid 浣滀负 value锛涜繖閲屽皾璇曠┖ fid 鍒犻櫎閿紙鍙垹鍏ㄩ敭锛?
        let (proof, _root) = self.0.delete(&keyword, "");
        if proof.is_empty() {
            Ok(None)
        } else {
            Ok(Some(proof))
        }
    }

    fn verify(&self, proof: &Self::Proof) -> bool {
        !proof.is_empty()
    }

    fn ads_type(&self) -> &'static str {
        "MEST"
    }

    fn estimate_proof_size(proof: &Self::Proof) -> usize {
        proof.len()
    }
}

impl AuthenticatedDataStructure for MptAdapter {
    type Key = UnifiedKey;
    type Value = UnifiedValue;
    type Proof = Vec<u8>;
    type Database = ();

    fn insert(
        &mut self,
        key: Self::Key,
        value: Self::Value,
        _db: Option<&mut Self::Database>,
    ) -> anyhow::Result<Self::Proof> {
        let keyword = String::from_utf8_lossy(key.as_bytes()).to_string();
        let fid = value.as_string();
        let (proof, _root) = self.0.add(&keyword, &fid);
        Ok(proof)
    }

    fn query(
        &mut self,
        key: &Self::Key,
        _db: Option<&mut Self::Database>,
    ) -> anyhow::Result<Option<(Self::Value, Self::Proof)>> {
        let keyword = String::from_utf8_lossy(key.as_bytes()).to_string();
        let (fids, proof) = self.0.query(&keyword);
        if fids.is_empty() {
            Ok(None)
        } else {
            Ok(Some((Self::Value::String(fids.join(",")), proof)))
        }
    }

    fn delete(
        &mut self,
        key: &Self::Key,
        _db: Option<&mut Self::Database>,
    ) -> anyhow::Result<Option<Self::Proof>> {
        let keyword = String::from_utf8_lossy(key.as_bytes()).to_string();
        let (proof, _root) = self.0.delete(&keyword, "");
        if proof.is_empty() {
            Ok(None)
        } else {
            Ok(Some(proof))
        }
    }

    fn verify(&self, proof: &Self::Proof) -> bool {
        !proof.is_empty()
    }

    fn ads_type(&self) -> &'static str {
        "MPT"
    }

    fn estimate_proof_size(proof: &Self::Proof) -> usize {
        proof.len()
    }
}
