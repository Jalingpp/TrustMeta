use super::snapshot::LoadedMestSnapshot;
use super::PersistedBucket;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MestMigrationRecord {
    pub key: String,
    pub value: String,
    pub source_bucket_key: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MestMigrationBatch {
    pub target_prefix: String,
    pub records: Vec<MestMigrationRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MestMigrationPlan {
    pub batches: Vec<MestMigrationBatch>,
}

pub fn build_migration_plan<F>(
    snapshot: &LoadedMestSnapshot,
    mut route: F,
) -> MestMigrationPlan
where
    F: FnMut(&str) -> Option<String>,
{
    let mut batches: BTreeMap<String, Vec<MestMigrationRecord>> = BTreeMap::new();
    for bucket in snapshot.buckets.values() {
        collect_bucket_records(bucket, &mut route, &mut batches);
    }

    MestMigrationPlan {
        batches: batches
            .into_iter()
            .map(|(target_prefix, records)| MestMigrationBatch {
                target_prefix,
                records,
            })
            .collect(),
    }
}

fn collect_bucket_records<F>(
    bucket: &PersistedBucket,
    route: &mut F,
    batches: &mut BTreeMap<String, Vec<MestMigrationRecord>>,
) where
    F: FnMut(&str) -> Option<String>,
{
    for segment in &bucket.segments {
        for kv in &segment.kv_pairs {
            if let Some(target_prefix) = route(&kv.key) {
                batches
                    .entry(target_prefix)
                    .or_default()
                    .push(MestMigrationRecord {
                        key: kv.key.clone(),
                        value: kv.value.clone(),
                        source_bucket_key: bucket.bucket_key.clone(),
                    });
            }
        }
    }
}

impl super::MEHT {
    pub fn apply_migration_batch(&self, batch: &MestMigrationBatch) {
        for record in &batch.records {
            let _ = self.insert(super::KVPair::new(record.key.clone(), record.value.clone()));
        }
        if self.cache.is_some() {
            let _ = self.persist_to_cache();
        }
    }

    pub fn rebuild_from_migration_batch(
        rdx: i32,
        bc: i32,
        bs: i32,
        batch: &MestMigrationBatch,
        cache_limit: usize,
    ) -> std::sync::Arc<std::sync::RwLock<super::MEHT>> {
        let meht = super::MEHT::new_with_cache_limit(rdx, bc, bs, cache_limit);
        {
            let guard = meht.read().unwrap();
            guard.apply_migration_batch(batch);
        }
        meht
    }

    pub fn remove_migrated_batch(&self, batch: &MestMigrationBatch) {
        for record in &batch.records {
            let _ = self.delete(&record.key, &record.value);
        }
        if self.cache.is_some() {
            let _ = self.persist_to_cache();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mest::{load_snapshot, MEHT, KVPair};

    #[test]
    fn test_build_migration_plan_splits_records_by_prefix() {
        let meht = MEHT::new_with_cache_limit(4, 2, 2, 2);
        let guard = meht.read().unwrap();
        let _ = guard.insert(KVPair::new(String::from_utf8(vec![97, 49]).unwrap(), String::from_utf8(vec![118, 49]).unwrap()));
        let _ = guard.insert(KVPair::new(String::from_utf8(vec![98, 49]).unwrap(), String::from_utf8(vec![118, 50]).unwrap()));
        let _ = guard.insert(KVPair::new(String::from_utf8(vec![99, 49]).unwrap(), String::from_utf8(vec![118, 51]).unwrap()));
        let _ = guard.persist_to_cache().unwrap();

        let cache_lock = guard.cache.as_ref().unwrap();
        let mut cache = cache_lock.write().unwrap();
        let snapshot = load_snapshot(&mut cache).unwrap().unwrap();

        let plan = build_migration_plan(&snapshot, |key| {
            if key.starts_with('a') || key.starts_with('b') {
                Some(String::from_utf8(vec![108, 101, 102, 116]).unwrap())
            } else if key.starts_with('c') {
                Some(String::from_utf8(vec![114, 105, 103, 104, 116]).unwrap())
            } else {
                None
            }
        });

        assert_eq!(plan.batches.len(), 2);
        assert_eq!(plan.batches[0].target_prefix, String::from_utf8(vec![108, 101, 102, 116]).unwrap());
        assert_eq!(plan.batches[0].records.len(), 2);
        assert_eq!(plan.batches[1].target_prefix, String::from_utf8(vec![114, 105, 103, 104, 116]).unwrap());
        assert_eq!(plan.batches[1].records.len(), 1);
    }

    #[test]
    fn test_apply_migration_batch_rebuilds_target_meht() {
        let source = MEHT::new_with_cache_limit(4, 2, 2, 2);
        let source_guard = source.read().unwrap();
        let _ = source_guard.insert(KVPair::new(String::from_utf8(vec![97, 49]).unwrap(), String::from_utf8(vec![118, 49]).unwrap()));
        let _ = source_guard.insert(KVPair::new(String::from_utf8(vec![98, 49]).unwrap(), String::from_utf8(vec![118, 50]).unwrap()));
        let _ = source_guard.insert(KVPair::new(String::from_utf8(vec![99, 49]).unwrap(), String::from_utf8(vec![118, 51]).unwrap()));
        let _ = source_guard.persist_to_cache().unwrap();

        let cache_lock = source_guard.cache.as_ref().unwrap();
        let mut cache = cache_lock.write().unwrap();
        let snapshot = load_snapshot(&mut cache).unwrap().unwrap();
        let plan = build_migration_plan(&snapshot, |key| {
            if key.starts_with('c') {
                Some(String::from_utf8(vec![114, 105, 103, 104, 116]).unwrap())
            } else {
                None
            }
        });

        let batch = &plan.batches[0];
        let target = MEHT::rebuild_from_migration_batch(4, 2, 2, batch, 2);
        let target_guard = target.read().unwrap();
        let proof = target_guard.query(&String::from_utf8(vec![99, 49]).unwrap()).unwrap();

        assert_eq!(proof.bucket_proof.value, String::from_utf8(vec![118, 51]).unwrap());
    }

    #[test]
    fn test_migration_closure_moves_data_from_source_to_target() {
        let source = MEHT::new_with_cache_limit(4, 2, 2, 2);
        let source_guard = source.read().unwrap();
        let key = String::from_utf8(vec![99, 49]).unwrap();
        let value = String::from_utf8(vec![118, 51]).unwrap();
        let _ = source_guard.insert(KVPair::new(key.clone(), value.clone()));
        let _ = source_guard.persist_to_cache().unwrap();

        let cache_lock = source_guard.cache.as_ref().unwrap();
        let mut cache = cache_lock.write().unwrap();
        let snapshot = load_snapshot(&mut cache).unwrap().unwrap();
        drop(cache);

        let plan = build_migration_plan(&snapshot, |route_key| {
            if route_key == key {
                Some(String::from_utf8(vec![114, 105, 103, 104, 116]).unwrap())
            } else {
                None
            }
        });

        let batch = &plan.batches[0];
        let target = MEHT::rebuild_from_migration_batch(4, 2, 2, batch, 2);
        source_guard.remove_migrated_batch(batch);

        assert!(source_guard.query(&key).is_none());

        let target_guard = target.read().unwrap();
        let proof = target_guard.query(&key).unwrap();
        assert_eq!(proof.bucket_proof.value, value);
    }
}
