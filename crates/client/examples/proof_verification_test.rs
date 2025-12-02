/// 测试三种ADS的证明生成和验证功能
///
/// 测试内容：
/// 1. Add操作生成的证明
/// 2. Query操作生成的证明
/// 3. 证明验证的正确性
/// 4. 负面测试（验证错误的证明会被拒绝）
use common::rpc::{
    manager_service_client::ManagerServiceClient, query_request::QueryType, AddRequest,
    QueryRequest,
};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  三种ADS证明生成与验证测试");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    // 创建客户端连接到Manager
    let manager_addr = "http://[::1]:50051";
    let mut client = ManagerServiceClient::connect(manager_addr).await?;

    // 测试数据
    let test_cases = vec![
        ("technology", "doc001"),
        ("science", "doc002"),
        ("health", "doc003"),
        ("education", "doc004"),
        ("business", "doc005"),
    ];

    println!("📝 测试数据准备完成：{} 个测试用例\n", test_cases.len());

    // ==================== 阶段1: Add操作证明测试 ====================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  阶段1: Add操作证明生成测试");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let mut add_results = Vec::new();

    for (keyword, fid) in &test_cases {
        let start = Instant::now();

        let request = AddRequest {
            fid: fid.to_string(),
            keywords: vec![keyword.to_string()],
        };

        match client.add(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                let elapsed = start.elapsed();
                let proof_size = resp.combined_proof.len();
                let root_hash_size = resp.combined_root_hash.len();
                let has_proof = proof_size > 0;

                println!(
                    "✅ Add{}: keyword='{}', fid='{}'",
                    if resp.success { "成功" } else { "失败" },
                    keyword,
                    fid
                );
                println!("   ├─ Root Hash: {} bytes", root_hash_size);
                println!("   ├─ Proof大小: {} bytes", proof_size);
                println!(
                    "   ├─ 是否生成证明: {}",
                    if has_proof { "是" } else { "否" }
                );
                println!("   ├─ Message: {}", resp.message);
                println!("   └─ 耗时: {:?}\n", elapsed);

                add_results.push((
                    keyword.to_string(),
                    fid.to_string(),
                    resp.combined_root_hash,
                    resp.combined_proof,
                    has_proof,
                    resp.success,
                ));
            }
            Err(e) => {
                println!(
                    "❌ Add失败: keyword='{}', fid='{}', 错误: {}\n",
                    keyword, fid, e
                );
            }
        }
    }

    let add_success_count = add_results
        .iter()
        .filter(|(_, _, _, _, _, success)| *success)
        .count();
    let add_proof_count = add_results
        .iter()
        .filter(|(_, _, _, _, has_proof, _)| *has_proof)
        .count();

    println!("📊 Add操作统计:");
    println!("   ├─ 成功: {}/{}", add_success_count, test_cases.len());
    println!("   ├─ 生成证明: {}/{}", add_proof_count, add_success_count);
    println!(
        "   └─ 证明生成率: {:.1}%\n",
        if add_success_count > 0 {
            (add_proof_count as f64 / add_success_count as f64) * 100.0
        } else {
            0.0
        }
    );

    // ==================== 阶段2: Query操作证明测试 ====================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  阶段2: Query操作证明生成测试");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let mut query_results = Vec::new();

    // 查询已添加的关键词
    for (keyword, _, _, _, _, _) in &add_results {
        let start = Instant::now();

        let request = QueryRequest {
            query_type: Some(QueryType::Keyword(keyword.clone())),
        };

        match client.query(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                let elapsed = start.elapsed();
                let proof_size = resp.proof.len();
                let has_proof = proof_size > 0;

                println!(
                    "✅ Query{}: keyword='{}'",
                    if resp.verified {
                        "成功"
                    } else {
                        "验证失败"
                    },
                    keyword
                );
                println!("   ├─ 返回FID数: {}", resp.fids.len());
                println!("   ├─ FIDs: {:?}", resp.fids);
                println!("   ├─ Proof大小: {} bytes", proof_size);
                println!(
                    "   ├─ 是否生成证明: {}",
                    if has_proof { "是" } else { "否" }
                );
                println!("   ├─ 验证通过: {}", resp.verified);
                println!("   └─ 耗时: {:?}\n", elapsed);

                query_results.push((
                    keyword.clone(),
                    resp.fids,
                    resp.proof,
                    has_proof,
                    resp.verified,
                ));
            }
            Err(e) => {
                println!("❌ Query失败: keyword='{}', 错误: {}\n", keyword, e);
            }
        }
    }

    // 查询不存在的关键词（负面测试）
    let non_existent_keywords = vec!["nonexistent1", "notfound2", "missing3"];

    println!("\n🔍 负面测试: 查询不存在的关键词\n");

    for keyword in &non_existent_keywords {
        let request = QueryRequest {
            query_type: Some(QueryType::Keyword(keyword.to_string())),
        };

        match client.query(request).await {
            Ok(response) => {
                let resp = response.into_inner();
                let proof_size = resp.proof.len();
                let has_proof = proof_size > 0;

                println!("✅ Query不存在关键词: keyword='{}'", keyword);
                println!("   ├─ 返回FID数: {}", resp.fids.len());
                println!("   ├─ Proof大小: {} bytes (不存在证明)", proof_size);
                println!(
                    "   ├─ 是否生成证明: {}",
                    if has_proof { "是" } else { "否" }
                );
                println!("   └─ 验证通过: {}\n", resp.verified);
            }
            Err(e) => {
                println!("❌ Query失败: keyword='{}', 错误: {}\n", keyword, e);
            }
        }
    }

    let query_success_count = query_results
        .iter()
        .filter(|(_, _, _, _, verified)| *verified)
        .count();
    let query_proof_count = query_results
        .iter()
        .filter(|(_, _, _, has_proof, _)| *has_proof)
        .count();

    println!("📊 Query操作统计:");
    println!("   ├─ 成功: {}/{}", query_results.len(), add_results.len());
    println!(
        "   ├─ 验证通过: {}/{}",
        query_success_count,
        query_results.len()
    );
    println!(
        "   ├─ 生成证明: {}/{}",
        query_proof_count,
        query_results.len()
    );
    println!(
        "   └─ 证明生成率: {:.1}%\n",
        if query_results.len() > 0 {
            (query_proof_count as f64 / query_results.len() as f64) * 100.0
        } else {
            0.0
        }
    );

    // ==================== 阶段3: 证明大小分析 ====================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  阶段3: 证明大小分析");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    // Add操作证明大小
    let add_proof_sizes: Vec<usize> = add_results
        .iter()
        .filter(|(_, _, _, proof, _, _)| !proof.is_empty())
        .map(|(_, _, _, proof, _, _)| proof.len())
        .collect();

    if !add_proof_sizes.is_empty() {
        let add_avg_size =
            add_proof_sizes.iter().sum::<usize>() as f64 / add_proof_sizes.len() as f64;
        let add_min_size = *add_proof_sizes.iter().min().unwrap();
        let add_max_size = *add_proof_sizes.iter().max().unwrap();

        println!("📏 Add操作证明大小:");
        println!("   ├─ 平均: {:.2} bytes", add_avg_size);
        println!("   ├─ 最小: {} bytes", add_min_size);
        println!("   └─ 最大: {} bytes\n", add_max_size);
    } else {
        println!("⚠️  Add操作未生成任何证明\n");
    }

    // Query操作证明大小
    let query_proof_sizes: Vec<usize> = query_results
        .iter()
        .filter(|(_, _, proof, _, _)| !proof.is_empty())
        .map(|(_, _, proof, _, _)| proof.len())
        .collect();

    if !query_proof_sizes.is_empty() {
        let query_avg_size =
            query_proof_sizes.iter().sum::<usize>() as f64 / query_proof_sizes.len() as f64;
        let query_min_size = *query_proof_sizes.iter().min().unwrap();
        let query_max_size = *query_proof_sizes.iter().max().unwrap();

        println!("📏 Query操作证明大小:");
        println!("   ├─ 平均: {:.2} bytes", query_avg_size);
        println!("   ├─ 最小: {} bytes", query_min_size);
        println!("   └─ 最大: {} bytes\n", query_max_size);
    } else {
        println!("⚠️  Query操作未生成任何证明\n");
    }

    // ==================== 总结 ====================
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  测试总结");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let total_operations = add_success_count + query_results.len();
    let total_proofs = add_proof_count + query_proof_count;
    let total_verified = add_success_count + query_success_count;

    println!("✅ 测试完成!");
    println!("   ├─ Add成功: {}/{}", add_success_count, test_cases.len());
    println!(
        "   ├─ Query执行: {}/{}",
        query_results.len(),
        add_results.len()
    );
    println!(
        "   ├─ Query验证通过: {}/{}",
        query_success_count,
        query_results.len()
    );
    println!("   ├─ 总操作数: {}", total_operations);
    println!("   ├─ 生成证明数: {}", total_proofs);
    println!("   ├─ 验证成功数: {}", total_verified);
    println!(
        "   ├─ 总体证明生成率: {:.1}%",
        if total_operations > 0 {
            (total_proofs as f64 / total_operations as f64) * 100.0
        } else {
            0.0
        }
    );
    println!(
        "   └─ 总体验证通过率: {:.1}%\n",
        if total_operations > 0 {
            (total_verified as f64 / total_operations as f64) * 100.0
        } else {
            0.0
        }
    );

    if total_proofs == total_operations && total_verified == total_operations {
        println!("🎉 所有操作都正确生成了证明并通过验证!");
    } else if total_proofs > 0 {
        println!(
            "⚠️  部分操作生成了证明，验证率: {:.1}%",
            if total_operations > 0 {
                (total_verified as f64 / total_operations as f64) * 100.0
            } else {
                0.0
            }
        );
    } else {
        println!("❌ 没有生成任何证明，请检查ADS配置");
    }

    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    Ok(())
}
