use super::error::MPTError;
use super::node::Database;

/// 内存数据库（用于测试）
pub struct MemoryDatabase {
    data: std::collections::HashMap<Vec<u8>, Vec<u8>>,
}

impl MemoryDatabase {
    pub fn new() -> Self {
        Self {
            data: std::collections::HashMap::new(),
        }
    }
}

impl Default for MemoryDatabase {
    fn default() -> Self {
        Self::new()
    }
}

impl Database for MemoryDatabase {
    fn get(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, MPTError> {
        Ok(self.data.get(key).cloned())
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), MPTError> {
        self.data.insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), MPTError> {
        self.data.remove(key);
        Ok(())
    }
}
