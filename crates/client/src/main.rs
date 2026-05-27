use client::client::{Client, QueryKeywordMetrics};
use common::{
    config::load_manager_http_addr_from_file, init_accumulator_public_parameters,
    metrics_output, parse_boolean_expr, AdsMode, SetProofMode,
};
use std::fs;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const DEFAULT_INPUT_DIR: &str = "crates/client/data";
const DEFAULT_RECORDS_FILE: &str = "records.csv";
const DEFAULT_QUERY_FILE: &str = "query_workload.txt";
const DEFAULT_UPDATE_FILE: &str = "update_workload.txt";

struct InputRecord {
    fid: String,
    keywords: Vec<String>,
}

struct UpdateRecord {
    fid: String,
    old_keywords: Vec<String>,
    new_keywords: Vec<String>,
}

struct BulkUploadMetrics {
    total_records: usize,
    total_keyword_pairs: usize,
    total_duration: Duration,
    total_insert_latency: Duration,
    total_proof_verification_latency: Duration,
}

struct BulkQueryMetrics {
    total_queries: usize,
    total_proof_size_bytes: usize,
    total_query_keyword_count: usize,
    total_duration: Duration,
    total_query_latency: Duration,
    total_proof_verification_latency: Duration,
    total_manager_proof_aggregation_latency: Duration,
    total_manager_set_operation_proof_generation_latency: Duration,
}

struct BulkUpdateMetrics {
    total_updates: usize,
    total_keyword_pairs: usize,
    total_duration: Duration,
    total_update_latency: Duration,
    total_proof_verification_latency: Duration,
}

enum OperationMode {
    Upload,
    UploadSequential,
    Query,
    Update,
    UploadAndQuery,
    Reset,
}

impl OperationMode {
    fn from_arg(value: &str) -> Result<Self, String> {
        match value.to_lowercase().as_str() {
            "upload-sequential" | "load-sequential" => Ok(Self::UploadSequential),
            "upload" | "load" => Ok(Self::Upload),
            "query" => Ok(Self::Query),
            "update" => Ok(Self::Update),
            "upload-and-query" | "load-and-query" => Ok(Self::UploadAndQuery),
            "reset" | "clear" => Ok(Self::Reset),
            other => Err(format!(
                "Unknown operation mode: {}. Expected upload|upload-sequential|query|update|upload-and-query|reset",
                other
            )),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut manager_addr =
        load_manager_http_addr_from_file().unwrap_or_else(|| "http://127.0.0.1:50051".to_string());
    let mut ads_mode = AdsMode::AccTrie;
    let mut set_proof_mode = SetProofMode::Accumulator;
    let mut input_dir = PathBuf::from(DEFAULT_INPUT_DIR);
    let mut client_id: u32 = std::env::var("CLIENT_ID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let mut report_count: Option<usize> = None;
    let mut records_file: Option<PathBuf> = None;
    let mut query_file: Option<PathBuf> = None;
    let mut update_file: Option<PathBuf> = None;
    let mut operation_mode = OperationMode::UploadAndQuery;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--manager-addr" | "-m" => {
                if i + 1 < args.len() {
                    manager_addr = args[i + 1].clone();
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--ads-mode" | "-a" => {
                if i + 1 < args.len() {
                    ads_mode = args[i + 1].parse().unwrap_or_else(|err| {
                        eprintln!("{}; using default ({})", err, AdsMode::AccTrie);
                        AdsMode::AccTrie
                    });
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--input-dir" | "-i" => {
                if i + 1 < args.len() {
                    input_dir = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--client-id" | "-c" => {
                if i + 1 < args.len() {
                    client_id = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("Invalid client id: {}, using default (1)", args[i + 1]);
                        1
                    });
                    if client_id == 0 {
                        client_id = 1;
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--report-count" => {
                if i + 1 < args.len() {
                    report_count = args[i + 1].parse::<usize>().ok().filter(|value| *value > 0);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--set-proof-mode" => {
                if i + 1 < args.len() {
                    set_proof_mode = args[i + 1].parse().unwrap_or_else(|err| {
                        eprintln!("{}; using default ({})", err, SetProofMode::Accumulator);
                        SetProofMode::Accumulator
                    });
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--records-file" => {
                if i + 1 < args.len() {
                    records_file = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--query-file" => {
                if i + 1 < args.len() {
                    query_file = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--update-file" => {
                if i + 1 < args.len() {
                    update_file = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--mode" => {
                if i + 1 < args.len() {
                    operation_mode = OperationMode::from_arg(&args[i + 1])?;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            _ => {
                i += 1;
            }
        }
    }

    let records_file = records_file.unwrap_or_else(|| input_dir.join(DEFAULT_RECORDS_FILE));
    let query_file = query_file.unwrap_or_else(|| input_dir.join(DEFAULT_QUERY_FILE));
    let update_file = update_file.unwrap_or_else(|| input_dir.join(DEFAULT_UPDATE_FILE));
    let dataset_label = metrics_output::dataset_label_from_path(&input_dir);

    init_accumulator_public_parameters()?;

    println!("Client connecting to: {}", manager_addr);
    println!("Client dataset: {}", dataset_label);
    println!("Client verification ADS mode: {:?}", ads_mode);
    println!("Client boolean set proof mode: {}", set_proof_mode);
    println!("Operation mode: {}", describe_mode(&operation_mode));

    let manager_addr_report = manager_addr.clone();
    let mut client = Client::new(manager_addr, ads_mode, set_proof_mode);

    if matches!(
        operation_mode,
        OperationMode::Upload | OperationMode::UploadSequential | OperationMode::UploadAndQuery
    ) {
        let records = load_records(&records_file)?;
        println!("Records file: {}", records_file.display());
        println!("Loaded {} input records", records.len());
        let metrics = run_bulk_put(&mut client, &records).await?;
        write_upload_report(
            &metrics,
            &dataset_label,
            client_id,
            report_count,
            &manager_addr_report,
            ads_mode,
            set_proof_mode,
        )?;
    }

    if matches!(operation_mode, OperationMode::Update) {
        let updates = load_update_workload(&update_file)?;
        println!("Update workload file: {}", update_file.display());
        println!("Loaded {} update records", updates.len());
        let metrics = run_bulk_updates(&mut client, &updates).await?;
        write_update_report(
            &metrics,
            &dataset_label,
            client_id,
            report_count,
            &manager_addr_report,
            ads_mode,
            set_proof_mode,
        )?;
    }

    if matches!(
        operation_mode,
        OperationMode::Query | OperationMode::UploadAndQuery
    ) {
        let queries = load_query_workload(&query_file)?;
        println!("Query workload file: {}", query_file.display());
        println!("Loaded {} query expressions", queries.len());
        let metrics = run_bulk_queries(&mut client, &queries).await?;
        write_query_report(
            &metrics,
            &dataset_label,
            client_id,
            report_count,
            &manager_addr_report,
            ads_mode,
            set_proof_mode,
        )?;
    }

    if matches!(operation_mode, OperationMode::Reset) {
        client.reset_system().await?;
    }

    Ok(())
}

#[allow(dead_code)]
async fn run_bulk_put(
    client: &mut Client,
    records: &[InputRecord],
) -> Result<BulkUploadMetrics, Box<dyn std::error::Error>> {
    let total_start = Instant::now();
    let mut total_insert_latency = Duration::from_secs(0);
    let mut total_proof_verification_latency = Duration::from_secs(0);
    let mut last_progress_bucket = 0usize;
    let total_keyword_pairs = records
        .iter()
        .map(|record| {
            record
                .keywords
                .iter()
                .cloned()
                .collect::<HashSet<_>>()
                .len()
        })
        .sum::<usize>();
    for (idx, record) in records.iter().enumerate() {
        let record_start = Instant::now();
        let verification_latency = client
            .put_file(
                record.fid.clone(),
                record.keywords.clone(),
                total_keyword_pairs as u32,
            )
            .await?;
        total_proof_verification_latency += verification_latency;
        total_insert_latency += record_start.elapsed();
        print_progress("upload", idx + 1, records.len(), &mut last_progress_bucket);
    }
    Ok(BulkUploadMetrics {
        total_records: records.len(),
        total_keyword_pairs,
        total_duration: total_start.elapsed(),
        total_insert_latency,
        total_proof_verification_latency,
    })
}

async fn run_bulk_updates(
    client: &mut Client,
    updates: &[UpdateRecord],
) -> Result<BulkUpdateMetrics, Box<dyn std::error::Error>> {
    let total_start = Instant::now();
    let mut total_update_latency = Duration::from_secs(0);
    let mut total_proof_verification_latency = Duration::from_secs(0);
    let mut last_progress_bucket = 0usize;
    let mut total_keyword_pairs = 0usize;
    println!("=== Bulk Update File ===");
    for (idx, update) in updates.iter().enumerate() {
        let record_start = Instant::now();
        let keyword_pairs = update
            .old_keywords
            .iter()
            .chain(update.new_keywords.iter())
            .cloned()
            .collect::<HashSet<_>>()
            .len();
        total_keyword_pairs += keyword_pairs;
        println!(
            "[{}/{}] Update fid={} old={} new={}",
            idx + 1,
            updates.len(),
            update.fid,
            update.old_keywords.len(),
            update.new_keywords.len()
        );
        let verification_latency = client
            .update_file(
                update.fid.clone(),
                update.old_keywords.clone(),
                update.new_keywords.clone(),
            )
            .await?;
        total_proof_verification_latency += verification_latency;
        total_update_latency += record_start.elapsed();
        print_progress("update", idx + 1, updates.len(), &mut last_progress_bucket);
    }
    Ok(BulkUpdateMetrics {
        total_updates: updates.len(),
        total_keyword_pairs,
        total_duration: total_start.elapsed(),
        total_update_latency,
        total_proof_verification_latency,
    })
}

async fn run_bulk_queries(
    client: &mut Client,
    queries: &[String],
) -> Result<BulkQueryMetrics, Box<dyn std::error::Error>> {
    let total_start = Instant::now();
    let mut total_proof_size_bytes = 0usize;
    let mut total_query_keyword_count = 0usize;
    let mut total_query_latency = Duration::from_secs(0);
    let mut total_proof_verification_latency = Duration::from_secs(0);
    let mut total_manager_proof_aggregation_latency = Duration::from_secs(0);
    let mut total_manager_set_operation_proof_generation_latency = Duration::from_secs(0);
    let mut last_progress_bucket = 0usize;
    for (idx, query) in queries.iter().enumerate() {
        let expr = parse_boolean_expr(query).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("Invalid query expression: {}", err),
            )
        })?;
        total_query_keyword_count += expr.get_keywords().len();

        let query_start = Instant::now();
        let QueryKeywordMetrics {
            proof_size_bytes,
            verification_latency,
            manager_proof_aggregation_latency,
            manager_set_operation_proof_generation_latency,
            ..
        } = client.query_by_func(query.clone()).await?;
        total_proof_size_bytes += proof_size_bytes;
        total_proof_verification_latency += verification_latency;
        total_manager_proof_aggregation_latency += manager_proof_aggregation_latency;
        total_manager_set_operation_proof_generation_latency +=
            manager_set_operation_proof_generation_latency;
        total_query_latency += query_start.elapsed();
        print_progress("query", idx + 1, queries.len(), &mut last_progress_bucket);
    }
    Ok(BulkQueryMetrics {
        total_queries: queries.len(),
        total_proof_size_bytes,
        total_query_keyword_count,
        total_duration: total_start.elapsed(),
        total_query_latency,
        total_proof_verification_latency,
        total_manager_proof_aggregation_latency,
        total_manager_set_operation_proof_generation_latency,
    })
}

fn describe_mode(mode: &OperationMode) -> &'static str {
    match mode {
        OperationMode::UploadSequential => "upload-sequential",
        OperationMode::Upload => "upload",
        OperationMode::Query => "query",
        OperationMode::Update => "update",
        OperationMode::UploadAndQuery => "upload-and-query",
        OperationMode::Reset => "reset",
    }
}

fn print_progress(kind: &str, done: usize, total: usize, last_progress_bucket: &mut usize) {
    if total == 0 {
        return;
    }

    let progress_bucket = done.saturating_mul(10) / total;
    if progress_bucket > *last_progress_bucket {
        *last_progress_bucket = progress_bucket;
        eprintln!("{} progress: {}/{}", kind, done, total);
    }
}

fn write_upload_report(
    metrics: &BulkUploadMetrics,
    dataset: &str,
    client_id: u32,
    report_count: Option<usize>,
    manager_addr: &str,
    ads_mode: AdsMode,
    set_proof_mode: SetProofMode,
) -> Result<(), Box<dyn std::error::Error>> {
    let throughput = if metrics.total_duration.as_secs_f64() > 0.0 {
        metrics.total_records as f64 / metrics.total_duration.as_secs_f64()
    } else {
        0.0
    };
    let avg_insert_latency_ms = if metrics.total_records > 0 {
        metrics.total_insert_latency.as_secs_f64() * 1000.0 / metrics.total_records as f64
    } else {
        0.0
    };
    let avg_proof_verification_latency_ms = if metrics.total_records > 0 {
        metrics.total_proof_verification_latency.as_secs_f64() * 1000.0
            / metrics.total_records as f64
    } else {
        0.0
    };
    let report = format!(
        "mode=upload\ndataset={}\nclient_id={}\nmanager_addr={}\nads_mode={:?}\nset_proof_mode={}\nrecords={}\nkeyword_pairs={}\ntotal_duration_ms={:.3}\nthroughput_records_per_sec={:.3}\naverage_insert_latency_ms={:.3}\naverage_proof_verification_latency_ms={:.3}\n",
        dataset,
        client_id,
        manager_addr,
        ads_mode,
        set_proof_mode,
        metrics.total_records,
        metrics.total_keyword_pairs,
        metrics.total_duration.as_secs_f64() * 1000.0,
        throughput,
        avg_insert_latency_ms,
        avg_proof_verification_latency_ms,
    );
    let file_count = report_count.unwrap_or(metrics.total_keyword_pairs);
    let path = metrics_output::write_scoped_report_file(
        &["clients", ads_mode.as_str()],
        &format!("{}-{}-upload-{}.txt", dataset, client_id, file_count),
        &report,
    )?;
    println!("upload metrics written: {}", path.display());
    Ok(())
}

fn write_query_report(
    metrics: &BulkQueryMetrics,
    dataset: &str,
    client_id: u32,
    report_count: Option<usize>,
    manager_addr: &str,
    ads_mode: AdsMode,
    set_proof_mode: SetProofMode,
) -> Result<(), Box<dyn std::error::Error>> {
    let throughput = if metrics.total_duration.as_secs_f64() > 0.0 {
        metrics.total_queries as f64 / metrics.total_duration.as_secs_f64()
    } else {
        0.0
    };
    let avg_query_latency_ms = if metrics.total_queries > 0 {
        metrics.total_query_latency.as_secs_f64() * 1000.0 / metrics.total_queries as f64
    } else {
        0.0
    };
    let avg_proof_size_bytes = if metrics.total_queries > 0 {
        metrics.total_proof_size_bytes as f64 / metrics.total_queries as f64
    } else {
        0.0
    };
    let avg_query_keyword_count = if metrics.total_queries > 0 {
        metrics.total_query_keyword_count as f64 / metrics.total_queries as f64
    } else {
        0.0
    };
    let avg_proof_verification_latency_ms = if metrics.total_queries > 0 {
        metrics.total_proof_verification_latency.as_secs_f64() * 1000.0
            / metrics.total_queries as f64
    } else {
        0.0
    };
    let avg_manager_proof_aggregation_latency_ms = if metrics.total_queries > 0 {
        metrics.total_manager_proof_aggregation_latency.as_secs_f64() * 1000.0
            / metrics.total_queries as f64
    } else {
        0.0
    };
    let avg_manager_set_operation_proof_generation_latency_ms = if metrics.total_queries > 0 {
        metrics.total_manager_set_operation_proof_generation_latency.as_secs_f64() * 1000.0
            / metrics.total_queries as f64
    } else {
        0.0
    };
    let report = format!(
        "mode=query\ndataset={}\nclient_id={}\nmanager_addr={}\nads_mode={:?}\nset_proof_mode={}\nqueries={}\ntotal_duration_ms={:.3}\nthroughput_queries_per_sec={:.3}\naverage_query_latency_ms={:.3}\naverage_proof_size_bytes={:.3}\naverage_query_keyword_count={:.3} keywords\naverage_proof_verification_latency_ms={:.3}\naverage_manager_proof_aggregation_latency_ms={:.3}\naverage_manager_set_operation_proof_generation_latency_ms={:.3}\n",
        dataset,
        client_id,
        manager_addr,
        ads_mode,
        set_proof_mode,
        metrics.total_queries,
        metrics.total_duration.as_secs_f64() * 1000.0,
        throughput,
        avg_query_latency_ms,
        avg_proof_size_bytes,
        avg_query_keyword_count,
        avg_proof_verification_latency_ms,
        avg_manager_proof_aggregation_latency_ms,
        avg_manager_set_operation_proof_generation_latency_ms,
    );
    let file_count = report_count.unwrap_or(metrics.total_queries);
    let path = metrics_output::write_scoped_report_file(
        &["clients", ads_mode.as_str()],
        &format!("{}-{}-query-{}.txt", dataset, client_id, file_count),
        &report,
    )?;
    println!("query metrics written: {}", path.display());
    Ok(())
}

fn write_update_report(
    metrics: &BulkUpdateMetrics,
    dataset: &str,
    client_id: u32,
    report_count: Option<usize>,
    manager_addr: &str,
    ads_mode: AdsMode,
    set_proof_mode: SetProofMode,
) -> Result<(), Box<dyn std::error::Error>> {
    let throughput = if metrics.total_duration.as_secs_f64() > 0.0 {
        metrics.total_updates as f64 / metrics.total_duration.as_secs_f64()
    } else {
        0.0
    };
    let avg_update_latency_ms = if metrics.total_updates > 0 {
        metrics.total_update_latency.as_secs_f64() * 1000.0 / metrics.total_updates as f64
    } else {
        0.0
    };
    let avg_proof_verification_latency_ms = if metrics.total_updates > 0 {
        metrics.total_proof_verification_latency.as_secs_f64() * 1000.0
            / metrics.total_updates as f64
    } else {
        0.0
    };
    let report = format!(
        "mode=update\ndataset={}\nclient_id={}\nmanager_addr={}\nads_mode={:?}\nset_proof_mode={}\nupdates={}\nkeyword_pairs={}\ntotal_duration_ms={:.3}\nthroughput_updates_per_sec={:.3}\naverage_update_latency_ms={:.3}\naverage_proof_verification_latency_ms={:.3}\n",
        dataset,
        client_id,
        manager_addr,
        ads_mode,
        set_proof_mode,
        metrics.total_updates,
        metrics.total_keyword_pairs,
        metrics.total_duration.as_secs_f64() * 1000.0,
        throughput,
        avg_update_latency_ms,
        avg_proof_verification_latency_ms,
    );
    let file_count = report_count.unwrap_or(metrics.total_keyword_pairs);
    let path = metrics_output::write_scoped_report_file(
        &["clients", ads_mode.as_str()],
        &format!("{}-{}-update-{}.txt", dataset, client_id, file_count),
        &report,
    )?;
    println!("update metrics written: {}", path.display());
    Ok(())
}

fn load_records(path: &Path) -> Result<Vec<InputRecord>, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut records = Vec::new();

    for (line_no, raw_line) in content.lines().enumerate() {
        let line = raw_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let parts: Vec<String> = line
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();

        if parts.len() < 2 {
            eprintln!(
                "Skipping invalid record at {}:{}; expected fid plus at least 1 keyword",
                path.display(),
                line_no + 1
            );
            continue;
        }

        let fid = parts[0].clone();
        let keywords = parts[1..].to_vec();

        records.push(InputRecord { fid, keywords });
    }

    if records.is_empty() {
        return Err(format!("No valid records found in {}", path.display()).into());
    }

    Ok(records)
}

fn load_query_workload(path: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let queries: Vec<String> = content
        .lines()
        .map(|line| line.trim_start_matches('\u{feff}').trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ToString::to_string)
        .collect();

    if queries.is_empty() {
        return Err(format!("No valid query expressions found in {}", path.display()).into());
    }

    Ok(queries)
}

fn load_update_workload(path: &Path) -> Result<Vec<UpdateRecord>, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut updates = Vec::new();

    for (line_no, raw_line) in content.lines().enumerate() {
        let line = raw_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (parts, format_name): (Vec<String>, &str) = if line.contains('|') {
            (
                line.split('|')
                    .map(|part| part.trim().to_string())
                    .collect(),
                "fid|old_keywords|new_keywords",
            )
        } else {
            (
                line.split(',')
                    .map(|part| part.trim().to_string())
                    .collect(),
                "fid,old_keyword,new_keyword",
            )
        };

        if parts.len() != 3 {
            return Err(format!(
                "Invalid update at {}:{}; expected {}",
                path.display(),
                line_no + 1,
                format_name
            )
            .into());
        }

        let fid = parts[0].clone();
        let old_keywords: Vec<String> = parts[1]
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();
        let new_keywords: Vec<String> = parts[2]
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect();

        if old_keywords.is_empty() || new_keywords.is_empty() {
            return Err(format!(
                "Invalid update at {}:{}; old and new keyword sets must be non-empty",
                path.display(),
                line_no + 1
            )
            .into());
        }

        updates.push(UpdateRecord {
            fid,
            old_keywords,
            new_keywords,
        });
    }

    if updates.is_empty() {
        return Err(format!("No valid updates found in {}", path.display()).into());
    }

    Ok(updates)
}

fn print_help() {
    println!("Client - Distributed Storage System");
    println!();
    println!("USAGE:");
    println!("    client [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!(
        "    -m, --manager-addr <ADDR>      Set manager address (default: scripts/data/manageraddrs)"
    );
    println!(
        "    -a, --ads-mode <MODE>          Set ADS mode: mpt|mest|acctrie|acctree (default: acctrie)"
    );
    println!("    -c, --client-id <ID>           Set client id used in output file names (default: 1)");
    println!("        --report-count <N>        Set the trailing count used in output file names");
    println!("        --set-proof-mode <MODE>    Set boolean set proof mode: polynomial|accumulator (default: accumulator)");
    println!("    -i, --input-dir <DIR>          Set base input data directory (default: crates/client/data)");
    println!("        --records-file <FILE>      Set records file path (default: <input-dir>/records.csv)");
    println!("        --query-file <FILE>        Set query workload file path (default: <input-dir>/query_workload.txt)");
    println!("        --update-file <FILE>       Set update workload file path (default: <input-dir>/update_workload.txt)");
    println!("        --mode <MODE>              Set operation mode: upload|upload-sequential|query|update|upload-and-query|reset (default: upload-and-query)");
    println!("    -h, --help                     Print this help message");
    println!();
    println!("RECORDS FORMAT:");
    println!("    Each line: fid,keyword1,keyword2,...");
    println!("    Each record must contain at least 1 keyword.");
    println!();
    println!("UPDATE FORMAT:");
    println!("    Each line: fid,old_keyword,new_keyword");
    println!("    Also accepted: fid|old_keyword1,old_keyword2|new_keyword1,new_keyword2");
    println!();
    println!("QUERY FORMAT:");
    println!("    Each line is a boolean query expression using AND / OR.");
    println!();
    println!("EXAMPLES:");
    println!("    client");
    println!("    client --mode upload");
    println!("    client --mode upload-sequential");
    println!("    client --mode query");
    println!("    client --mode update --update-file crates/client/data/update_workload.txt");
    println!("    client --manager-addr http://127.0.0.1:50051 --ads-mode mpt --input-dir crates/client/data");
    println!("    client --mode reset");
}
