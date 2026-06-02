
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
./scripts/startStorager.sh 3 acctrie 32 page
./scripts/startStorager.sh 172.23.166.114 3 acctrie 32 page
```

Parameters: `arg1` = storager count, or an optional IPv4 address filter followed by the count; `arg2` = `ads_mode`, `arg3` = MPT full-persist interval (default: `32`, only used by `mpt`), `arg4` = AccTrie persistence mode (`page|kvdb`, default: `page`). When an IP filter is provided, only storagers whose address in `scripts/data/snaddrs` matches that IP are started.
`startSNs.sh` is kept as a compatible alias.

When using the legacy `name,addr` format together with an IP filter, the script will auto-bind matched storagers to `0.0.0.0:<port>`.

`scripts/data/snaddrs` supports two formats:
1) `name,addr` (legacy; both bind and public address are `addr`)
2) `name,bind_addr,public_addr` (new; storager binds to `bind_addr`, manager connects via `public_addr`)


## Start Manager

```
./scripts/startManager.sh [storager_count] [ads_mode] [set_proof_mode] [split_threshold] [route_mode]
e.g. ./scripts/startManager.sh 90 acctrie accumulator 100000
./scripts/startManager.sh 90 acctrie accumulator 100000 epring
```

Parameters: `storager_count` is optional, `ads_mode` is `mpt|mest|acctrie|acctree`, `set_proof_mode` is `polynomial|accumulator`, `split_threshold` is the EPRing split threshold (default: `150`), and `route_mode` is `epring|chring` (default: `epring`).
The Manager startup command itself does not change for the recent scheduling optimizations; the new behavior is controlled by optional environment variables instead of new required CLI flags.
The manager bind/listen address is read from `scripts/data/manageraddrs` (or `MANAGER_BIND_ADDR_FILE`).
The manager public (externally reachable) address for clients is read from `scripts/data/managerpublicaddrs` (or `MANAGER_PUBLIC_ADDR_FILE` / `MANAGER_PUBLIC_ADDR`).
Manager fan-out can be bounded with `MANAGER_MAX_INFLIGHT_SUBREQUESTS` (global in-flight subrequests, default: `max(storager_count * 8, 8)`) and `MANAGER_MAX_INFLIGHT_PER_STORAGER` (per-storager in-flight subrequests, default: `8`).
Proof verification and boolean proof aggregation can be bounded with `MANAGER_MAX_BLOCKING_PROOF_TASKS` (default: available CPU parallelism, fallback: `4`).
Manager report writing is flushed by background tasks. `MANAGER_METRICS_FLUSH_INTERVAL_SECS` controls the manager metrics flush interval (default: `5`), and `MANAGER_PREFIX_REPORT_FLUSH_INTERVAL_SECS` controls the upload-prefix report flush interval (default: `5`).


## Start Clients

```
./scripts/startClients.sh
./scripts/startClients.sh acctrie accumulator
CLIENT_CONCURRENCY=4 ./scripts/startClients.sh acctrie accumulator
CLIENT_CONCURRENCY=32 CLIENT_UPLOAD_BATCH_SIZE=1024 ./scripts/startClients.sh acctrie accumulator
```

`CLIENT_CONCURRENCY` controls the maximum number of concurrent requests per workload inside a single client process; default is `1`.
`CLIENT_UPLOAD_BATCH_SIZE` controls the number of records sent per `batch_add` request when the client runs in `upload` or `upload-and-query` mode; default is `512`.
In interactive mode, you can type `upload <records> [count]`, `query <workload> [count]`, `update <updates> [count]`, `reset`, and `clear`.
For `upload/query/update`, `count` means "process only the first `count` records from the file".
`reset` and `clear` wipe the data stored on the manager and all storagers, while keeping the processes online.
The third argument of `startStorager.sh` configures the MPT full-persist interval; for example: `./scripts/startStorager.sh 3 mpt 64`.
The fourth argument of `startStorager.sh` configures AccTrie persistence; for example: `./scripts/startStorager.sh 3 acctrie 32 kvdb`.
The client also accepts `--concurrency <N>` internally; the shell script passes `CLIENT_CONCURRENCY` through to it.
The client startup command is unchanged, but the default upload behavior is different: `upload` and `upload-and-query` now use Manager `batch_add` by default. Use `--mode upload-sequential` to keep the legacy per-record add path when invoking the client binary directly.

Equivalent direct client CLI examples:
```
client --mode upload --upload-batch-size 1024
client --mode upload-sequential
client --mode upload-and-query
```

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
./scripts/collect_exp1_fig2.sh
./scripts/collect_exp1_fig3.sh
./scripts/collect_exp1_fig4.sh
./scripts/collect_exp2_fig1.sh
./scripts/collect_exp2_fig2.sh
./scripts/collect_exp2_fig3.sh
./scripts/collect_exp2_fig4.sh
./scripts/collect_exp3_fig1.sh
./scripts/collect_exp4_fig1.sh
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

### `collect_exp1_fig2.sh`
Output format:
`[dataset],[adsmode],[concurrency]:[query_throughput_per_sec],[average_query_latency_ms]ms`

Field meanings:
- `dataset`: dataset name from the client query report.
- `adsmode`: ADS mode.
- `concurrency`: query concurrency from the client report; legacy reports default to `1` if the field is absent.
- `query_throughput_per_sec`: query throughput from the client report.
- `average_query_latency_ms`: average query latency in milliseconds.

### `collect_exp1_fig3.sh`
Output format:
`[dataset],[adsmode],[concurrency]:[update_throughput_per_sec],[average_update_latency_ms]ms`

Field meanings:
- `dataset`: dataset name from the client update report.
- `adsmode`: ADS mode.
- `concurrency`: update concurrency from the client report; legacy reports default to `1` if the field is absent.
- `update_throughput_per_sec`: update throughput from the client report.
- `average_update_latency_ms`: average update latency in milliseconds.

### `collect_exp1_fig4.sh`
Output format:
`[dataset],[adsmode],[uploads number]:[avg_record_count]kv_pairs,[average_query_proof_size_bytes]bytes`

Field meanings:
- `dataset`: dataset name inferred from the latest client upload report for the same ADS mode.
- `adsmode`: ADS mode.
- `uploads number`: upload count inferred from the latest client upload report filename for the same ADS mode.
- `avg_record_count`: average `record_count` across all storager reports for that ADS mode.
- `average_query_proof_size_bytes`: average query proof size across all storager reports for that ADS mode.

### `collect_exp2_fig1.sh`
Usage:
`./scripts/collect_exp2_fig1.sh`

Output format:
`[dataset],[adsmode],[route_mode],[storager_count]:[average_storagers_per_boolean_query]`

Field meanings:
- `dataset`: dataset name read from the manager report.
- `adsmode`: ADS mode read from the manager output directory.
- `route_mode`: read from the manager report's `route_mode` field.
- `storager_count`: number of storagers managed by the manager.
- `average_storagers_per_boolean_query`: average number of storagers visited per boolean query.

### `collect_exp2_fig2.sh`
Usage:
`./scripts/collect_exp2_fig2.sh`

Output format:
`[dataset],[adsmode],[route_mode],[average_query_keyword_count]:[average_query_latency_ms]ms`

Field meanings:
- `dataset`: dataset name from the client query report.
- `adsmode`: ADS mode.
- `route_mode`: read from the manager report's `route_mode` field.
- `average_query_keyword_count`: average number of keywords per query.
- `average_query_latency_ms`: average query latency in milliseconds.

### `collect_exp2_fig3.sh`
Usage:
`./scripts/collect_exp2_fig3.sh`

Output format:
`[dataset],[adsmode],[route_mode],[concurrency]:[query_throughput_per_sec],[average_query_latency_ms]ms`

Field meanings:
- `dataset`: dataset name from the client query report.
- `adsmode`: ADS mode.
- `route_mode`: read from the client query report.
- `concurrency`: query concurrency from the client report; legacy reports default to `1` if the field is absent.
- `query_throughput_per_sec`: query throughput from the client report.
- `average_query_latency_ms`: average query latency in milliseconds.

### `collect_exp2_fig4.sh`
Usage:
`./scripts/collect_exp2_fig4.sh`

Output format:
`[dataset],[adsmode],[route_mode]:[storager_id 1],[record_count],[storage_bytes]B([storage_bytes_kb]KB);...`

Field meanings:
- `dataset`: dataset name read from the storager report.
- `adsmode`: ADS mode.
- `route_mode`: read from the storager report's `route_mode` field.
- `storager_id`: storager node identifier.
- `record_count`: upload-time number of key-value pairs stored on that node.
- `record_count_after_update`: post-update count when present.
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

### `collect_exp4_fig1.sh`
Usage:
`./scripts/collect_exp4_fig1.sh`

Output format:
`[dataset],[adsmode],[persistence_mode],[record number]:[split_migration_total_duration_ms]ms,[split_migration_count],[payload]mb,[io_total_mb]mb,[io_amp_ratio]`

Field meanings:
- `dataset`: dataset name from the manager report.
- `adsmode`: ADS mode read from the manager report's `route_mode` field.
- `persistence_mode`: persistence mode read from the manager report.
- `record number`: upload record count from the manager report.
- `split_migration_total_duration_ms`: total split migration duration in milliseconds.
- `split_migration_count`: number of split migrations.
- `payload`: payload size in MB from `split_migration_io`.
- `io_total_mb`: total I/O volume in MB from `split_migration_io`.
- `io_amp_ratio`: I/O amplification ratio from `split_migration_io`.
