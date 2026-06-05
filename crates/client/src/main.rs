use client::client::{Client, QueryKeywordMetrics, RunMetadata};
use common::{
    config::load_manager_http_addr_from_file, init_accumulator_public_parameters, metrics_output,
    parse_boolean_expr, AdsMode, SetProofMode,
};
use std::collections::HashSet;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const DEFAULT_INPUT_DIR: &str = "crates/client/data";
const DEFAULT_RECORDS_FILE: &str = "records.csv";
const DEFAULT_QUERY_FILE: &str = "query_workload.txt";
const DEFAULT_UPDATE_FILE: &str = "update_workload.txt";
const DEFAULT_UPLOAD_BATCH_SIZE: usize = 512;
const FIXED_UPLOAD_BATCH_SIZE_FOR_MPT: usize = 3;
const FIXED_UPLOAD_BATCH_SIZE_FOR_MEST: usize = 1;
const DEFAULT_UPDATE_TASK_TIMEOUT_SECS: u64 = 900;

#[derive(Clone)]
struct InputRecord {
    fid: String,
    keywords: Vec<String>,
}

#[derive(Clone)]
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
    route_mode: String,
    persistence_mode: String,
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
    route_mode: String,
    persistence_mode: String,
}

struct BulkUpdateMetrics {
    total_updates: usize,
    total_keyword_pairs: usize,
    total_duration: Duration,
    total_update_latency: Duration,
    total_proof_verification_latency: Duration,
    route_mode: String,
    persistence_mode: String,
}

enum OperationMode {
    Upload,
    UploadSequential,
    Query,
    Update,
    UploadAndQuery,
    Reset,
}

fn effective_upload_batch_size(ads_mode: AdsMode, configured_batch_size: usize) -> usize {
    match ads_mode {
        AdsMode::Mpt => FIXED_UPLOAD_BATCH_SIZE_FOR_MPT,
        AdsMode::Mest => FIXED_UPLOAD_BATCH_SIZE_FOR_MEST,
        AdsMode::AccTrie | AdsMode::AccTree => configured_batch_size.max(1),
    }
}

fn update_task_timeout() -> Duration {
    std::env::var("CLIENT_UPDATE_TASK_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_UPDATE_TASK_TIMEOUT_SECS))
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
    let mut concurrency: usize = 1;
    let mut upload_batch_size: usize = std::env::var("CLIENT_UPLOAD_BATCH_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_UPLOAD_BATCH_SIZE);
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
            "--concurrency" => {
                if i + 1 < args.len() {
                    concurrency = args[i + 1]
                        .parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                        .unwrap_or_else(|| {
                            eprintln!("Invalid concurrency: {}, using default (1)", args[i + 1]);
                            1
                        });
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--upload-batch-size" => {
                if i + 1 < args.len() {
                    upload_batch_size = args[i + 1]
                        .parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                        .unwrap_or_else(|| {
                            eprintln!(
                                "Invalid upload batch size: {}, using default ({})",
                                args[i + 1],
                                DEFAULT_UPLOAD_BATCH_SIZE
                            );
                            DEFAULT_UPLOAD_BATCH_SIZE
                        });
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

    let needs_records = matches!(
        operation_mode,
        OperationMode::Upload | OperationMode::UploadSequential | OperationMode::UploadAndQuery
    );
    let needs_queries = matches!(
        operation_mode,
        OperationMode::Query | OperationMode::UploadAndQuery
    );
    let needs_updates = matches!(operation_mode, OperationMode::Update);

    let records = if needs_records {
        load_records(&records_file)?
    } else {
        Vec::new()
    };
    let queries = if needs_queries {
        load_query_workload(&query_file)?
    } else {
        Vec::new()
    };
    let updates = if needs_updates {
        load_update_workload(&update_file)?
    } else {
        Vec::new()
    };
    let run_metadata = RunMetadata::new(
        dataset_label.clone(),
        concurrency as u32,
        records.len() as u32,
        queries.len() as u32,
        updates.len() as u32,
    );
    let upload_batch_size = effective_upload_batch_size(ads_mode, upload_batch_size);

    println!("Client connecting to: {}", manager_addr);
    println!("Client dataset: {}", dataset_label);
    println!("Client verification ADS mode: {:?}", ads_mode);
    println!("Client boolean set proof mode: {}", set_proof_mode);
    println!("Client concurrency: {}", concurrency);
    println!("Client upload batch size: {}", upload_batch_size);
    println!("Operation mode: {}", describe_mode(&operation_mode));

    let manager_addr_report = manager_addr.clone();
    let client = Arc::new(Client::new(manager_addr, ads_mode, set_proof_mode));

    if matches!(
        operation_mode,
        OperationMode::Upload | OperationMode::UploadSequential | OperationMode::UploadAndQuery
    ) {
        println!("Records file: {}", records_file.display());
        println!("Loaded {} input records", records.len());
        let encode_keywords_as_hex = true;
        let use_batch_add = matches!(
            operation_mode,
            OperationMode::Upload | OperationMode::UploadAndQuery
        ) && !matches!(ads_mode, AdsMode::Mpt);
        let metrics = run_bulk_put(
            Arc::clone(&client),
            &records,
            concurrency,
            encode_keywords_as_hex,
            use_batch_add,
            upload_batch_size,
            &run_metadata,
        )
        .await?;
        write_upload_report(
            &metrics,
            &dataset_label,
            client_id,
            report_count,
            &manager_addr_report,
            ads_mode,
            set_proof_mode,
            concurrency,
        )?;
    }

    if matches!(operation_mode, OperationMode::Update) {
        println!("Update workload file: {}", update_file.display());
        println!("Loaded {} update records", updates.len());
        let metrics =
            run_bulk_updates(Arc::clone(&client), &updates, concurrency, &run_metadata).await?;
        write_update_report(
            &metrics,
            &dataset_label,
            client_id,
            report_count,
            &manager_addr_report,
            ads_mode,
            set_proof_mode,
            concurrency,
        )?;
    }

    if matches!(
        operation_mode,
        OperationMode::Query | OperationMode::UploadAndQuery
    ) {
        println!("Query workload file: {}", query_file.display());
        println!("Loaded {} query expressions", queries.len());
        let metrics =
            run_bulk_queries(Arc::clone(&client), &queries, concurrency, &run_metadata).await?;
        write_query_report(
            &metrics,
            &dataset_label,
            client_id,
            report_count,
            &manager_addr_report,
            ads_mode,
            set_proof_mode,
            concurrency,
        )?;
    }

    if matches!(operation_mode, OperationMode::Reset) {
        client.reset_system().await?;
    }

    Ok(())
}

#[allow(dead_code)]
async fn run_bulk_put(
    client: Arc<Client>,
    records: &[InputRecord],
    concurrency: usize,
    encode_keywords_as_hex: bool,
    use_batch_add: bool,
    upload_batch_size: usize,
    metadata: &RunMetadata,
) -> Result<BulkUploadMetrics, Box<dyn std::error::Error>> {
    let metadata = metadata.clone();
    let prepared_records: Vec<InputRecord> = if encode_keywords_as_hex {
        records
            .iter()
            .map(|record| InputRecord {
                fid: record.fid.clone(),
                keywords: client.encode_upload_keywords(record.keywords.clone()),
            })
            .collect()
    } else {
        records.to_vec()
    };

    let total_start = Instant::now();
    let mut total_insert_latency = Duration::from_secs(0);
    let mut total_proof_verification_latency = Duration::from_secs(0);
    let mut route_mode = String::new();
    let mut persistence_mode = String::new();
    let mut last_progress_bucket = 0usize;
    let total_keyword_pairs = prepared_records
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
    if use_batch_add {
        let batch_size = upload_batch_size.max(1);
        let prepared_batches: Vec<Vec<InputRecord>> = prepared_records
            .chunks(batch_size)
            .map(|chunk| chunk.to_vec())
            .collect();
        let task_results =
            run_indexed_concurrent_tasks(prepared_batches, concurrency, "upload-batch", {
                let client = Arc::clone(&client);
                let metadata = metadata.clone();
                let total_upload_kv_pairs = total_keyword_pairs as u32;
                move |_idx, batch| {
                    let client = Arc::clone(&client);
                    let metadata = metadata.clone();
                    async move {
                        let batch_record_count = batch.len();
                        let batch_records: Vec<(String, Vec<String>)> = batch
                            .into_iter()
                            .map(|record| (record.fid, record.keywords))
                            .collect();
                        let batch_start = Instant::now();
                        let (route_mode, persistence_mode) = client
                            .batch_put_files(batch_records, total_upload_kv_pairs, &metadata)
                            .await
                            .map_err(|err| format!("batch upload failed: {err}"))?;
                        Ok::<(usize, Duration, String, String), String>((
                            batch_record_count,
                            batch_start.elapsed(),
                            route_mode,
                            persistence_mode,
                        ))
                    }
                }
            })
            .await?;

        let mut uploaded_records = 0usize;
        for (_idx, (batch_record_count, batch_latency, item_route_mode, item_persistence_mode)) in
            task_results
        {
            total_insert_latency += batch_latency.mul_f64(batch_record_count as f64);
            if route_mode.is_empty() {
                route_mode = item_route_mode;
            }
            if persistence_mode.is_empty() {
                persistence_mode = item_persistence_mode;
            }
            uploaded_records += batch_record_count;
            print_progress(
                "upload",
                uploaded_records,
                records.len(),
                &mut last_progress_bucket,
            );
        }
    } else {
        let task_results = run_indexed_concurrent_tasks(prepared_records, concurrency, "upload", {
            let client = Arc::clone(&client);
            let metadata = metadata.clone();
            let total_upload_kv_pairs = total_keyword_pairs as u32;
            move |_idx, record| {
                let client = Arc::clone(&client);
                let metadata = metadata.clone();
                async move {
                    let fid = record.fid;
                    let keywords = record.keywords;
                    let record_summary = format!("fid={}, keywords={:?}", fid, keywords);
                    let record_start = Instant::now();
                    let (verification_latency, route_mode, persistence_mode) = client
                        .put_file_hex(fid, keywords, total_upload_kv_pairs, &metadata)
                        .await
                        .map_err(|err| {
                            format!("record input failed: {record_summary}, err={err}")
                        })?;
                    Ok::<(Duration, Duration, String, String), String>((
                        verification_latency,
                        record_start.elapsed(),
                        route_mode,
                        persistence_mode,
                    ))
                }
            }
        })
        .await?;

        for (idx, (verification_latency, insert_latency, item_route_mode, item_persistence_mode)) in
            task_results
        {
            total_proof_verification_latency += verification_latency;
            total_insert_latency += insert_latency;
            if route_mode.is_empty() {
                route_mode = item_route_mode;
            }
            if persistence_mode.is_empty() {
                persistence_mode = item_persistence_mode;
            }
            print_progress("upload", idx + 1, records.len(), &mut last_progress_bucket);
        }
    }
    Ok(BulkUploadMetrics {
        total_records: records.len(),
        total_keyword_pairs,
        total_duration: total_start.elapsed(),
        total_insert_latency,
        total_proof_verification_latency,
        route_mode,
        persistence_mode,
    })
}

async fn run_bulk_updates(
    client: Arc<Client>,
    updates: &[UpdateRecord],
    concurrency: usize,
    metadata: &RunMetadata,
) -> Result<BulkUpdateMetrics, Box<dyn std::error::Error>> {
    let metadata = metadata.clone();
    let task_timeout = update_task_timeout();
    let prepared_updates: Vec<UpdateRecord> = updates
        .iter()
        .map(|update| UpdateRecord {
            fid: update.fid.clone(),
            old_keywords: client.encode_update_keywords(update.old_keywords.clone()),
            new_keywords: client.encode_update_keywords(update.new_keywords.clone()),
        })
        .collect();

    let total_start = Instant::now();
    let mut total_update_latency = Duration::from_secs(0);
    let mut total_proof_verification_latency = Duration::from_secs(0);
    let mut route_mode = String::new();
    let mut persistence_mode = String::new();
    let mut last_progress_bucket = 0usize;
    println!("=== Bulk Update File ===");
    let total_keyword_pairs = prepared_updates
        .iter()
        .map(|update| {
            update
                .old_keywords
                .iter()
                .chain(update.new_keywords.iter())
                .cloned()
                .collect::<HashSet<_>>()
                .len()
        })
        .sum::<usize>();
    let task_results = run_indexed_concurrent_tasks(prepared_updates, concurrency, "update", {
        let client = Arc::clone(&client);
        let metadata = metadata.clone();
        move |_idx, update| {
            let client = Arc::clone(&client);
            let metadata = metadata.clone();
            async move {
                let fid = update.fid;
                let old_keywords = update.old_keywords;
                let new_keywords = update.new_keywords;
                let update_summary = format!(
                    "fid={}, old_keywords={:?}, new_keywords={:?}",
                    fid, old_keywords, new_keywords
                );
                let record_start = Instant::now();
                let update_result = tokio::time::timeout(
                    task_timeout,
                    client.update_file_hex(fid, old_keywords, new_keywords, &metadata),
                )
                .await;
                let (verification_latency, route_mode, persistence_mode) = match update_result {
                    Ok(Ok(result)) => result,
                    Ok(Err(err)) => {
                        return Err(format!("update input failed: {update_summary}, err={err}"));
                    }
                    Err(_) => {
                        return Err(format!(
                            "update task timed out after {:?}: {update_summary}",
                            task_timeout
                        ));
                    }
                };
                Ok::<(Duration, Duration, String, String), String>((
                    verification_latency,
                    record_start.elapsed(),
                    route_mode,
                    persistence_mode,
                ))
            }
        }
    })
    .await?;

    for (idx, (verification_latency, update_latency, item_route_mode, item_persistence_mode)) in
        task_results
    {
        total_proof_verification_latency += verification_latency;
        total_update_latency += update_latency;
        if route_mode.is_empty() {
            route_mode = item_route_mode;
        }
        if persistence_mode.is_empty() {
            persistence_mode = item_persistence_mode;
        }
        print_progress("update", idx + 1, updates.len(), &mut last_progress_bucket);
    }
    Ok(BulkUpdateMetrics {
        total_updates: updates.len(),
        total_keyword_pairs,
        total_duration: total_start.elapsed(),
        total_update_latency,
        total_proof_verification_latency,
        route_mode,
        persistence_mode,
    })
}

async fn run_bulk_queries(
    client: Arc<Client>,
    queries: &[String],
    concurrency: usize,
    metadata: &RunMetadata,
) -> Result<BulkQueryMetrics, Box<dyn std::error::Error>> {
    let metadata = metadata.clone();
    let mut skipped_queries = 0usize;
    let prepared_queries: Vec<(String, usize)> = queries
        .iter()
        .filter_map(|query| {
            let expr = match parse_boolean_expr(query) {
                Ok(expr) => expr,
                Err(err) => {
                    eprintln!("Skipping invalid query expression {:?}: {}", query, err);
                    skipped_queries += 1;
                    return None;
                }
            };
            let keyword_count = expr.get_keywords().len();
            let encoded = client.encode_boolean_query_expression(&expr);
            Some((encoded, keyword_count))
        })
        .collect();

    if prepared_queries.is_empty() {
        return Err("No valid query expressions found after filtering".into());
    }

    if skipped_queries > 0 {
        println!("Skipped {} invalid query expression(s)", skipped_queries);
    }

    let prepared_query_count = prepared_queries.len();

    let total_start = Instant::now();
    let mut total_proof_size_bytes = 0usize;
    let mut total_query_latency = Duration::from_secs(0);
    let mut total_proof_verification_latency = Duration::from_secs(0);
    let mut total_manager_proof_aggregation_latency = Duration::from_secs(0);
    let mut total_manager_set_operation_proof_generation_latency = Duration::from_secs(0);
    let mut route_mode = String::new();
    let mut persistence_mode = String::new();
    let mut last_progress_bucket = 0usize;
    let task_results = run_indexed_concurrent_tasks(prepared_queries, concurrency, "query", {
        let client = Arc::clone(&client);
        let metadata = metadata.clone();
        move |_idx, (encoded_query, keyword_count)| {
            let client = Arc::clone(&client);
            let metadata = metadata.clone();
            async move {
                let query_summary = format!("query={encoded_query}");
                let query_start = Instant::now();
                let metrics = client
                    .query_by_func_hex(encoded_query, &metadata)
                    .await
                    .map_err(|err| format!("query input failed: {query_summary}, err={err}"))?;
                Ok::<(QueryKeywordMetrics, usize, Duration), String>((
                    metrics,
                    keyword_count,
                    query_start.elapsed(),
                ))
            }
        }
    })
    .await?;

    let total_query_keyword_count = task_results
        .iter()
        .map(|(_, (_, keyword_count, _))| *keyword_count)
        .sum();

    for (idx, (metrics, _keyword_count, query_latency)) in task_results {
        total_proof_size_bytes += metrics.proof_size_bytes;
        total_proof_verification_latency += metrics.verification_latency;
        total_manager_proof_aggregation_latency += metrics.manager_proof_aggregation_latency;
        total_manager_set_operation_proof_generation_latency +=
            metrics.manager_set_operation_proof_generation_latency;
        total_query_latency += query_latency;
        if route_mode.is_empty() {
            route_mode = metrics.route_mode.clone();
        }
        if persistence_mode.is_empty() {
            persistence_mode = metrics.persistence_mode.clone();
        }
        print_progress(
            "query",
            idx + 1,
            prepared_query_count,
            &mut last_progress_bucket,
        );
    }
    Ok(BulkQueryMetrics {
        total_queries: prepared_query_count,
        total_proof_size_bytes,
        total_query_keyword_count,
        total_duration: total_start.elapsed(),
        total_query_latency,
        total_proof_verification_latency,
        total_manager_proof_aggregation_latency,
        total_manager_set_operation_proof_generation_latency,
        route_mode,
        persistence_mode,
    })
}

async fn run_indexed_concurrent_tasks<I, O, F, Fut>(
    items: Vec<I>,
    concurrency: usize,
    kind: &'static str,
    task_fn: F,
) -> Result<Vec<(usize, O)>, Box<dyn std::error::Error>>
where
    I: Send + 'static,
    O: Send + 'static,
    F: Fn(usize, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, String>> + Send + 'static,
{
    let total = items.len();
    if total == 0 {
        return Ok(Vec::new());
    }

    if concurrency <= 1 || total <= 1 {
        let mut results = Vec::with_capacity(total);
        for (idx, item) in items.into_iter().enumerate() {
            let output = task_fn(idx, item)
                .await
                .map_err(|err| format!("{kind} task {idx} failed: {err}"))?;
            results.push((idx, output));
        }
        return Ok(results);
    }

    let semaphore = Arc::new(Semaphore::new(concurrency));
    let task_fn = Arc::new(task_fn);
    let mut join_set = JoinSet::new();

    for (idx, item) in items.into_iter().enumerate() {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| format!("failed to acquire {kind} permit: {err}"))?;
        let task_fn = Arc::clone(&task_fn);
        join_set.spawn(async move {
            let _permit = permit;
            let output = (task_fn)(idx, item)
                .await
                .map_err(|err| format!("{kind} task {idx} failed: {err}"))?;
            Ok::<(usize, O), String>((idx, output))
        });
    }

    let mut results = Vec::with_capacity(total);
    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok(Ok((idx, output))) => {
                results.push((idx, output));
            }
            Ok(Err(err)) => {
                join_set.abort_all();
                while join_set.join_next().await.is_some() {}
                return Err(format!("{kind} task failed: {err}").into());
            }
            Err(err) => {
                join_set.abort_all();
                while join_set.join_next().await.is_some() {}
                return Err(format!("{kind} task join error: {err}").into());
            }
        }
    }

    results.sort_by_key(|(idx, _)| *idx);
    Ok(results)
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
    concurrency: usize,
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
        "mode=upload\ndataset={}\nclient_id={}\nmanager_addr={}\nads_mode={:?}\nset_proof_mode={}\nroute_mode={}\npersistence_mode={}\nconcurrency={}\nrecords={}\nkeyword_pairs={}\ntotal_duration_ms={:.3}\nthroughput_records_per_sec={:.3}\nupload_throughput_per_sec={:.3}\naverage_insert_latency_ms={:.3}\naverage_upload_latency_ms={:.3}\naverage_proof_verification_latency_ms={:.3}\n",
        dataset,
        client_id,
        manager_addr,
        ads_mode,
        set_proof_mode,
        metrics.route_mode,
        metrics.persistence_mode,
        concurrency,
        metrics.total_records,
        metrics.total_keyword_pairs,
        metrics.total_duration.as_secs_f64() * 1000.0,
        throughput,
        throughput,
        avg_insert_latency_ms,
        avg_insert_latency_ms,
        avg_proof_verification_latency_ms,
    );
    let file_count = report_count.unwrap_or(metrics.total_keyword_pairs);
    let path = metrics_output::write_scoped_report_file(
        &["clients", ads_mode.as_str()],
        &format!(
            "{}-{}-{}-{}-upload-{}.txt",
            dataset, concurrency, metrics.route_mode, metrics.persistence_mode, file_count
        ),
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
    concurrency: usize,
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
        metrics
            .total_manager_proof_aggregation_latency
            .as_secs_f64()
            * 1000.0
            / metrics.total_queries as f64
    } else {
        0.0
    };
    let avg_manager_set_operation_proof_generation_latency_ms = if metrics.total_queries > 0 {
        metrics
            .total_manager_set_operation_proof_generation_latency
            .as_secs_f64()
            * 1000.0
            / metrics.total_queries as f64
    } else {
        0.0
    };
    let report = format!(
        "mode=query\ndataset={}\nclient_id={}\nmanager_addr={}\nads_mode={:?}\nset_proof_mode={}\nroute_mode={}\npersistence_mode={}\nconcurrency={}\nqueries={}\ntotal_duration_ms={:.3}\nthroughput_queries_per_sec={:.3}\nquery_throughput_per_sec={:.3}\naverage_query_latency_ms={:.3}\naverage_proof_size_bytes={:.3}\naverage_query_keyword_count={:.3} keywords\naverage_proof_verification_latency_ms={:.3}\naverage_manager_proof_aggregation_latency_ms={:.3}\naverage_manager_set_operation_proof_generation_latency_ms={:.3}\n",
        dataset,
        client_id,
        manager_addr,
        ads_mode,
        set_proof_mode,
        metrics.route_mode,
        metrics.persistence_mode,
        concurrency,
        metrics.total_queries,
        metrics.total_duration.as_secs_f64() * 1000.0,
        throughput,
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
        &format!(
            "{}-{}-{}-{}-query-{}.txt",
            dataset, concurrency, metrics.route_mode, metrics.persistence_mode, file_count
        ),
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
    concurrency: usize,
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
        "mode=update\ndataset={}\nclient_id={}\nmanager_addr={}\nads_mode={:?}\nset_proof_mode={}\nroute_mode={}\npersistence_mode={}\nconcurrency={}\nupdates={}\nkeyword_pairs={}\ntotal_duration_ms={:.3}\nthroughput_updates_per_sec={:.3}\nupdate_throughput_per_sec={:.3}\naverage_update_latency_ms={:.3}\naverage_proof_verification_latency_ms={:.3}\n",
        dataset,
        client_id,
        manager_addr,
        ads_mode,
        set_proof_mode,
        metrics.route_mode,
        metrics.persistence_mode,
        concurrency,
        metrics.total_updates,
        metrics.total_keyword_pairs,
        metrics.total_duration.as_secs_f64() * 1000.0,
        throughput,
        throughput,
        avg_update_latency_ms,
        avg_proof_verification_latency_ms,
    );
    let file_count = report_count.unwrap_or(metrics.total_keyword_pairs);
    let path = metrics_output::write_scoped_report_file(
        &["clients", ads_mode.as_str()],
        &format!(
            "{}-{}-{}-{}-update-{}.txt",
            dataset, concurrency, metrics.route_mode, metrics.persistence_mode, file_count
        ),
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
        .map(|line| line.trim_start_matches('\u{feff}').trim().to_string())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
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

        let parts: Vec<String> = line
            .split(',')
            .map(|part| part.trim().to_string())
            .collect();
        if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
            return Err(format!(
                "Invalid update at {}:{}; expected {}",
                path.display(),
                line_no + 1,
                "fid,old_keyword,new_keyword"
            )
            .into());
        }

        updates.push(UpdateRecord {
            fid: parts[0].clone(),
            old_keywords: vec![parts[1].clone()],
            new_keywords: vec![parts[2].clone()],
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
    println!(
        "    -c, --client-id <ID>           Set client id used in report content (default: 1)"
    );
    println!("        --report-count <N>        Set the trailing count used in output file names");
    println!(
        "        --concurrency <N>         Set max concurrent requests per workload (default: 1)"
    );
    println!(
        "        --upload-batch-size <N>   Set records per batch for upload mode (default: 512 or CLIENT_UPLOAD_BATCH_SIZE)"
    );
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
    println!();
    println!("QUERY FORMAT:");
    println!("    Each line is a boolean query expression using AND / OR.");
    println!();
    println!("EXAMPLES:");
    println!("    client");
    println!("    client --mode upload");
    println!("    client --mode upload --upload-batch-size 1024");
    println!("    client --mode upload-sequential");
    println!("    client --mode query");
    println!("    client --mode update --update-file crates/client/data/update_workload.txt");
    println!("    client --manager-addr http://127.0.0.1:50051 --ads-mode mpt --input-dir crates/client/data");
    println!("    client --mode reset");
}
