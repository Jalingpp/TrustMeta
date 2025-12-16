# Benchmark Summary — Medium Workload

Date: 2025-12-16

This file summarizes the medium-workload system benchmarks (3 storagers) for three ADS implementations.

## Summary

- **MPT**:
  - Operations: 40000 (Add/Query/Update/Delete each 10000)
  - Success Rate: 100.00%
  - Avg Latency: ~0.254 ms
  - Throughput: ~3935.55 ops/sec
  - Log: logs/benchmark_mpt_medium_3.log
  - Report: logs/system_test_mpt_20251216_105714

- **MEST**:
  - Operations: 40000
  - Success Rate: 100.00%
  - Avg Latency: ~0.284 ms
  - Throughput: ~3523.13 ops/sec
  - Log: logs/benchmark_mest_medium_3.log
  - Report: logs/system_test_mest_20251216_105748

- **AccTrie**:
  - Operations: 40000
  - Success Rate: 100.00%
  - Avg Latency: ~0.924 ms
  - Throughput: ~1082.03 ops/sec
  - Log: logs/benchmark_acctrie_medium_3.log
  - Report: logs/system_test_acctrie_20251216_105854

## Notes

- All tests run with `./scripts/run_system_benchmark.sh <ads> medium 3` and saved logs/reports under `logs/`.
- Next: if you want, I can (a) run small/large workloads, (b) commit the changes that removed batch APIs, or (c) create CSV of these results.
