import subprocess
import re
import os
import time
import statistics

MODES = ["mpt", "mest", "acctrie"]
WORKLOAD = "medium"

def run_benchmark(mode):
    print(f"Running benchmark for {mode}...")
    subprocess.run(["./scripts/run_system_benchmark.sh", mode, WORKLOAD], check=True)

def parse_logs(mode):
    metrics = {
        "Proof Generation (Add)": [],
        "Proof Generation (Query)": [],
        "Proof Generation (Delete)": [],
        "Proof Verification (Add)": [],
        "Proof Verification (Query)": [],
        "Proof Verification (Delete)": [],
        "Proof Verification (Update-Delete)": [],
        "Proof Verification (Update-Add)": [],
    }

    # Parse Manager logs
    if os.path.exists("logs/manager.log"):
        with open("logs/manager.log", "r") as f:
            for line in f:
                match = re.search(r"\[METRIC\] (Proof Verification .*?): (.*)", line)
                if match:
                    metric_name = match.group(1)
                    duration_str = match.group(2)
                    duration_us = parse_duration(duration_str)
                    if metric_name in metrics:
                        metrics[metric_name].append(duration_us)
                    else:
                        # Handle variations like "Proof Verification" (generic)
                        if metric_name == "Proof Verification":
                             # We might not know which op it is, but let's store it
                             pass

    # Parse Storager logs
    for filename in os.listdir("logs"):
        if filename.startswith("storager_") and filename.endswith(".log"):
            with open(os.path.join("logs", filename), "r") as f:
                for line in f:
                    match = re.search(r"\[METRIC\] (Proof Generation .*?): (.*)", line)
                    if match:
                        metric_name = match.group(1)
                        duration_str = match.group(2)
                        duration_us = parse_duration(duration_str)
                        if metric_name in metrics:
                            metrics[metric_name].append(duration_us)

    with open("benchmark_results_large.txt", "a") as f:
        f.write(f"\nResults for {mode}:\n")
        f.write("-" * 40 + "\n")
        for metric_name, values in metrics.items():
            if values:
                avg_val = statistics.mean(values)
                min_val = min(values)
                max_val = max(values)
                f.write(f"{metric_name}:\n")
                f.write(f"  Count: {len(values)}\n")
                f.write(f"  Avg:   {avg_val:.2f} µs\n")
                f.write(f"  Min:   {min_val:.2f} µs\n")
                f.write(f"  Max:   {max_val:.2f} µs\n")
            else:
                f.write(f"{metric_name}: No data\n")
        f.write("-" * 40 + "\n")

    print(f"\nResults for {mode} saved to benchmark_results_large.txt")

def parse_duration(duration_str):
    # Format is usually like "123.45µs" or "1.23ms" or "123ns"
    # Rust Debug format for Duration: "123.45µs"
    # But wait, I used {:?} which produces "123.45µs"
    
    # Simple parsing logic
    if "ns" in duration_str:
        return float(duration_str.replace("ns", "")) / 1000.0
    elif "µs" in duration_str:
        return float(duration_str.replace("µs", ""))
    elif "ms" in duration_str:
        return float(duration_str.replace("ms", "")) * 1000.0
    elif "s" in duration_str:
        return float(duration_str.replace("s", "")) * 1000000.0
    else:
        # Fallback or error
        return 0.0

def main():
    for mode in MODES:
        try:
            run_benchmark(mode)
            parse_logs(mode)
            # Backup logs
            os.system(f"cp logs/manager.log logs/manager_{mode}.log")
            os.system(f"cp logs/storager_50052.log logs/storager_50052_{mode}.log")
        except Exception as e:
            print(f"Error running {mode}: {e}")

if __name__ == "__main__":
    main()
