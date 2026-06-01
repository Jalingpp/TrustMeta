use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Default)]
pub struct IoStatsSnapshot {
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub read_ops: u64,
    pub write_ops: u64,
}

static READ_BYTES: AtomicU64 = AtomicU64::new(0);
static WRITE_BYTES: AtomicU64 = AtomicU64::new(0);
static READ_OPS: AtomicU64 = AtomicU64::new(0);
static WRITE_OPS: AtomicU64 = AtomicU64::new(0);

pub fn record_read(bytes: usize) {
    READ_OPS.fetch_add(1, Ordering::Relaxed);
    READ_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

pub fn record_write(bytes: usize) {
    WRITE_OPS.fetch_add(1, Ordering::Relaxed);
    WRITE_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

pub fn snapshot() -> IoStatsSnapshot {
    IoStatsSnapshot {
        read_bytes: READ_BYTES.load(Ordering::Relaxed),
        write_bytes: WRITE_BYTES.load(Ordering::Relaxed),
        read_ops: READ_OPS.load(Ordering::Relaxed),
        write_ops: WRITE_OPS.load(Ordering::Relaxed),
    }
}

pub fn reset() {
    READ_BYTES.store(0, Ordering::Relaxed);
    WRITE_BYTES.store(0, Ordering::Relaxed);
    READ_OPS.store(0, Ordering::Relaxed);
    WRITE_OPS.store(0, Ordering::Relaxed);
}

