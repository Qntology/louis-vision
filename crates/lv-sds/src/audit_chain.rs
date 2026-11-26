//! Shared SHA-256 hash-chained audit trail (`AuditChain`, `AuditEntry`) and buffered
//! reflection log writer with RAII flush guard (`ReflectionFlushGuard`, 64 KiB buffer).

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::audit::{Level, LogLine, GENESIS};

pub const REFLECTION_BUF_CAPACITY: usize = 64 * 1024;
pub const REFLECTION_BATCH_FLUSH_RECORDS: u32 = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    pub line: LogLine,
    pub prev: String,
    pub hash: String,
}

/// Append-only SHA-256 hash-chained audit trail backed by `lv_sds::audit::LogLine`.
#[derive(Debug, Clone)]
pub struct AuditChain {
    head: String,
    entries: Vec<AuditEntry>,
}

impl Default for AuditChain {
    fn default() -> Self {
        Self {
            head: GENESIS.to_string(),
            entries: Vec::new(),
        }
    }
}

impl AuditChain {
    pub fn head(&self) -> &str {
        &self.head
    }

    pub fn append(
        &mut self,
        ts: i64,
        cat: &str,
        msg: &str,
        data: Option<serde_json::Value>,
    ) -> &AuditEntry {
        let seq = self.entries.len() as u64 + 1;
        let line = LogLine {
            seq,
            ts,
            tick: None,
            cat: cat.to_string(),
            level: Level::Info,
            msg: msg.to_string(),
            data,
        };
        let hash = hash_entry(&self.head, &line);
        let entry = AuditEntry {
            line,
            prev: self.head.clone(),
            hash: hash.clone(),
        };
        self.head = hash;
        self.entries.push(entry);
        self.entries.last().expect("pushed entry")
    }

    /// Dual-hash verification across every link starting from `GENESIS`.
    /// Returns `Ok(())` when intact or `Err(broken_seq)` pointing to the first tampered entry.
    pub fn verify_broken_at(&self) -> Result<(), u64> {
        let mut prev = GENESIS.to_string();
        for e in &self.entries {
            if e.prev != prev {
                return Err(e.line.seq);
            }
            let want = hash_entry(&prev, &e.line);
            if e.hash != want {
                return Err(e.line.seq);
            }
            prev = want;
        }
        Ok(())
    }

    pub fn verify(&self) -> bool {
        self.verify_broken_at().is_ok()
    }

    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    pub fn entries_mut(&mut self) -> &mut Vec<AuditEntry> {
        &mut self.entries
    }
}

fn hash_entry(prev: &str, line: &LogLine) -> String {
    let body = serde_json::to_string(line).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(prev.as_bytes());
    h.update(b":");
    h.update(body.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionSyncPolicy {
    /// 64 KiB `BufWriter`, flushes every 16 records or on explicit `flush()` / RAII guard `Drop`.
    #[default]
    Batch,
    /// Flushes user-space buffer to OS page cache on every record (survives `rocm_exit_now_if_used`).
    OsFlush,
    /// Flushes buffer and calls `File::sync_all()` on every record.
    SyncAll,
}

impl ReflectionSyncPolicy {
    pub fn from_env() -> Self {
        match std::env::var("LV_REFLECTION_SYNC")
            .or_else(|_| std::env::var("LOGIS_REFLECTION_SYNC"))
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "always" | "sync" | "fsync" | "crash" => Self::SyncAll,
            "os" | "line" => Self::OsFlush,
            _ => Self::Batch,
        }
    }
}

/// Persistent 64 KiB buffered JSONL reflection writer.
pub struct ReflectionLogWriter {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
    policy: ReflectionSyncPolicy,
    unflushed_records: u32,
    written_bytes: u64,
}

impl ReflectionLogWriter {
    pub fn open(path: impl AsRef<Path>, policy: ReflectionSyncPolicy) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written_bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            path,
            writer: Some(BufWriter::with_capacity(REFLECTION_BUF_CAPACITY, file)),
            policy,
            unflushed_records: 0,
            written_bytes,
        })
    }

    pub fn policy(&self) -> ReflectionSyncPolicy {
        self.policy
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn written_bytes(&self) -> u64 {
        self.written_bytes
    }

    pub fn write_json_line<T: Serialize>(&mut self, value: &T) -> std::io::Result<()> {
        let line = serde_json::to_string(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.write_raw_line(&line)
    }

    pub fn write_raw_line(&mut self, line: &str) -> std::io::Result<()> {
        if self.writer.is_none() {
            let file = OpenOptions::new().create(true).append(true).open(&self.path)?;
            self.writer = Some(BufWriter::with_capacity(REFLECTION_BUF_CAPACITY, file));
        }
        let w = self.writer.as_mut().expect("writer reopened");
        w.write_all(line.as_bytes())?;
        w.write_all(b"\n")?;
        self.written_bytes = self.written_bytes.saturating_add(line.len() as u64 + 1);
        self.unflushed_records += 1;

        match self.policy {
            ReflectionSyncPolicy::SyncAll => {
                w.flush()?;
                w.get_ref().sync_all()?;
                self.unflushed_records = 0;
            }
            ReflectionSyncPolicy::OsFlush => {
                w.flush()?;
                self.unflushed_records = 0;
            }
            ReflectionSyncPolicy::Batch => {
                if self.unflushed_records >= REFLECTION_BATCH_FLUSH_RECORDS {
                    w.flush()?;
                    self.unflushed_records = 0;
                }
            }
        }
        Ok(())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        if let Some(w) = self.writer.as_mut() {
            w.flush()?;
            if self.policy == ReflectionSyncPolicy::SyncAll {
                w.get_ref().sync_all()?;
            }
            self.unflushed_records = 0;
        }
        Ok(())
    }

    /// Flushes pending bytes and closes the underlying OS file handle cleanly.
    pub fn release_handle(&mut self) -> std::io::Result<()> {
        self.flush()?;
        self.writer = None;
        Ok(())
    }
}

impl Drop for ReflectionLogWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

/// RAII flush guard ensuring buffered reflection logs reach disk before scope exit or fast-exit.
pub struct ReflectionFlushGuard {
    writer: Arc<Mutex<ReflectionLogWriter>>,
}

impl ReflectionFlushGuard {
    pub fn open(path: impl AsRef<Path>, policy: ReflectionSyncPolicy) -> std::io::Result<Self> {
        Ok(Self {
            writer: Arc::new(Mutex::new(ReflectionLogWriter::open(path, policy)?)),
        })
    }

    pub fn from_writer(writer: Arc<Mutex<ReflectionLogWriter>>) -> Self {
        Self { writer }
    }

    pub fn write_entry(&self, entry: &AuditEntry) -> std::io::Result<()> {
        self.writer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .write_json_line(entry)
    }

    pub fn write_raw_line(&self, line: &str) -> std::io::Result<()> {
        self.writer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .write_raw_line(line)
    }

    pub fn flush_now(&self) -> std::io::Result<()> {
        self.writer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .flush()
    }
}

impl Drop for ReflectionFlushGuard {
    fn drop(&mut self) {
        let _ = self
            .writer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .flush();
    }
}
