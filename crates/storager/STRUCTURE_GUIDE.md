# Storager 目录结构说明

## 📁 当前结构

```
crates/storager/
│
├── 📄 Cargo.toml                    # Storager 包配置
├── 📄 README.md                     # 使用说明
├── 📄 ARCHITECTURE.md               # 架构文档(详细)
│
├── 📦 ads_lib/                      # ADS 底层算法库(esa_rust crate)
│   ├── Cargo.toml                   # 独立 crate 配置
│   ├── src/
│   │   ├── crypto_accumulator/      # 密码学累加器完整实现
│   │   ├── mpt/                     # MPT 完整实现
│   │   ├── digest.rs                # 摘要工具
│   │   ├── set.rs                   # 集合操作
│   │   └── lib.rs                   # 库入口
│   └── tests/                       # 底层算法测试
│
├── 📂 src/                          # Storager 主代码
│   ├── ads/                         # ⭐ ADS 适配器层(统一接口)
│   │   ├── mod.rs                   # AdsOperations trait 定义
│   │   ├── README.md                # ADS 使用指南
│   │   ├── _template.rs             # 新 ADS 模板
│   │   │
│   │   ├── crypto_accumulator.rs    # 累加器适配器(使用 ads_lib)
│   │   ├── mpt.rs                   # MPT 适配器(使用 ads_lib)
│   │   │
│   │   ├── mest_ads.rs              # MEST 适配器
│   │   └── mest/                    # MEST 完整实现
│   │       ├── mod.rs               # MEHT 结构
│   │       ├── mgt.rs               # Merkle Group Tree
│   │       ├── seh.rs               # 可扩展哈希
│   │       ├── bucket.rs            # 数据桶
│   │       ├── kvpair.rs            # 键值对
│   │       ├── merkletree.rs        # Merkle 树
│   │       └── util.rs              # 工具函数
│   │
│   ├── storager.rs                  # Storager 核心结构
│   ├── service.rs                   # gRPC 服务
│   ├── main.rs                      # 程序入口
│   └── lib.rs                       # 库导出
│
└── 📂 examples/
    └── test_mest.rs                 # MEST 测试示例
```

## 🎯 为什么有 ads_lib?

**ads_lib** 是底层 ADS 算法的完整实现库:
- 包含复杂的密码学累加器实现(BLS12-381)
- 包含完整的 MPT 实现
- 可以被其他项目复用(虽然你现在不发布)
- 保持底层算法的独立性和可测试性

**src/ads/** 是适配器层:
- 将不同的 ADS 实现统一到 `AdsOperations` trait
- `crypto_accumulator.rs` 和 `mpt.rs` 是轻量级适配器,调用 ads_lib
- `mest/` 是直接实现(因为不在原始 ads_lib 中)

## 🔄 数据流

```
gRPC 请求
    ↓
service.rs (gRPC 服务)
    ↓
storager.rs (Storager 结构)
    ↓
src/ads/mod.rs (AdsOperations trait)
    ↓
    ├─→ crypto_accumulator.rs ──→ ads_lib/crypto_accumulator/
    ├─→ mpt.rs ──→ ads_lib/mpt/
    └─→ mest_ads.rs ──→ src/ads/mest/
```

## 💡 简化建议

如果你觉得目录层次太多,可以考虑:

### 方案 A: 保持现状(推荐)
- ✅ 代码已经工作
- ✅ 职责清晰
- ✅ 改动最小

### 方案 B: 删除 ads_lib
- 需要将 ads_lib 的代码移入 src/ads/
- 创建 `src/ads/crypto/` 和 `src/ads/mpt_impl/`  
- 更新所有 import
- ⚠️ 改动大,可能引入问题

## 📊 三种 ADS 对比

| ADS               | 实现位置         | 依赖             |
| ----------------- | ---------------- | ---------------- |
| CryptoAccumulator | ads_lib + 适配器 | BLS12-381, ark-* |
| MPT               | ads_lib + 适配器 | sha2, rocksdb    |
| MEST              | src/ads/mest/    | sha2, chrono     |

## 🚀 使用方式

```bash
# 切换 ADS 很简单
./scripts/start_system.sh -a mest        # 使用 MEST
./scripts/start_system.sh -a mpt         # 使用 MPT  
./scripts/start_system.sh -a accumulator # 使用累加器
```

## 🎨 核心文件

**最重要的文件:**
- `src/ads/mod.rs` - AdsOperations trait 定义(所有 ADS 必须实现)
- `src/storager.rs` - Storager 结构,通过 trait 使用 ADS
- `src/ads/README.md` - ADS 选择和使用指南

**如果要添加新 ADS:**
只需要关注 `src/ads/` 目录,参考 `_template.rs`

---

**总结:** 当前结构是合理的,`ads_lib` 提供底层算法,`src/ads/` 提供统一接口。如果你想完全扁平化,我可以帮你重构,但建议保持现状。
