use serde::{Deserialize, Serialize};
use std::fmt;

// KVPair 结构体定义
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct KVPair {
    pub key: String,
    pub value: String,
}

impl KVPair {
    // 创建一个新的 KVPair 实例
    pub fn new(key: String, value: String) -> Self {
        KVPair { key, value }
    }

    // 倒置 KV
    pub fn reverse(&self) -> Self {
        KVPair {
            key: self.value.clone(),
            value: self.key.clone(),
        }
    }

    // 添加值，如果值改变则返回 true
    pub fn add_value(&mut self, new_value: String) -> bool {
        if self.value.is_empty() {
            self.value = new_value;
            true
        } else {
            let values = self
                .value
                .split(',')
                .map(|s| s.trim())
                .collect::<Vec<&str>>();
            if !values.contains(&new_value.as_str()) {
                self.value = format!("{},{}", self.value, new_value);
                true
            } else {
                false
            }
        }
    }

    // 删除值，如果删除成功则返回 true
    pub fn del_value(&mut self, to_del: &str) -> bool {
        if self.value.is_empty() {
            return false;
        }

        let values = self
            .value
            .split(',')
            .map(|s| s.trim())
            .collect::<Vec<&str>>();
        let original_len = values.len();
        let new_values: Vec<&str> = values
            .into_iter()
            .filter(|&value| value != to_del)
            .collect();

        if new_values.len() < original_len {
            self.value = new_values.join(",");
            true
        } else {
            false
        }
    }

    // 获取键
    pub fn get_key(&self) -> &str {
        &self.key
    }

    // 获取值
    pub fn get_value(&self) -> &str {
        &self.value
    }

    // 设置值
    pub fn set_value(&mut self, value: String) {
        self.value = value;
    }

    // 设置键
    pub fn set_key(&mut self, key: String) {
        self.key = key;
    }

    // 如果需要字符串表示，请使用 `to_string()`（由 `Display` trait 提供）或 `format!` 宏

    // 判断是否相等
    pub fn equals(&self, other: &KVPair) -> bool {
        self.key == other.key && self.value == other.value
    }

    // 判断是否小于另一个 KVPair
    pub fn less_than(&self, other: &KVPair) -> bool {
        self.key < other.key
    }

    // 判断是否大于另一个 KVPair
    pub fn greater_than(&self, other: &KVPair) -> bool {
        self.key > other.key
    }

    // 打印 KVPair
    pub fn print_kv_pair(&self) {
        println!("key={}, value={}", self.key, self.value);
    }
}

// 实现 fmt::Display trait 以支持打印
impl fmt::Display for KVPair {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.key, self.value)
    }
}
