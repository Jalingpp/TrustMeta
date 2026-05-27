use super::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

pub struct LoadedMestSnapshot {
    pub manifest: PersistedMestManifest,
    pub seh_directory: PersistedSehDirectory,
    pub buckets: BTreeMap<MestObjectHash, PersistedBucket>,
    pub mgt_nodes: BTreeMap<MestObjectHash, PersistedMgtNode>,
}

pub fn persist_to_cache(meht: &MEHT) -> Result<PersistedMestManifest, MestStorageError> {
    let cache_lock = meht
        .cache
        .as_ref()
        .ok_or_else(|| MestStorageError::Open(String::new()))?;
    let bucket_hashes = persist_buckets(meht, cache_lock)?;
    let seh_hash = persist_seh_directory(meht, cache_lock, &bucket_hashes)?;
    let mgt_hash = persist_mgt_root(meht, cache_lock)?;
    let mgt_root_hash = meht.mgt.read().unwrap().mgt_root_hash;
    let manifest = PersistedMestManifest {
        version: MEST_STORAGE_FORMAT_VERSION,
        rdx: meht.rdx,
        bucket_capacity: meht.bc,
        bucket_seg_num: meht.bs,
        seh_directory_hash: Some(seh_hash),
        mgt_root_object_hash: mgt_hash,
        mgt_root_hash,
        current_root_hash: mgt_root_hash,
    };
    cache_lock.write().unwrap().persist_manifest(&manifest)?;
    Ok(manifest)
}

pub fn load_snapshot(cache: &mut MestWriteBackCache) -> Result<Option<LoadedMestSnapshot>, MestStorageError> {
    let manifest = match cache.load_manifest()? {
        Some(manifest) => manifest,
        None => return Ok(None),
    };
    let seh_hash = match manifest.seh_directory_hash {
        Some(hash) => hash,
        None => return Ok(None),
    };
    let seh_directory = load_seh_directory(cache, &seh_hash)?;
    let buckets = load_all_buckets(cache, &seh_directory)?;
    let mgt_nodes = match manifest.mgt_root_object_hash {
        Some(root_hash) => load_all_mgt_nodes(cache, &root_hash)?,
        None => BTreeMap::new(),
    };
    Ok(Some(LoadedMestSnapshot {
        manifest,
        seh_directory,
        buckets,
        mgt_nodes,
    }))
}

fn snapshot_bucket(bucket: &Arc<RwLock<Bucket>>) -> Result<PersistedBucket, MestStorageError> {
    let bucket_r = bucket.read().unwrap();
    let segments_map = bucket_r.segments.read().unwrap();
    let mut segment_keys: Vec<String> = segments_map.keys().cloned().collect();
    segment_keys.sort();
    let segments = segment_keys
        .iter()
        .map(|seg_key| PersistedBucketSegment {
            seg_key: seg_key.clone(),
            kv_pairs: segments_map.get(seg_key).cloned().unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    drop(segments_map);

    let merkle_trees = bucket_r.merkle_trees.read().unwrap();
    let mut segment_roots = BTreeMap::new();
    for seg_key in &segment_keys {
        if let Some(root) = merkle_trees.get(seg_key).and_then(|mt| mt.get_root_hash()) {
            segment_roots.insert(seg_key.clone(), root);
        }
    }
    drop(merkle_trees);

    let delegation_list = bucket_r
        .delegation_list
        .read()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.iter().map(|(inner_k, inner_v)| (inner_k.clone(), *inner_v)).collect(),
            )
        })
        .collect::<BTreeMap<String, BTreeMap<String, bool>>>();
    let pending_num = *bucket_r.pending_num.read().unwrap();
    let to_del_map = bucket_r
        .to_del_map
        .read()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.iter().map(|(inner_k, inner_v)| (inner_k.clone(), *inner_v)).collect(),
            )
        })
        .collect::<BTreeMap<String, BTreeMap<String, i32>>>();

    Ok(PersistedBucket {
        version: MEST_STORAGE_FORMAT_VERSION,
        bucket_key: bucket_r.bucket_key.clone(),
        ld: bucket_r.ld,
        rdx: bucket_r.rdx,
        capacity: bucket_r.capacity,
        number: bucket_r.number,
        seg_num: bucket_r.seg_num,
        segments,
        segment_roots,
        latch_timestamp: bucket_r.latch_timestamp,
        delegation_list,
        pending_num,
        to_del_map,
    })
}

fn load_seh_directory(
    cache: &mut MestWriteBackCache,
    hash: &MestObjectHash,
) -> Result<PersistedSehDirectory, MestStorageError> {
    match cache.load_object(hash)? {
        Some(PersistedMestObject::SehDirectory(directory)) => Ok(directory),
        Some(other) => Err(MestStorageError::ObjectTypeMismatch {
            expected: PersistedMestObject::SehDirectory(PersistedSehDirectory {
                version: 0,
                gd: 0,
                rdx: 0,
                bucket_capacity: 0,
                bucket_seg_num: 0,
                buckets_number: 0,
                directory: BTreeMap::new(),
            })
            .kind_name(),
            actual: other.kind_name(),
        }),
        None => Err(MestStorageError::Open(String::new())),
    }
}

fn load_all_buckets(
    cache: &mut MestWriteBackCache,
    seh_directory: &PersistedSehDirectory,
) -> Result<BTreeMap<MestObjectHash, PersistedBucket>, MestStorageError> {
    let mut buckets = BTreeMap::new();
    for hash in seh_directory.directory.values() {
        if buckets.contains_key(hash) {
            continue;
        }
        match cache.load_object(hash)? {
            Some(PersistedMestObject::Bucket(bucket)) => {
                buckets.insert(*hash, bucket);
            }
            Some(other) => {
                return Err(MestStorageError::ObjectTypeMismatch {
                    expected: PersistedMestObject::Bucket(PersistedBucket {
                        version: 0,
                        bucket_key: Vec::new(),
                        ld: 0,
                        rdx: 0,
                        capacity: 0,
                        number: 0,
                        seg_num: 0,
                        segments: Vec::new(),
                        segment_roots: BTreeMap::new(),
                        latch_timestamp: 0,
                        delegation_list: BTreeMap::new(),
                        pending_num: 0,
                        to_del_map: BTreeMap::new(),
                    })
                    .kind_name(),
                    actual: other.kind_name(),
                })
            }
            None => return Err(MestStorageError::Open(String::new())),
        }
    }
    Ok(buckets)
}

fn load_all_mgt_nodes(
    cache: &mut MestWriteBackCache,
    root_hash: &MestObjectHash,
) -> Result<BTreeMap<MestObjectHash, PersistedMgtNode>, MestStorageError> {
    let mut nodes = BTreeMap::new();
    load_mgt_node_recursive(cache, root_hash, &mut nodes)?;
    Ok(nodes)
}

fn load_mgt_node_recursive(
    cache: &mut MestWriteBackCache,
    hash: &MestObjectHash,
    nodes: &mut BTreeMap<MestObjectHash, PersistedMgtNode>,
) -> Result<(), MestStorageError> {
    if nodes.contains_key(hash) {
        return Ok(());
    }
    let node = match cache.load_object(hash)? {
        Some(PersistedMestObject::MgtNode(node)) => node,
        Some(other) => {
            return Err(MestStorageError::ObjectTypeMismatch {
                expected: PersistedMestObject::MgtNode(PersistedMgtNode {
                    version: 0,
                    node_hash: [0; 32],
                    is_leaf: false,
                    is_dirty: false,
                    bucket_key: Vec::new(),
                    bucket_hash: None,
                    sub_node_hashes: Vec::new(),
                    data_hashes: Vec::new(),
                    cached_node_hashes: Vec::new(),
                    cached_data_hashes: Vec::new(),
                })
                .kind_name(),
                actual: other.kind_name(),
            })
        }
        None => return Err(MestStorageError::Open(String::new())),
    };

    for child_hash in node.sub_node_hashes.iter().flatten() {
        load_mgt_node_recursive(cache, child_hash, nodes)?;
    }
    for child_hash in node.cached_node_hashes.iter().flatten() {
        load_mgt_node_recursive(cache, child_hash, nodes)?;
    }

    nodes.insert(*hash, node);
    Ok(())
}

fn persist_buckets(
    meht: &MEHT,
    cache_lock: &RwLock<MestWriteBackCache>,
) -> Result<HashMap<String, MestObjectHash>, MestStorageError> {
    let buckets: Vec<(String, Arc<RwLock<Bucket>>)> = {
        let seh_r = meht.seh.read().unwrap();
        let ht = seh_r.ht.read().unwrap();
        ht.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };
    let mut hashes = HashMap::new();
    let mut cache = cache_lock.write().unwrap();
    for (dir_key, bucket) in buckets {
        let object = PersistedMestObject::Bucket(snapshot_bucket(&bucket)?);
        let hash = cache.cache_object(object, true)?;
        hashes.insert(dir_key, hash);
    }
    Ok(hashes)
}

fn persist_seh_directory(
    meht: &MEHT,
    cache_lock: &RwLock<MestWriteBackCache>,
    bucket_hashes: &HashMap<String, MestObjectHash>,
) -> Result<MestObjectHash, MestStorageError> {
    let seh_r = meht.seh.read().unwrap();
    let directory = PersistedSehDirectory {
        version: MEST_STORAGE_FORMAT_VERSION,
        gd: seh_r.gd,
        rdx: seh_r.rdx,
        bucket_capacity: seh_r.bucket_capacity,
        bucket_seg_num: seh_r.bucket_seg_num,
        buckets_number: seh_r.buckets_number,
        directory: bucket_hashes
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect::<BTreeMap<_, _>>(),
    };
    cache_lock
        .write()
        .unwrap()
        .cache_object(PersistedMestObject::SehDirectory(directory), true)
}

fn persist_mgt_root(
    meht: &MEHT,
    cache_lock: &RwLock<MestWriteBackCache>,
) -> Result<Option<MestObjectHash>, MestStorageError> {
    let root = { meht.mgt.read().unwrap().root.clone() };
    match root {
        Some(root) => persist_mgt_node(cache_lock, &root).map(Some),
        None => Ok(None),
    }
}

fn persist_mgt_node(
    cache_lock: &RwLock<MestWriteBackCache>,
    node: &Arc<RwLock<MGTNode>>,
) -> Result<MestObjectHash, MestStorageError> {
    let (node_hash, is_leaf, is_dirty, bucket_key, bucket, sub_nodes, data_hashes, cached_nodes, cached_data_hashes) = {
        let node_r = node.read().unwrap();
        (
            node_r.node_hash,
            node_r.is_leaf,
            node_r.is_dirty,
            node_r.bucket_key.clone(),
            node_r.bucket.clone(),
            node_r.sub_nodes.clone(),
            node_r.data_hashes.clone(),
            node_r.cached_nodes.clone(),
            node_r.cached_data_hashes.clone(),
        )
    };

    let bucket_hash = match bucket {
        Some(bucket) => {
            let object = PersistedMestObject::Bucket(snapshot_bucket(&bucket)?);
            Some(cache_lock.write().unwrap().cache_object(object, true)?)
        }
        None => None,
    };

    let mut sub_hashes = Vec::with_capacity(sub_nodes.len());
    for child in sub_nodes {
        sub_hashes.push(match child {
            Some(child) => Some(persist_mgt_node(cache_lock, &child)?),
            None => None,
        });
    }

    let mut cached_hashes = Vec::with_capacity(cached_nodes.len());
    for child in cached_nodes {
        cached_hashes.push(match child {
            Some(child) => Some(persist_mgt_node(cache_lock, &child)?),
            None => None,
        });
    }

    let object = PersistedMestObject::MgtNode(PersistedMgtNode {
        version: MEST_STORAGE_FORMAT_VERSION,
        node_hash,
        is_leaf,
        is_dirty,
        bucket_key,
        bucket_hash,
        sub_node_hashes: sub_hashes,
        data_hashes,
        cached_node_hashes: cached_hashes,
        cached_data_hashes,
    });
    cache_lock.write().unwrap().cache_object(object, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_persist_to_cache_snapshots_full_mest() {
        let meht = MEHT::new_with_cache_limit(4, 2, 2, 2);
        let guard = meht.read().unwrap();
        let _ = guard.insert(KVPair::new(String::from_utf8(vec![107, 48, 48]).unwrap(), String::from_utf8(vec![118, 48, 48]).unwrap()));
        let _ = guard.insert(KVPair::new(String::from_utf8(vec![107, 48, 49]).unwrap(), String::from_utf8(vec![118, 48, 49]).unwrap()));
        let _ = guard.insert(KVPair::new(String::from_utf8(vec![107, 48, 50]).unwrap(), String::from_utf8(vec![118, 48, 50]).unwrap()));
        let manifest = guard.persist_to_cache().unwrap();
        let cache_lock = guard.cache.as_ref().unwrap();
        let mut cache = cache_lock.write().unwrap();
        assert_eq!(cache.load_manifest().unwrap().unwrap(), manifest);
    }

    #[test]
    fn test_load_snapshot_reads_full_tree() {
        let meht = MEHT::new_with_cache_limit(4, 2, 2, 2);
        let guard = meht.read().unwrap();
        let _ = guard.insert(KVPair::new(String::from_utf8(vec![97]).unwrap(), String::from_utf8(vec![49]).unwrap()));
        let _ = guard.insert(KVPair::new(String::from_utf8(vec![98]).unwrap(), String::from_utf8(vec![50]).unwrap()));
        let _ = guard.insert(KVPair::new(String::from_utf8(vec![99]).unwrap(), String::from_utf8(vec![51]).unwrap()));
        let manifest = guard.persist_to_cache().unwrap();
        let cache_lock = guard.cache.as_ref().unwrap();
        let mut cache = cache_lock.write().unwrap();

        let snapshot = load_snapshot(&mut cache).unwrap().unwrap();

        assert_eq!(snapshot.manifest, manifest);
        assert!(!snapshot.seh_directory.directory.is_empty());
        assert!(!snapshot.buckets.is_empty());
        assert!(!snapshot.mgt_nodes.is_empty());
    }
}
