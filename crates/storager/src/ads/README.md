# 分布式存储系统 - ADS 配置说明

## 支持的 ADS 类型

本系统支持三种认证数据结构(Authenticated Data Structure):

### 1. CryptoAccumulator (密码学累加器)
- **技术**: 基于 BLS12-381 椭圆曲线的密码学累加器
- **特点**: 
  - 证明大小固定(约 200 字节)
  - 安全性高,基于数学难题
  - 适合需要强密码学保证的场景
- **性能**: 
  - 添加/删除: O(n) - 需要重新计算累加器
  - 查询: O(1)
  - 证明大小: ~201 字节

### 2. MPT (Merkle Patricia Trie)
- **技术**: 以太坊风格的 Merkle Patricia Trie
- **特点**:
  - 树形结构,支持高效的键值存储
  - 证明大小与树的深度相关
  - 广泛应用于区块链系统
- **性能**:
  - 添加/删除: O(log n)
  - 查询: O(log n)
  - 证明大小: 32 字节(根哈希)

### 3. MEST (Merkle-based Extendible Segmented Hash Tree)
- **技术**: 基于 Merkle 树的可扩展分段哈希树
- **特点**:
  - 结合了 MGT (Merkle Group Tree) 和 SEH (Segmented Extendible Hashing)
  - 动态扩展能力强
  - 适合大规模数据存储
- **性能**:
  - 添加/删除: O(log n)
  - 查询: O(log n)  
  - 证明大小: 32 字节(MGT 根哈希)

## ADS 文件结构

```
crates/storager/src/ads/
├── mod.rs                      # ADS 模块定义和通用接口
├── _template.rs                # ADS 实现模板
├── crypto_accumulator.rs       # 密码学累加器实现
├── mpt.rs                      # MPT 实现
├── mest_ads.rs                 # MEST 适配器
└── mest/                       # MEST 核心实现
    ├── mod.rs                  # MEST 模块导出
    ├── meht.rs                 # Merkle Extendible Hash Table
    ├── mgt.rs                  # Merkle Group Tree
    ├── seh.rs                  # Segmented Extendible Hashing
    ├── bucket.rs               # 数据桶
    ├── kvpair.rs               # 键值对
    ├── merkletree.rs           # Merkle 树基础实现
    └── util.rs                 # 工具函数
```

## 如何切换 ADS 类型

### 方法 1: 使用通用启动脚本

```bash
# 使用 MEST (默认)
./scripts/start_system.sh

# 使用 MPT
./scripts/start_system.sh -a mpt

# 使用密码学累加器
./scripts/start_system.sh -a accumulator

# 使用 Debug 模式 + MPT
./scripts/start_system.sh -a mpt -m debug

# 启动 5 个 Storager 节点
./scripts/start_system.sh -n 5
```

### 方法 2: 使用专用启动脚本

```bash
# 使用 start.sh (默认 MPT)
./scripts/start.sh

# 使用 start_mest.sh (MEST)
./scripts/start_mest.sh
```

### 方法 3: 手动启动

```bash
# 启动 Manager
./target/release/manager --ads-mode <MODE>

# 启动 Storager
./target/release/storager <PORT> <ADS_TYPE>
```

其中:
- `<MODE>`: accumulator | mpt | mest
- `<ADS_TYPE>`: accumulator | crypto | mpt | mest

### 方法 4: 修改配置文件

编辑 `config.json`:

```json
{
    "ads_mode": "MEST",
    ...
}
```

然后不带参数启动 Storager,它会从配置文件读取。

## ADS 选择建议

### 使用场景建议

| 场景         | 推荐 ADS          | 原因                |
| ------------ | ----------------- | ------------------- |
| 区块链应用   | MPT               | 以太坊兼容,成熟稳定 |
| 高安全性需求 | CryptoAccumulator | 强密码学保证        |
| 大规模数据   | MEST              | 动态扩展性好        |
| 原型开发     | MPT/MEST          | 实现简单,性能好     |
| 生产环境     | 根据需求选择      | 需要进行性能测试    |

### 性能对比

| ADS               | 添加 | 查询 | 删除 | 证明大小  | 内存占用 |
| ----------------- | ---- | ---- | ---- | --------- | -------- |
| CryptoAccumulator | 慢   | 快   | 慢   | 大(~201B) | 小       |
| MPT               | 中   | 中   | 中   | 小(32B)   | 中       |
| MEST              | 快   | 快   | 快   | 小(32B)   | 中       |

## 添加新的 ADS 实现

### 步骤 1: 创建实现文件

在 `crates/storager/src/ads/` 下创建新文件,例如 `my_ads.rs`:

```rust
use super::AdsOperations;
use common::RootHash;

pub struct MyAds {
    // 你的数据结构
}

impl MyAds {
    pub fn new() -> Self {
        MyAds { /* ... */ }
    }
}

impl AdsOperations for MyAds {
    fn add(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        // 实现添加逻辑
    }

    fn query(&self, keyword: &str) -> (Vec<String>, Vec<u8>) {
        // 实现查询逻辑
    }

    fn delete(&mut self, keyword: &str, fid: &str) -> (Vec<u8>, RootHash) {
        // 实现删除逻辑
    }
}
```

### 步骤 2: 在 mod.rs 中注册

在 `crates/storager/src/ads/mod.rs` 中添加:

```rust
pub mod my_ads;
pub use my_ads::MyAds;
```

### 步骤 3: 在 common/types.rs 中添加枚举

在 `crates/common/src/types.rs` 的 `AdsMode` 中添加:

```rust
pub enum AdsMode {
    CryptoAccumulator,
    Mpt,
    Mest,
    MyAds,  // 新增
}
```

### 步骤 4: 更新 Storager

在 `crates/storager/src/storager.rs` 中添加工厂方法:

```rust
impl Storager {
    pub fn with_my_ads() -> Self {
        Storager {
            ads: Arc::new(RwLock::new(Box::new(MyAds::new()))),
        }
    }
    
    pub fn from_config(ads_type: &str) -> Self {
        match ads_type.to_lowercase().as_str() {
            // ... 现有的匹配
            "myads" => Self::with_my_ads(),
            _ => { /* ... */ }
        }
    }
}
```

### 步骤 5: 更新 Manager 验证逻辑

在 `crates/manager/src/core/verification.rs` 中添加验证方法:

```rust
impl ProofVerifier {
    pub fn verify(&self, proof: &[u8], _root_hash: &[u8]) -> bool {
        match self.ads_mode {
            // ... 现有的匹配
            AdsMode::MyAds => self.verify_my_ads(proof),
        }
    }
    
    fn verify_my_ads(&self, proof: &[u8]) -> bool {
        // 实现验证逻辑
    }
}
```

### 步骤 6: 测试

```bash
# 编译测试
cargo build --release

# 启动系统
./scripts/start_system.sh -a myads

# 运行测试
cargo run --release --example integration_test
```

## 测试各种 ADS

```bash
# 测试 MEST
./scripts/start_system.sh -a mest
cargo run --release --example integration_test

# 测试 MPT  
./scripts/start_system.sh -a mpt
cargo run --release --example integration_test

# 测试密码学累加器
./scripts/start_system.sh -a accumulator
cargo run --release --example integration_test
```

## 故障排除

### 问题: Manager 和 Storager ADS 类型不匹配

**症状**: "Proof verification failed" 错误

**解决**: 确保 Manager 和所有 Storager 使用相同的 ADS 模式

```bash
# 检查日志
tail -f logs/manager.log
tail -f logs/storager1.log
```

### 问题: 编译错误

**解决**: 检查是否在所有必要的地方添加了新的 ADS 枚举变体

```bash
cargo build 2>&1 | grep "non-exhaustive patterns"
```

### 问题: 性能问题

**解决**: 使用 Release 模式并进行性能分析

```bash
# 使用 Release 模式
./scripts/start_system.sh -m release

# 性能测试
cargo build --release
time cargo run --release --example integration_test
```
