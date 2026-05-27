
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

Parameters: `arg1` = storager count, `arg2` = `ads_mode`, `arg3` = MPT full-persist interval (default: `32`, only used by `mpt`).
`startSNs.sh` is kept as a compatible alias.


## Start Manager

```
./scripts/startManager.sh [storager_count] [ads_mode] [set_proof_mode] [split_threshold]
e.g. ./scripts/startManager.sh 3 acctrie accumulator 100000
```

Parameters: `storager_count` is optional, `ads_mode` is `mpt|mest|acctrie|acctree`, `set_proof_mode` is `polynomial|accumulator`, and `split_threshold` is the EPRing split threshold (default: `150`).
The manager listen address is read from `scripts/data/manageraddrs`.


## Start Clients

```
./scripts/startClients.sh
./scripts/startClients.sh acctrie accumulator
```

In interactive mode, you can type `upload <records> [count]`, `query <workload> [count]`, `update <updates> [count]`, `reset`, and `clear`.
For `upload/query/update`, `count` means "process only the first `count` records from the file".
`reset` and `clear` wipe the data stored on the manager and all storagers, while keeping the processes online.
The third argument of `startStorager.sh` configures the MPT full-persist interval; for example: `./scripts/startStorager.sh 3 mpt 64`.

Examples:
```
upload /root/TrustMeta/scripts/input/testdata/records_minimal.csv
upload /root/TrustMeta/scripts/input/testdata/records_minimal.csv 100
query /root/TrustMeta/scripts/input/testdata/query_minimal.txt 20
update /root/TrustMeta/scripts/input/testdata/update_minimal.csv 10
reset
clear
```

Outputs are written to `scripts/output/clients/<ads_mode>/`, `scripts/output/manager/<ads_mode>/`, and `scripts/output/storagers/<ads_mode>/`.
Detailed logs are written to `scripts/logs/`.

## Exp Data Scripts

```
./scripts/collect_exp1_fig1.sh
./scripts/collect_exp1_fig4.sh
./scripts/collect_exp2_fig1.sh OAGPub epring
./scripts/collect_exp2_fig2.sh epring
./scripts/collect_exp2_fig4.sh OAGPub epring
./scripts/collect_exp3_fig1.sh
```

### `collect_exp1_fig1.sh`
Output format:
`[dataset],[adsmode],[record_number]:[total_duration_ms]ms,[average_insert_latency_ms]ms`

Field meanings:
- `dataset`: dataset name from the client report.
- `adsmode`: ADS mode used by the client.
- `record_number`: number of uploaded records.
- `total_duration_ms`: total upload time in milliseconds.
- `average_insert_latency_ms`: average insert latency per record in milliseconds.

### `collect_exp1_fig4.sh`
Output format:
`[dataset],[adsmode],[avg_record_count]kv_pairs:[average_query_proof_size_bytes]bytes`

Field meanings:
- `dataset`: dataset name inferred from the latest client upload report for the same ADS mode.
- `adsmode`: ADS mode.
- `avg_record_count`: average `record_count` across all storager reports for that ADS mode.
- `average_query_proof_size_bytes`: average query proof size across all storager reports for that ADS mode.

### `collect_exp2_fig1.sh`
Usage:
`./scripts/collect_exp2_fig1.sh <dataset> <hashmode>`

Output format:
`[dataset],[adsmode],[hashmode],[storager_count]:[average_storagers_per_boolean_query]`

Field meanings:
- `dataset`: script argument.
- `adsmode`: ADS mode read from the manager output directory.
- `hashmode`: script argument.
- `storager_count`: number of storagers managed by the manager.
- `average_storagers_per_boolean_query`: average number of storagers visited per boolean query.

### `collect_exp2_fig2.sh`
Usage:
`./scripts/collect_exp2_fig2.sh <hashmode>`

Output format:
`[dataset],[adsmode],[hashmode],[average_query_keyword_count]:[average_query_latency_ms]ms`

Field meanings:
- `dataset`: dataset name from the client query report.
- `adsmode`: ADS mode.
- `hashmode`: script argument.
- `average_query_keyword_count`: average number of keywords per query.
- `average_query_latency_ms`: average query latency in milliseconds.

### `collect_exp2_fig4.sh`
Usage:
`./scripts/collect_exp2_fig4.sh <dataset> <hashmode>`

Output format:
`[dataset],[adsmode],[hashmode]:[storager_id 1],[record_count],[storage_bytes]B([storage_bytes_kb]KB);...`

Field meanings:
- `dataset`: script argument.
- `adsmode`: ADS mode.
- `hashmode`: script argument.
- `storager_id`: storager node identifier.
- `record_count`: number of key-value pairs stored on that node.
- `storage_bytes`: node storage size in bytes, also shown in KB.

### `collect_exp3_fig1.sh`
Output format:
`[dataset],[adsmode],[average_query_keyword_count]:[manager_aggregation_ms]ms,[average_proof_size_bytes]B,[average_proof_verification_latency_ms]ms`

Field meanings:
- `dataset`: dataset name from the client query report.
- `adsmode`: ADS mode.
- `average_query_keyword_count`: average number of keywords per query.
- `manager_aggregation_ms`: sum of manager proof aggregation time and set-operation proof generation time, shown in milliseconds.
- `average_proof_size_bytes`: average proof size in bytes.
- `average_proof_verification_latency_ms`: average client-side proof verification latency in milliseconds.
