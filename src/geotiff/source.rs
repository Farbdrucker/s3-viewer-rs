//! Random access to a file that may live behind HTTP range requests.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Bytes at arbitrary offsets. Implemented for memory and for S3 objects.
pub trait RangeSource: Send + Sync {
    fn size(&self) -> u64;
    /// Returns exactly `len` bytes, or fewer at the end of the file.
    fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>, String>;
}

pub struct MemorySource(pub Vec<u8>);

impl RangeSource for MemorySource {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }

    fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let start = (offset as usize).min(self.0.len());
        let end = (offset.saturating_add(len) as usize).min(self.0.len());
        Ok(self.0[start..end].to_vec())
    }
}

/// Wraps a closure, for example one that calls `Session::read_range`.
pub struct FnSource<F> {
    pub len: u64,
    pub read: F,
}

impl<F> RangeSource for FnSource<F>
where
    F: Fn(u64, u64) -> Result<Vec<u8>, String> + Send + Sync,
{
    fn size(&self) -> u64 {
        self.len
    }

    fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let end = offset.saturating_add(len).min(self.len);
        if offset >= end {
            return Ok(Vec::new());
        }
        (self.read)(offset, end - offset)
    }
}

const BLOCK: u64 = 64 * 1024;

/// Caches the source in 64 KiB blocks for the many small reads of header
/// parsing, and counts every byte that crosses the wire.
pub struct Cached<'a> {
    inner: &'a dyn RangeSource,
    blocks: Mutex<HashMap<u64, std::sync::Arc<Vec<u8>>>>,
    fetched: AtomicU64,
    requests: AtomicU64,
}

impl<'a> Cached<'a> {
    pub fn new(inner: &'a dyn RangeSource) -> Self {
        Cached { inner, blocks: Mutex::new(HashMap::new()), fetched: AtomicU64::new(0), requests: AtomicU64::new(0) }
    }

    pub fn size(&self) -> u64 {
        self.inner.size()
    }

    pub fn fetched(&self) -> u64 {
        self.fetched.load(Ordering::Relaxed)
    }

    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Read for large payloads such as tiles. Served from the cache when the
    /// header blocks already hold it (COGs keep small overviews up front),
    /// otherwise fetched without filling the cache.
    pub fn read_direct(&self, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        if len > 0 && offset < self.inner.size() {
            let end = offset.saturating_add(len).min(self.inner.size());
            let cached = {
                let blocks = self.blocks.lock().unwrap_or_else(|e| e.into_inner());
                (offset / BLOCK..=(end - 1) / BLOCK).all(|block| blocks.contains_key(&block))
            };
            if cached {
                return self.read(offset, len);
            }
        }
        let data = self.inner.read(offset, len)?;
        self.fetched.fetch_add(data.len() as u64, Ordering::Relaxed);
        self.requests.fetch_add(1, Ordering::Relaxed);
        Ok(data)
    }

    /// Cached read for header structures.
    pub fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let total = self.inner.size();
        if offset >= total {
            return Err("read beyond the end of the file".into());
        }
        let end = offset.saturating_add(len).min(total);
        let first = offset / BLOCK;
        let last = (end - 1) / BLOCK;
        // Fetch missing blocks, merging neighbours into one request.
        let missing: Vec<u64> = {
            let blocks = self.blocks.lock().unwrap_or_else(|e| e.into_inner());
            (first..=last).filter(|block| !blocks.contains_key(block)).collect()
        };
        let mut runs: Vec<(u64, u64)> = Vec::new();
        for block in missing {
            match runs.last_mut() {
                Some((_, run_end)) if *run_end + 1 == block => *run_end = block,
                _ => runs.push((block, block)),
            }
        }
        for (run_start, run_end) in runs {
            let start = run_start * BLOCK;
            let stop = ((run_end + 1) * BLOCK).min(total);
            let data = self.inner.read(start, stop - start)?;
            self.fetched.fetch_add(data.len() as u64, Ordering::Relaxed);
            self.requests.fetch_add(1, Ordering::Relaxed);
            let mut blocks = self.blocks.lock().unwrap_or_else(|e| e.into_inner());
            for (index, chunk) in data.chunks(BLOCK as usize).enumerate() {
                blocks.insert(run_start + index as u64, std::sync::Arc::new(chunk.to_vec()));
            }
        }
        let blocks = self.blocks.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::with_capacity((end - offset) as usize);
        for block in first..=last {
            let data = blocks.get(&block).ok_or("short read")?;
            let block_start = block * BLOCK;
            let from = offset.max(block_start) - block_start;
            let to = (end.min(block_start + data.len() as u64)).saturating_sub(block_start);
            if from >= to {
                return Err("short read".into());
            }
            out.extend_from_slice(&data[from as usize..to as usize]);
        }
        Ok(out)
    }
}
