//! Path-injected binary SigLIP2 phrase embedding cache (`SGL2` magic, default dim 1152).

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use half::f16;
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 4] = b"SGL2";
const HEADER_LEN: usize = 8;

/// Persistent FP16 phrase vector cache keyed by SHA-256 of canonical phrase text.
#[derive(Debug, Clone)]
pub struct PhraseCache {
    path: PathBuf,
    dim: usize,
    entries: HashMap<[u8; 32], Vec<f16>>,
    dirty: bool,
}

impl PhraseCache {
    pub fn open(path: impl AsRef<Path>, dim: usize) -> Self {
        let path = path.as_ref().to_path_buf();
        let mut entries = HashMap::new();
        if let Ok(file) = File::open(&path) {
            let mut r = BufReader::new(file);
            let mut hdr = [0u8; HEADER_LEN];
            if r.read_exact(&mut hdr).is_ok() && &hdr[0..4] == MAGIC {
                let file_dim = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
                if file_dim == dim && dim > 0 {
                    let mut hash = [0u8; 32];
                    let mut raw = vec![0u8; dim * 2];
                    while r.read_exact(&mut hash).is_ok() && r.read_exact(&mut raw).is_ok() {
                        let vec16 = raw.chunks_exact(2).map(|b| f16::from_le_bytes([b[0], b[1]])).collect();
                        entries.insert(hash, vec16);
                    }
                }
            }
        }
        Self { path, dim, entries, dirty: false }
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn key(phrase: &str) -> [u8; 32] {
        Sha256::digest(phrase.as_bytes()).into()
    }

    pub fn get(&self, phrase: &str) -> Option<Vec<f32>> {
        self.entries.get(&Self::key(phrase)).map(|v| v.iter().map(|h| h.to_f32()).collect())
    }

    pub fn insert(&mut self, phrase: &str, emb: &[f32]) {
        if emb.len() != self.dim {
            return;
        }
        let encoded: Vec<f16> = emb.iter().map(|&x| f16::from_f32(x)).collect();
        self.entries.insert(Self::key(phrase), encoded);
        self.dirty = true;
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("sgl2.tmp");
        {
            let mut w = BufWriter::new(File::create(&tmp)?);
            w.write_all(MAGIC)?;
            w.write_all(&(self.dim as u32).to_le_bytes())?;
            for (h, v) in &self.entries {
                w.write_all(h)?;
                for val in v {
                    w.write_all(&val.to_le_bytes())?;
                }
            }
            w.flush()?;
        }
        let _ = std::fs::remove_file(&self.path);
        std::fs::rename(&tmp, &self.path)?;
        self.dirty = false;
        Ok(())
    }
}
