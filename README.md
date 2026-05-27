
## Install Cargo

```
curl https://sh.rustup.rs -sSf | sh -s -- -y
echo 'source "$HOME/.cargo/env"' >> "$HOME/.bashrc"
source "$HOME/.bashrc"
cargo --version
rustc --version
```

## Start Storagers

```
./scripts/startStorager.sh 3 acctrie 32
```

params: arg1 = storager number, arg2 = adsmode, arg3 = MPT full-persist interval (default 32; only used by mpt)
`startSNs.sh` is kept as a compatible alias.


## Start Manager

```
./scripts/startManager.sh [storager_count] [ads_mode] [set_proof_mode] [split_threshold]
e.g. ./scripts/startManager.sh 3 acctrie accumulator 100000
```

params: storager_count optional, ads_mode: mpt|mest|acctrie|acctree, set_proof_mode: polynomial|accumulator, split_threshold: EPRing 分裂阈值（默认 150）
manager listen address is read from `scripts/data/manageraddrs`


## Start Clients

```
./scripts/startClients.sh
./scripts/startClients.sh acctrie accumulator
```

进入交互模式后可输入 `upload <records> [count]`, `query <workload> [count]`, `update <updates> [count]`, `reset`, `clear`。
其中 `upload/query/update` 的 `count` 表示只处理文件中的前 `count` 条记录；`reset` 和 `clear` 会清空 manager 和所有 storager 上的数据，但保持进程在线。
当使用 `mpt` 或 `acctree` 时，`upload` 过程中触发的前缀分裂会在后台异步执行，不阻塞 `upload` 返回。
`startStorager.sh` 的第三个参数可设置 MPT 的 full-persist 间隔，默认 32；例如 `./scripts/startStorager.sh 3 mpt 64`。

示例：
```
upload /root/TrustMeta/scripts/input/testdata/records_minimal.csv
upload /root/TrustMeta/scripts/input/testdata/records_minimal.csv 100
query /root/TrustMeta/scripts/input/testdata/query_minimal.txt 20
update /root/TrustMeta/scripts/input/testdata/update_minimal.csv 10
reset
clear
```

输出会分别写入 `scripts/output/clients/<ads_mode>/`、`scripts/output/manager/<ads_mode>/`、`scripts/output/storagers/<ads_mode>/`。
详细日志写入 `scripts/logs/` 目录。

## Exp Data Scripts

```
./scripts/collect_exp1_fig1.sh
./scripts/collect_exp1_fig4.sh
./scripts/collect_exp2_fig1.sh OAGPub epring
./scripts/collect_exp2_fig2.sh epring
./scripts/collect_exp2_fig4.sh OAGPub epring
./scripts/collect_exp3_fig1.sh
```

说明：
`collect_exp2_fig1.sh` 的参数顺序为 `dataset hashmode`。
`collect_exp2_fig2.sh` 的参数为 `hashmode`。
`collect_exp2_fig4.sh` 的参数顺序为 `dataset hashmode`。
`collect_exp3_fig1.sh` 无需参数。
