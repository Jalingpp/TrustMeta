#!/usr/bin/env python3
import json, os, glob, csv
from pathlib import Path

# 搜索 metrics.json 在 logs/ 和 experiments/logs/
search_paths = ['logs/*/metrics.json', 'experiments/logs/*/metrics.json']
results = []
for pattern in search_paths:
    for path in glob.glob(pattern):
        try:
            with open(path, 'r', encoding='utf-8') as f:
                data = json.load(f)
        except Exception:
            continue
        # 推断 ADS 名称从目录名或文件名
        ads = 'unknown'
        parts = Path(path).parts
        for p in parts:
            if 'mpt' in p.lower():
                ads = 'MPT'
                break
            if 'mest' in p.lower():
                ads = 'MEST'
                break
            if 'acctrie' in p.lower() or 'acc' in p.lower():
                ads = 'AccTrie'
                break
        # extract fields safely
        op = data.get('operation_stats', {})
        lat = data.get('end_to_end_latency', {})
        total_throughput = data.get('total_throughput', 0.0)
        total_duration = data.get('total_duration', 0.0)
        success = data.get('success_count', 0)
        failure = data.get('failure_count', 0)
        results.append({
            'ads': ads,
            'path': path,
            'add': op.get('add_count', 0),
            'query': op.get('query_count', 0),
            'update': op.get('update_count', 0),
            'delete': op.get('delete_count', 0),
            'throughput': total_throughput,
            'total_duration_s': total_duration if isinstance(total_duration, (int, float)) else 0.0,
            'success': success,
            'failure': failure,
            'success_rate': (success / (success+failure)*100.0) if (success+failure)>0 else 0.0,
            'min_ms': lat.get('min_ms', 0.0),
            'avg_ms': lat.get('avg_ms', 0.0),
            'p95_ms': lat.get('p95_ms', 0.0),
            'p99_ms': lat.get('p99_ms', 0.0),
        })

# Group by ADS, pick the latest by path modification time
grouped = {}
for r in results:
    key = r['ads']
    if key not in grouped:
        grouped[key] = r
    else:
        # choose one with later mtime
        a = grouped[key]
        if os.path.getmtime(r['path']) > os.path.getmtime(a['path']):
            grouped[key] = r

# prepare output dir
out_dir = Path('experiments/results')
out_dir.mkdir(parents=True, exist_ok=True)
csv_path = out_dir / 'ads_comparison.csv'
md_path = out_dir / 'ads_comparison.md'

headers = ['ads','throughput','min_ms','avg_ms','p95_ms','p99_ms','success_rate','total_duration_s','add','query','update','delete']
with open(csv_path, 'w', newline='', encoding='utf-8') as cf:
    writer = csv.DictWriter(cf, fieldnames=headers)
    writer.writeheader()
    for ads, r in grouped.items():
        writer.writerow({k: r.get(k, '') for k in headers})

# generate simple markdown
with open(md_path, 'w', encoding='utf-8') as mf:
    mf.write('# ADS Performance Comparison\n\n')
    mf.write('| ADS | Throughput (ops/s) | Min (ms) | Avg (ms) | P95 (ms) | P99 (ms) | Success Rate (%) | Duration (s) | Add | Query | Update | Delete |\n')
    mf.write('|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n')
    for ads, r in grouped.items():
        mf.write(f"| {ads} | {r['throughput']:.2f} | {r['min_ms']:.3f} | {r['avg_ms']:.3f} | {r['p95_ms']:.3f} | {r['p99_ms']:.3f} | {r['success_rate']:.2f} | {r['total_duration_s']:.2f} | {r['add']} | {r['query']} | {r['update']} | {r['delete']} |\n")

print('Comparison generated:', csv_path, md_path)
