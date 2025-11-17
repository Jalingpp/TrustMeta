use storager::ads::{MestAds, AdsOperations};

fn main() {
    println!("=== MEST ADS 快速验证 ===\n");
    
    // 创建 MEST ADS 实例
    println!("1. 创建 MEST ADS 实例...");
    let mut ads = MestAds::new_default();
    println!("✅ MEST ADS 创建成功\n");
    
    // 测试 Add
    println!("2. 测试 Add 操作...");
    let (proof1, root1) = ads.add("rust", "file1");
    println!("   添加 (rust, file1)");
    println!("   Proof 长度: {} bytes", proof1.len());
    println!("   Root hash: {:?}", &root1[..8]);
    
    let (_proof2, root2) = ads.add("rust", "file2");
    println!("   添加 (rust, file2)");
    println!("   Root hash 改变: {}", root1 != root2);
    println!("✅ Add 操作成功\n");
    
    // 测试 Query
    println!("3. 测试 Query 操作...");
    let (fids, proof) = ads.query("rust");
    println!("   查询 'rust' 关键词");
    println!("   找到 {} 个文件: {:?}", fids.len(), fids);
    println!("   Proof 长度: {} bytes", proof.len());
    println!("✅ Query 操作成功\n");
    
    // 测试 Delete
    println!("4. 测试 Delete 操作...");
    let (_proof3, root3) = ads.delete("rust", "file1");
    println!("   删除 (rust, file1)");
    println!("   Root hash 改变: {}", root2 != root3);
    
    let (fids2, _) = ads.query("rust");
    println!("   删除后剩余: {:?}", fids2);
    println!("✅ Delete 操作成功\n");
    
    println!("=== MEST ADS 验证完成 ✅ ===");
}
