use manager::{EPRing, RouteMode, Router};
use std::env;
use std::fs::File;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let workload_path = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("crates/client/data/records_migration.csv"));
    let iterations: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let storager_count: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);

    let keywords = load_keywords(&workload_path)?;
    if keywords.is_empty() {
        anyhow::bail!("no keywords found in {}", workload_path.display());
    }

    let storager_addrs = (0..storager_count)
        .map(|idx| format!("http://127.0.0.1:{}", 50052 + idx as u16))
        .collect::<Vec<_>>();

    println!("Workload: {}", workload_path.display());
    println!("Keywords: {}", keywords.len());
    println!("Iterations: {}", iterations);
    println!("Storagers: {}", storager_count);

    let workloads = build_workloads(&keywords, iterations);
    let workload_hexes = workloads
        .iter()
        .map(|keyword| EPRing::keyword_to_hex(keyword))
        .collect::<Vec<_>>();

    let epring = bench_route_mode(
        RouteMode::Epring,
        &storager_addrs,
        &workloads,
        Some(&workload_hexes),
    );
    let chring = bench_route_mode(RouteMode::Chring, &storager_addrs, &workloads, None);

    print_result("epring", &epring);
    print_result("chring", &chring);

    let speedup = if epring.seconds > 0.0 {
        chring.seconds / epring.seconds
    } else {
        0.0
    };
    let faster = if epring.seconds < chring.seconds {
        "epring"
    } else if chring.seconds < epring.seconds {
        "chring"
    } else {
        "tie"
    };

    println!("Winner: {}", faster);
    println!("Speed ratio (chring/epring): {:.3}x", speedup);

    Ok(())
}

struct BenchResult {
    seconds: f64,
    per_op_ns: f64,
    throughput: f64,
}

fn bench_route_mode(
    route_mode: RouteMode,
    storager_addrs: &[String],
    workloads: &[String],
    workload_hexes: Option<&[String]>,
) -> BenchResult {
    let router = Router::new_with_mode(storager_addrs.to_vec(), 150, route_mode);
    let mut last_route = None;
    let route_count = workloads.len().max(1) as f64;

    let start = Instant::now();
    match (route_mode, workload_hexes) {
        (RouteMode::Epring, Some(hexes)) => {
            for key_hex in hexes {
                last_route = Some(black_box(router.route_key_hex(key_hex)));
            }
        }
        _ => {
            for keyword in workloads {
                last_route = Some(black_box(router.route_keyword(keyword)));
            }
        }
    }
    let elapsed = start.elapsed().as_secs_f64();

    let per_op_ns = elapsed * 1_000_000_000.0 / route_count;
    let throughput = route_count / elapsed.max(f64::MIN_POSITIVE);

    black_box(last_route);

    BenchResult {
        seconds: elapsed,
        per_op_ns,
        throughput,
    }
}

fn build_workloads(base_keywords: &[String], iterations: usize) -> Vec<String> {
    let mut workloads = Vec::with_capacity(iterations);
    for idx in 0..iterations {
        workloads.push(format!(
            "{}-{}",
            base_keywords[idx % base_keywords.len()],
            idx
        ));
    }
    workloads
}

fn load_keywords(path: &PathBuf) -> anyhow::Result<Vec<String>> {
    let file = File::open(path)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(file);

    let mut keywords = Vec::new();
    for record in reader.records() {
        let record = record?;
        for keyword in record.iter().skip(1).filter(|s| !s.is_empty()) {
            keywords.push(keyword.to_string());
        }
    }

    Ok(keywords)
}

fn print_result(label: &str, result: &BenchResult) {
    println!(
        "{label}: {:.6}s total, {:.2} ns/op, {:.2} ops/sec",
        result.seconds, result.per_op_ns, result.throughput
    );
}
