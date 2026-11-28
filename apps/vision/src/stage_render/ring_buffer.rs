//! Multi-camera WebP snapshot ring buffer with strict byte-cap LRU eviction (`vision-sandbox v1.8`).

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CamFrameEntry {
    pub cam_id: String,
    pub frame_idx: u32,
    pub width: u32,
    pub height: u32,
    pub webp_bytes: Vec<u8>,
}

pub struct MultiCamRingBuffer {
    max_bytes: usize,
    used_bytes: usize,
    order: VecDeque<(String, u32)>,
    frames: HashMap<(String, u32), CamFrameEntry>,
}

impl MultiCamRingBuffer {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes: max_bytes.max(4096),
            used_bytes: 0,
            order: VecDeque::new(),
            frames: HashMap::new(),
        }
    }

    pub fn push(&mut self, entry: CamFrameEntry) {
        let key = (entry.cam_id.clone(), entry.frame_idx);
        let sz = entry.webp_bytes.len();
        if let Some(old) = self.frames.remove(&key) {
            self.used_bytes = self.used_bytes.saturating_sub(old.webp_bytes.len());
            self.order.retain(|k| k != &key);
        }
        self.used_bytes += sz;
        self.frames.insert(key.clone(), entry);
        self.order.push_back(key);

        while self.used_bytes > self.max_bytes && self.order.len() > 1 {
            if let Some(evicted_key) = self.order.pop_front() {
                if let Some(old) = self.frames.remove(&evicted_key) {
                    self.used_bytes = self.used_bytes.saturating_sub(old.webp_bytes.len());
                }
            }
        }
    }

    pub fn get(&self, cam_id: &str, frame_idx: u32) -> Option<&CamFrameEntry> {
        self.frames.get(&(cam_id.to_string(), frame_idx))
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }
}
