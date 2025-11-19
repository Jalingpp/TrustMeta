#!/usr/bin/env python3
"""
生成 Workload 测试数据集
模拟真实场景的文档和关键词分布
"""

import random
import json
from pathlib import Path

# 数据集规模配置
CONFIG = {
    "small": 1000,      # 小型数据集
    "medium": 10000,    # 中型数据集  
    "large": 100000,    # 大型数据集
}

# 真实场景的类别和关键词
CATEGORIES = {
    "technology": {
        "keywords": ["ai", "ml", "blockchain", "cloud", "iot", "5g", "quantum", "cybersecurity", 
                    "software", "hardware", "programming", "database", "network", "server"],
        "weight": 0.15
    },
    "business": {
        "keywords": ["finance", "marketing", "sales", "investment", "startup", "enterprise",
                    "strategy", "management", "analytics", "revenue", "customer", "market"],
        "weight": 0.12
    },
    "science": {
        "keywords": ["research", "experiment", "biology", "chemistry", "physics", "astronomy",
                    "genetics", "laboratory", "hypothesis", "theory", "data", "analysis"],
        "weight": 0.10
    },
    "education": {
        "keywords": ["learning", "course", "student", "teacher", "university", "study",
                    "exam", "assignment", "lecture", "textbook", "degree", "scholarship"],
        "weight": 0.12
    },
    "health": {
        "keywords": ["medical", "hospital", "doctor", "patient", "treatment", "diagnosis",
                    "medicine", "surgery", "healthcare", "wellness", "nutrition", "fitness"],
        "weight": 0.10
    },
    "entertainment": {
        "keywords": ["movie", "music", "game", "concert", "artist", "celebrity", "show",
                    "streaming", "album", "video", "performance", "comedy", "drama"],
        "weight": 0.08
    },
    "sports": {
        "keywords": ["football", "basketball", "soccer", "tennis", "olympics", "championship",
                    "team", "player", "coach", "tournament", "match", "score", "athlete"],
        "weight": 0.08
    },
    "news": {
        "keywords": ["politics", "election", "government", "policy", "international", "domestic",
                    "report", "breaking", "update", "announcement", "event", "crisis"],
        "weight": 0.10
    },
    "lifestyle": {
        "keywords": ["fashion", "food", "travel", "home", "family", "cooking", "recipe",
                    "design", "decoration", "shopping", "trend", "style", "beauty"],
        "weight": 0.08
    },
    "environment": {
        "keywords": ["climate", "sustainability", "renewable", "pollution", "conservation",
                    "ecology", "green", "carbon", "recycling", "energy", "wildlife"],
        "weight": 0.07
    }
}

# Zipf 分布的关键词流行度
POPULAR_KEYWORDS = [
    "important", "urgent", "review", "update", "new", "latest", "trending",
    "featured", "popular", "recommended", "special", "exclusive", "premium"
]

def generate_fid(index: int, category: str) -> str:
    """生成文件ID"""
    return f"{category[:4]}_{index:08d}"

def select_category() -> str:
    """根据权重选择类别"""
    categories = list(CATEGORIES.keys())
    weights = [CATEGORIES[cat]["weight"] for cat in categories]
    return random.choices(categories, weights=weights)[0]

def generate_keywords(category: str, num_keywords: int = None) -> list:
    """生成关键词集合"""
    if num_keywords is None:
        # Zipf 分布:大部分文档有2-4个关键词,少数有更多
        num_keywords = random.choices([2, 3, 4, 5, 6, 7], 
                                     weights=[30, 35, 20, 10, 3, 2])[0]
    
    keywords = []
    
    # 添加类别关键词
    keywords.append(category)
    
    # 从该类别选择特定关键词
    category_keywords = CATEGORIES[category]["keywords"]
    num_category_kw = min(num_keywords - 1, len(category_keywords))
    keywords.extend(random.sample(category_keywords, num_category_kw))
    
    # 20% 的概率添加热门关键词(模拟热点数据)
    if random.random() < 0.2:
        keywords.append(random.choice(POPULAR_KEYWORDS))
    
    # 确保关键词数量
    while len(keywords) < num_keywords:
        keywords.append(random.choice(category_keywords))
    
    return keywords[:num_keywords]

def generate_dataset(size: int, output_file: str):
    """生成数据集"""
    print(f"生成 {size} 条记录到 {output_file}...")
    
    records = []
    category_distribution = {}
    keyword_distribution = {}
    
    for i in range(size):
        category = select_category()
        fid = generate_fid(i, category)
        keywords = generate_keywords(category)
        
        # 统计分布
        category_distribution[category] = category_distribution.get(category, 0) + 1
        for kw in keywords:
            keyword_distribution[kw] = keyword_distribution.get(kw, 0) + 1
        
        # CSV 格式: fid,keyword1,keyword2,...
        record = f"{fid},{','.join(keywords)}\n"
        records.append(record)
        
        if (i + 1) % 10000 == 0:
            print(f"  进度: {i + 1}/{size}")
    
    # 写入文件
    with open(output_file, 'w') as f:
        f.writelines(records)
    
    # 生成统计信息
    stats = {
        "total_records": size,
        "categories": len(category_distribution),
        "unique_keywords": len(keyword_distribution),
        "category_distribution": category_distribution,
        "top_keywords": sorted(keyword_distribution.items(), 
                              key=lambda x: x[1], reverse=True)[:20]
    }
    
    stats_file = output_file.replace('.csv', '_stats.json')
    with open(stats_file, 'w') as f:
        json.dump(stats, f, indent=2)
    
    print(f"✅ 完成!")
    print(f"  数据文件: {output_file}")
    print(f"  统计文件: {stats_file}")
    print(f"  类别数: {len(category_distribution)}")
    print(f"  唯一关键词数: {len(keyword_distribution)}")
    print(f"  类别分布:")
    for cat, count in sorted(category_distribution.items(), key=lambda x: x[1], reverse=True):
        print(f"    {cat}: {count} ({count/size*100:.1f}%)")

def main():
    """主函数"""
    import argparse
    
    parser = argparse.ArgumentParser(description='生成 Workload 测试数据集')
    parser.add_argument('--size', type=str, default='medium',
                       choices=['small', 'medium', 'large', 'custom'],
                       help='数据集规模 (small=1K, medium=10K, large=100K)')
    parser.add_argument('--custom-size', type=int, default=None,
                       help='自定义数据集大小')
    parser.add_argument('--output', type=str, default=None,
                       help='输出文件路径')
    
    args = parser.parse_args()
    
    # 确定数据集大小
    if args.size == 'custom':
        if args.custom_size is None:
            print("错误: 使用 custom 时必须指定 --custom-size")
            return
        size = args.custom_size
    else:
        size = CONFIG[args.size]
    
    # 确定输出文件
    if args.output is None:
        data_dir = Path(__file__).parent.parent / 'data'
        data_dir.mkdir(exist_ok=True)
        output_file = data_dir / f'workload_{args.size}_{size}.csv'
    else:
        output_file = Path(args.output)
    
    # 生成数据集
    generate_dataset(size, str(output_file))

if __name__ == '__main__':
    main()
