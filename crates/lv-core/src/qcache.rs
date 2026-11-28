//! 양자화 결과 캐시 (GGUF). 원본(BF16 safetensors)에서 한 번 양자화한 텐서를 모델 폴더(`.pd-cache/`)에 두고 다음 로드에서 그대로 읽는다.
//!
//! - **지문**: 원본 파일의 길이, 헤더와 데이터 표본(데이터 구간 전체에 고르게 퍼진 4KB 블록 64개)의 SHA-256, 그리고 `recipe_key`
//!   (어느 텐서를 몇 비트로, 양자화 코드 판). 하나라도 다르면 버리고 다시 만든다.
//!   그래서 캐시는 언제나 "그 원본에서 그 레시피로 한 번 양자화한 결과"다 (재양자화가 아니다).
//!   헤더만으로는 부족하다: 구조가 같은 다른 체크포인트는 헤더가 같다 (시험으로 확인). 파일 전체 해시는 매번 1.5GB를 읽어야 해서
//!   표본을 쓴다. 표본 사이의 바이트 몇 개만 손으로 고친 파일은 알아채지 못한다 (내려받은 파일은 설치할 때 전체 해시로 확인한다).
//! - **효과**: 원본 BF16을 F32로 펼쳐 양자화하는 과정을 건너뛰어 로드가 빠르고 최대 메모리가 낮다 (실측은 README 17절).
//! - **안전**: 임시 파일에 쓰고 이름을 바꾼다. 읽다가 깨졌거나 모양·자료형이 다르면 쓰지 않고 원본에서 다시 만든다.
//!   쓰지 못해도(읽기 전용 폴더 등) 모델은 그대로 동작한다.

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use candle_core::quantized::{gguf_file, QTensor};
use candle_core::Device;
use sha2::{Digest, Sha256};

pub const CACHE_DIR: &str = ".pd-cache";
/// 캐시 파일 형식·양자화 코드의 판. 양자화 방식이 바뀌면 올린다 (옛 캐시는 지문이 달라 버려진다)
pub const FORMAT: u32 = 1;
/// 컴포넌트당 공존 가능한 기본 최대 레시피 슬롯 수 (`q8_0` + `tiered` 동시 공존, 초과 시 가장 오래된 슬롯부터 정리).
pub const MAX_RECIPE_SLOTS: usize = 2;
/// 모바일/저용량 환경에서 디스크 상주 슬롯 상한을 1로 줄일 때 쓰는 환경변수 (`PD_QCACHE_SLOTS=1`).
pub const QCACHE_SLOTS_ENV: &str = "PD_QCACHE_SLOTS";

/// 현재 프로세스의 유효 레시피 슬롯 상한 (`PD_QCACHE_SLOTS` 환경변수 지정 시 `1..=8` 클램프, 미지정 시 `MAX_RECIPE_SLOTS = 2`).
pub fn effective_recipe_slots() -> usize {
    std::env::var(QCACHE_SLOTS_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|n| n.clamp(1, 8))
        .unwrap_or(MAX_RECIPE_SLOTS)
}

/// 캐시가 어떤 원본·레시피에서 나왔는지
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub component: String,
    pub recipe_key: String,
    pub source_len: u64,
    /// 헤더 + 데이터 표본의 SHA-256
    pub source_header_sha256: String,
}

/// 데이터 표본: 블록 수와 크기
const SAMPLES: u64 = 64;
const SAMPLE_BYTES: u64 = 4096;

impl Fingerprint {
    /// safetensors 원본의 헤더(앞 8바이트 + JSON)와 데이터 표본을 읽어 지문을 만든다 (읽는 양 약 256KB)
    pub fn of(weights: &Path, component: &str, recipe_key: &str) -> std::io::Result<Self> {
        let mut f = std::fs::File::open(weights)?;
        let source_len = f.metadata()?.len();
        let mut len = [0u8; 8];
        f.read_exact(&mut len)?;
        let n = u64::from_le_bytes(len);
        if n > source_len {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "safetensors 헤더 길이가 파일보다 깁니다"));
        }
        let mut header = vec![0u8; n as usize];
        f.read_exact(&mut header)?;
        let mut h = Sha256::new();
        h.update(len);
        h.update(&header);
        let data_start = 8 + n;
        let data_len = source_len - data_start;
        let block = SAMPLE_BYTES.min(data_len);
        if block > 0 {
            let mut buf = vec![0u8; block as usize];
            for k in 0..SAMPLES {
                let offset = data_start + (data_len - block) * k / (SAMPLES - 1);
                f.seek(std::io::SeekFrom::Start(offset))?;
                f.read_exact(&mut buf)?;
                h.update(&buf);
            }
        }
        Ok(Self {
            component: component.into(),
            recipe_key: recipe_key.into(),
            source_len,
            source_header_sha256: h.finalize().iter().map(|b| format!("{b:02x}")).collect(),
        })
    }

    fn entries(&self) -> Vec<(&'static str, gguf_file::Value)> {
        vec![
            ("pd.format", gguf_file::Value::U32(FORMAT)),
            ("pd.component", gguf_file::Value::String(self.component.clone())),
            ("pd.recipe_key", gguf_file::Value::String(self.recipe_key.clone())),
            ("pd.source_len", gguf_file::Value::U64(self.source_len)),
            ("pd.source_header_sha256", gguf_file::Value::String(self.source_header_sha256.clone())),
        ]
    }

    fn matches(&self, meta: &HashMap<String, gguf_file::Value>) -> bool {
        let s = |k: &str| meta.get(k).and_then(|v| v.to_string().ok()).cloned();
        meta.get("pd.format").and_then(|v| v.to_u32().ok()) == Some(FORMAT)
            && s("pd.component").as_deref() == Some(&self.component)
            && s("pd.recipe_key").as_deref() == Some(&self.recipe_key)
            && meta.get("pd.source_len").and_then(|v| v.to_u64().ok()) == Some(self.source_len)
            && s("pd.source_header_sha256").as_deref() == Some(&self.source_header_sha256)
    }
}

/// 캐시 파일 위치: `<가중치 폴더>/.pd-cache/<component>-<recipe_key>.gguf`
pub fn cache_path(weights: &Path, fp: &Fingerprint) -> PathBuf {
    let dir = weights.parent().unwrap_or(Path::new(".")).join(CACHE_DIR);
    dir.join(format!("{}-{}.gguf", fp.component, fp.recipe_key))
}

/// 읽은 캐시. 텐서를 이름으로 하나씩 꺼내 쓴다.
pub struct QCache {
    tensors: HashMap<String, QTensor>,
}

impl QCache {
    /// 지문이 맞는 캐시를 읽는다. 없거나, 지문이 다르거나, 깨졌으면 None (원본에서 다시 만든다)
    pub fn load(path: &Path, fp: &Fingerprint, device: &Device) -> Option<Self> {
        let mut f = std::fs::File::open(path).ok()?;
        let content = gguf_file::Content::read(&mut f).ok()?;
        if !fp.matches(&content.metadata) {
            return None;
        }
        let mut tensors = HashMap::new();
        for name in content.tensor_infos.keys() {
            let q = content.tensor(&mut f, name, device).ok()?;
            tensors.insert(name.clone(), q);
        }
        Some(Self { tensors })
    }

    pub fn len(&self) -> usize {
        self.tensors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tensors.is_empty()
    }

    /// 이름이 같고 자료형·모양이 기대와 같을 때만 꺼낸다
    pub fn take(&mut self, name: &str, dtype: candle_core::quantized::GgmlDType, shape: &[usize]) -> Option<QTensor> {
        let q = self.tensors.remove(name)?;
        (q.dtype() == dtype && q.shape().dims() == shape).then_some(q)
    }

    /// 텐서들을 캐시로 쓴다 (고유 임시 파일 → fsync → 원자적 교체).
    /// 기본 `MAX_RECIPE_SLOTS = 2` (또는 `PD_QCACHE_SLOTS` 환경변수) 슬롯을 유지하고 초과한 가장 오래된 레시피만 정리한다.
    pub fn save(path: &Path, fp: &Fingerprint, tensors: &[(String, &QTensor)]) -> std::io::Result<()> {
        Self::save_with_slots(path, fp, tensors, effective_recipe_slots())
    }

    pub fn save_with_slots(
        path: &Path,
        fp: &Fingerprint,
        tensors: &[(String, &QTensor)],
        max_slots: usize,
    ) -> std::io::Result<()> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = dir.join(format!(".tmp-{}-{}-{seq}.gguf", fp.component, std::process::id()));
        {
            let mut f = std::fs::File::create(&tmp)?;
            let meta = fp.entries();
            let meta_refs: Vec<(&str, &gguf_file::Value)> = meta.iter().map(|(k, v)| (*k, v)).collect();
            let tensor_refs: Vec<(&str, &QTensor)> = tensors.iter().map(|(n, q)| (n.as_str(), *q)).collect();
            gguf_file::write(&mut f, &meta_refs, &tensor_refs).map_err(|e| std::io::Error::other(e.to_string()))?;
            f.sync_all()?;
        }
        if let Err(e) = atomic_replace(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        // 컴포넌트당 최대 `max_slots`개 레시피(기본 2: `q8_0` + `tiered`, 모바일 `PD_QCACHE_SLOTS=1`: 1)를 공존시키고,
        // 슬롯을 초과한 가장 오래된 캐시만 정리한다.
        let keep = path.file_name().map(|n| n.to_os_string());
        let prefix = format!("{}-", fp.component);
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut others: Vec<(std::time::SystemTime, String, PathBuf)> = Vec::new();
            for e in entries.flatten() {
                let name = e.file_name();
                let s = name.to_string_lossy().into_owned();
                if s.starts_with(&prefix) && s.ends_with(".gguf") && Some(name) != keep {
                    let mtime = e
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    others.push((mtime, s, e.path()));
                }
            }
            let keep_other = max_slots.max(1).saturating_sub(1);
            if others.len() > keep_other {
                // Keep newest `keep_other` entries; remove the oldest excess entries deterministically.
                others.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
                for (_, _, stale_path) in others.into_iter().skip(keep_other) {
                    let _ = std::fs::remove_file(stale_path);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp_dir(tag: &str) -> std::path::PathBuf { let d = std::env::temp_dir().join(format!("lv_qcache_{tag}_{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }
    use candle_core::quantized::GgmlDType;
    use candle_core::Tensor;

    fn source(dir: &Path, seed: f32) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join("model.safetensors");
        // 값만 다르고 이름·모양·자료형이 같다 → 헤더가 같다 (구조가 같은 다른 체크포인트)
        let t = Tensor::from_vec((0..20_000).map(|i| seed + i as f32).collect::<Vec<f32>>(), (20_000,), &Device::Cpu).unwrap();
        candle_core::safetensors::save(&HashMap::from([("w".to_string(), t)]), &p).unwrap();
        p
    }

    fn q(seed: f32) -> QTensor {
        let v: Vec<f32> = (0..256 * 2).map(|i| ((i as f32) * 0.01 + seed).sin()).collect();
        QTensor::quantize(&Tensor::from_vec(v, (2, 256), &Device::Cpu).unwrap(), GgmlDType::Q8_0).unwrap()
    }

    #[test]
    fn a_saved_cache_reads_back_only_with_the_same_fingerprint() {
        let dir = temp_dir("qcache");
        let weights = source(&dir, 1.0);
        let fp = Fingerprint::of(&weights, "text", "eg2_text_q8_0_v1").unwrap();
        let path = cache_path(&weights, &fp);
        let (a, b) = (q(0.1), q(0.2));
        QCache::save(&path, &fp, &[("layers.0.q".into(), &a), ("layers.1.q".into(), &b)]).unwrap();

        let mut cache = QCache::load(&path, &fp, &Device::Cpu).expect("같은 지문이면 읽는다");
        assert_eq!(cache.len(), 2);
        let got = cache.take("layers.0.q", GgmlDType::Q8_0, &[2, 256]).unwrap();
        let (x, y) = (got.dequantize(&Device::Cpu).unwrap(), a.dequantize(&Device::Cpu).unwrap());
        assert_eq!(x.flatten_all().unwrap().to_vec1::<f32>().unwrap(), y.flatten_all().unwrap().to_vec1::<f32>().unwrap(), "바이트 그대로");
        assert!(cache.take("layers.1.q", GgmlDType::Q4K, &[2, 256]).is_none(), "자료형이 다르면 쓰지 않는다");

        // 다른 레시피·다른 원본이면 버린다
        let other_recipe = Fingerprint { recipe_key: "eg2_text_q6k_v1".into(), ..fp.clone() };
        assert!(QCache::load(&path, &other_recipe, &Device::Cpu).is_none());
        let weights2 = source(&temp_dir("qcache-other"), 2.0);
        let header_len = |p: &Path| 8 + u64::from_le_bytes(std::fs::read(p).unwrap()[..8].try_into().unwrap()) as usize;
        assert_eq!(std::fs::read(&weights).unwrap()[..header_len(&weights)], std::fs::read(&weights2).unwrap()[..header_len(&weights2)], "헤더는 같다");
        let other_source = Fingerprint::of(&weights2, "text", "eg2_text_q8_0_v1").unwrap();
        assert_ne!(other_source.source_header_sha256, fp.source_header_sha256, "데이터가 다르면 지문이 다르다");
        // 복사본(내용이 같다)은 같은 지문: 캐시를 그대로 쓴다
        let copy_dir = temp_dir("qcache-copy");
        std::fs::create_dir_all(&copy_dir).unwrap();
        std::fs::copy(&weights, copy_dir.join("model.safetensors")).unwrap();
        assert_eq!(Fingerprint::of(&copy_dir.join("model.safetensors"), "text", "eg2_text_q8_0_v1").unwrap(), fp);
        assert!(QCache::load(&path, &Fingerprint { source_header_sha256: other_source.source_header_sha256, ..fp.clone() }, &Device::Cpu).is_none());
        // 깨진 파일은 없는 것과 같다
        std::fs::write(&path, b"not a gguf").unwrap();
        assert!(QCache::load(&path, &fp, &Device::Cpu).is_none());
    }

    #[test]
    fn saving_a_new_recipe_keeps_two_slots_and_evicts_oldest_excess_of_same_component_only() {
        let dir = temp_dir("qcache-clean");
        let weights = source(&dir, 1.0);
        let a = Fingerprint::of(&weights, "text", "eg2_text_q8_0_v1").unwrap();
        let b = Fingerprint::of(&weights, "text", "eg2_text_tiered_v1").unwrap();
        let c = Fingerprint::of(&weights, "text", "eg2_text_custom-ffff_v1").unwrap();
        let v = Fingerprint::of(&weights, "vision", "eg2_vision_q8_0_v1").unwrap();
        let t = q(0.3);
        QCache::save(&cache_path(&weights, &a), &a, &[("x".into(), &t)]).unwrap();
        QCache::save(&cache_path(&weights, &v), &v, &[("x".into(), &t)]).unwrap();
        // 2-slot coexistence: q8_0 and tiered coexist under the same component
        std::thread::sleep(std::time::Duration::from_millis(20));
        QCache::save(&cache_path(&weights, &b), &b, &[("x".into(), &t)]).unwrap();
        assert!(cache_path(&weights, &a).exists(), "q8_0 slot must coexist with tiered slot");
        assert!(cache_path(&weights, &b).exists(), "tiered slot must exist");
        assert!(cache_path(&weights, &v).exists(), "other component must remain untouched");

        // 3rd recipe exceeds MAX_RECIPE_SLOTS=2 -> oldest text cache is evicted
        std::thread::sleep(std::time::Duration::from_millis(20));
        QCache::save(&cache_path(&weights, &c), &c, &[("x".into(), &t)]).unwrap();
        let text_present = [
            cache_path(&weights, &a).exists(),
            cache_path(&weights, &b).exists(),
            cache_path(&weights, &c).exists(),
        ]
        .into_iter()
        .filter(|&ok| ok)
        .count();
        assert_eq!(text_present, MAX_RECIPE_SLOTS);
        assert!(cache_path(&weights, &c).exists());
        assert!(cache_path(&weights, &v).exists());

        // Single-slot mobile mode (`PD_QCACHE_SLOTS=1`) keeps only the latest recipe of the component.
        QCache::save_with_slots(&cache_path(&weights, &a), &a, &[("x".into(), &t)], 1).unwrap();
        assert!(cache_path(&weights, &a).exists());
        assert!(!cache_path(&weights, &b).exists());
        assert!(!cache_path(&weights, &c).exists());
        assert!(cache_path(&weights, &v).exists());
    }
}

fn atomic_replace(tmp: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if dst.exists() { let _ = std::fs::remove_file(dst); }
    std::fs::rename(tmp, dst)
}
