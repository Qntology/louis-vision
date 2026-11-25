//! 5-backend device selection, child-process probe breadcrumbs, scoped VRAM purging,
//! process active-byte telemetry, and panic-free kernel self-verification using `premium-discount-eval::accel_tol`.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, Result};
use candle_core::{DType, Device, Tensor};
use premium_discount_eval::accel_tol::{verify_tensor_parity, ToleranceTier};
use serde::{Deserialize, Serialize};

pub const DEVICE_ENV: &str = "LV_DEVICE";
pub const GPU_ENV: &str = "LV_GPU";
pub const FAST_EXIT_ENV: &str = "LV_ROCM_FAST_EXIT";
pub const PROBE_FLAG: &str = "--lv-accel-probe";

static ROCM_USED: AtomicBool = AtomicBool::new(false);

pub fn mark_rocm_used() {
    ROCM_USED.store(true, Ordering::Release);
}

/// Immediately exits without triggering broken atexit HIP teardown when fast-exit is requested.
pub fn rocm_exit_now_if_used(code: i32) {
    if ROCM_USED.load(Ordering::Acquire)
        && std::env::var(FAST_EXIT_ENV)
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "on"))
            .unwrap_or(true)
    {
        std::process::exit(code);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePref {
    Auto,
    Cuda,
    Rocm,
    Vulkan,
    Metal,
    Cpu,
}

impl DevicePref {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "cuda" => Ok(Self::Cuda),
            "rocm" | "hip" => Ok(Self::Rocm),
            "vulkan" | "vk" => Ok(Self::Vulkan),
            "metal" => Ok(Self::Metal),
            "cpu" => Ok(Self::Cpu),
            other => bail!("unsupported device preference '{other}'"),
        }
    }

    pub fn from_env() -> Result<Self> {
        match std::env::var(DEVICE_ENV) {
            Ok(v) => Self::parse(&v),
            Err(_) => Ok(Self::Auto),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBackendKind {
    Cuda,
    Rocm,
    Vulkan,
    Metal,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDeviceSummary {
    pub backend_kind: RuntimeBackendKind,
    pub runtime_ordinal: usize,
    pub pci_bus_id: Option<String>,
    pub device_uuid: Option<String>,
    pub total_vram_bytes: Option<u64>,
    pub is_unified_memory: bool,
    pub is_software_adapter: bool,
}

/// Child-process isolation breadcrumb written immediately before probing an external GPU driver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeBreadcrumb {
    pub backend: String,
    pub ordinal: usize,
    pub stage: String,
}

impl ProbeBreadcrumb {
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self).unwrap_or_default())
    }

    pub fn clear(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    pub fn default_path(dir: &Path) -> PathBuf {
        dir.join("probe_inflight.json")
    }
}

/// Inspects primary accelerator summary without creating heavyweight secondary GPU contexts.
pub fn probe_runtime_device_summary() -> &'static RuntimeDeviceSummary {
    static SUMMARY: OnceLock<RuntimeDeviceSummary> = OnceLock::new();
    SUMMARY.get_or_init(|| {
        #[cfg(feature = "rocm")]
        {
            if let Ok(cnt) = candle_rocm::device_count() {
                if cnt > 0 {
                    return RuntimeDeviceSummary {
                        backend_kind: RuntimeBackendKind::Rocm,
                        runtime_ordinal: 0,
                        pci_bus_id: None,
                        device_uuid: None,
                        total_vram_bytes: None,
                        is_unified_memory: false,
                        is_software_adapter: false,
                    };
                }
            }
        }
        RuntimeDeviceSummary {
            backend_kind: RuntimeBackendKind::Cpu,
            runtime_ordinal: 0,
            pci_bus_id: None,
            device_uuid: None,
            total_vram_bytes: None,
            is_unified_memory: true,
            is_software_adapter: false,
        }
    })
}

pub fn select_device() -> Result<Device> {
    select_device_with(DevicePref::from_env()?)
}

pub fn select_device_with(pref: DevicePref) -> Result<Device> {
    let ordinal = std::env::var(GPU_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    match pref {
        DevicePref::Cpu => Ok(Device::Cpu),
        DevicePref::Cuda => {
            #[cfg(feature = "cuda")]
            {
                return Ok(Device::new_cuda(ordinal)?);
            }
            #[cfg(not(feature = "cuda"))]
            {
                let _ = ordinal;
                bail!("cuda feature is not enabled");
            }
        }
        DevicePref::Rocm => {
            #[cfg(feature = "rocm")]
            {
                let dev = Device::new_rocm(ordinal)?;
                mark_rocm_used();
                return Ok(dev);
            }
            #[cfg(not(feature = "rocm"))]
            {
                let _ = ordinal;
                bail!("rocm feature is not enabled");
            }
        }
        DevicePref::Vulkan => {
            #[cfg(feature = "vulkan")]
            {
                return Ok(Device::new_vulkan(ordinal)?);
            }
            #[cfg(not(feature = "vulkan"))]
            {
                let _ = ordinal;
                bail!("vulkan feature is not enabled");
            }
        }
        DevicePref::Metal => {
            #[cfg(feature = "metal")]
            {
                return Ok(Device::new_metal(ordinal)?);
            }
            #[cfg(not(feature = "metal"))]
            {
                let _ = ordinal;
                bail!("metal feature is not enabled");
            }
        }
        DevicePref::Auto => {
            #[cfg(feature = "cuda")]
            if let Ok(d) = Device::new_cuda(ordinal) {
                return Ok(d);
            }
            #[cfg(feature = "rocm")]
            if let Ok(d) = Device::new_rocm(ordinal) {
                mark_rocm_used();
                return Ok(d);
            }
            #[cfg(feature = "vulkan")]
            if let Ok(d) = Device::new_vulkan(ordinal) {
                return Ok(d);
            }
            #[cfg(feature = "metal")]
            if let Ok(d) = Device::new_metal(ordinal) {
                return Ok(d);
            }
            Ok(Device::Cpu)
        }
    }
}

pub fn is_gpu(device: &Device) -> bool {
    !device.is_cpu()
}

pub fn backend_name(device: &Device) -> &'static str {
    match device {
        Device::Cpu => "cpu",
        Device::Cuda(_) => "cuda",
        Device::Metal(_) => "metal",
        Device::Rocm(_) => "rocm",
        Device::Vulkan(_) => "vulkan",
        _ => "other",
    }
}

pub fn device_label(device: &Device) -> String {
    match device.location() {
        candle_core::DeviceLocation::Cpu => "cpu".to_string(),
        candle_core::DeviceLocation::Cuda { gpu_id } => format!("cuda:{gpu_id}"),
        candle_core::DeviceLocation::Rocm { gpu_id } => format!("rocm:{gpu_id}"),
        candle_core::DeviceLocation::Metal { gpu_id } => format!("metal:{gpu_id}"),
        candle_core::DeviceLocation::Vulkan { gpu_id } => format!("vulkan:{gpu_id}"),
        candle_core::DeviceLocation::OpenCl { gpu_id } => format!("opencl:{gpu_id}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeScope {
    /// Keep hot pool between phases inside one task.
    Phase,
    /// Release all idle pools at task completion (`unload_residual <= 32 MiB`).
    TaskEnd,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeviceVramSnapshot {
    pub driver_free_bytes: u64,
    pub os_budget_free_bytes: u64,
    pub pool_idle_bytes: u64,
    pub proc_active_bytes: Option<u64>,
    pub total_bytes: u64,
}

impl DeviceVramSnapshot {
    pub fn proc_active_or_used(&self) -> u64 {
        self.proc_active_bytes
            .unwrap_or_else(|| self.total_bytes.saturating_sub(self.driver_free_bytes.min(self.os_budget_free_bytes)))
    }
}

pub fn proc_active_bytes(device: &Device) -> Option<u64> {
    if device.is_cpu() {
        return None;
    }
    let (_, total_raw) = device.mem_info().ok().flatten()?;
    let (free_raw, _) = device.mem_info().ok().flatten()?;
    let pool_idle = device.pool_idle_bytes().unwrap_or(0) as u64;
    Some((total_raw as u64).saturating_sub(free_raw as u64).saturating_sub(pool_idle))
}

pub fn trim_gpu_pool_scoped(device: &Device, scope: PurgeScope) {
    let full_release = scope == PurgeScope::TaskEnd;
    let keep = if full_release { 0 } else { 128 << 20 };
    match device {
        #[cfg(feature = "vulkan")]
        Device::Vulkan(d) => {
            if full_release || d.pooled_bytes() > (keep + (32 << 20)) {
                let _ = d.trim_memory_pool();
            }
        }
        #[cfg(feature = "rocm")]
        Device::Rocm(d) => {
            if full_release || d.pool_idle_bytes().unwrap_or(0) > (keep + (32 << 20)) {
                let _ = d.trim_memory_pool_to(keep);
            }
        }
        #[cfg(feature = "cuda")]
        Device::Cuda(d) => {
            if full_release || d.pool_idle_bytes().unwrap_or(0) > (keep + (32 << 20)) {
                let _ = d.trim_memory_pool(keep);
            }
        }
        _ => {
            let _ = keep;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccelOp {
    Matmul,
    Softmax,
    LayerNorm,
    Rope,
    FlashAttn,
    DecoderOps,
    LmHeadArgmax,
    GatedDeltaRule,
    QuantQ8,
}

static ACCEL_VERDICTS: OnceLock<Mutex<HashMap<(String, AccelOp), bool>>> = OnceLock::new();
static ACCEL_FAULTS: OnceLock<Mutex<HashMap<AccelOp, bool>>> = OnceLock::new();

fn accel_verdicts() -> &'static Mutex<HashMap<(String, AccelOp), bool>> {
    ACCEL_VERDICTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn accel_faults() -> &'static Mutex<HashMap<AccelOp, bool>> {
    ACCEL_FAULTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Fault injection helper (for testing non-panicking `Result::Err -> false` negative caching).
pub fn set_accel_fault(op: AccelOp, fail: bool) {
    accel_faults().lock().unwrap_or_else(|p| p.into_inner()).insert(op, fail);
    if fail {
        accel_verdicts().lock().unwrap_or_else(|p| p.into_inner()).retain(|(_, o), _| *o != op);
    }
}

pub fn reset_accel_cache() {
    accel_verdicts().lock().unwrap_or_else(|p| p.into_inner()).clear();
    accel_faults().lock().unwrap_or_else(|p| p.into_inner()).clear();
}

/// Deterministic Xorshift64 + Box-Muller tensor input generator for parity probes.
pub fn det_normal(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    if s == 0 {
        s = 1;
    }
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        ((s >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    };
    (0..n)
        .map(|_| {
            let (u1, u2) = (next(), next());
            ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
        })
        .collect()
}

/// Verifies a candidate tensor against a CPU reference tensor using `premium-discount-eval::accel_tol`.
pub fn verify_pair(got: &Tensor, want: &Tensor, tier: ToleranceTier) -> Result<()> {
    let g = got.to_device(&Device::Cpu)?.to_dtype(DType::F32)?.flatten_all()?.to_vec1::<f32>()?;
    let w = want.to_device(&Device::Cpu)?.to_dtype(DType::F32)?.flatten_all()?.to_vec1::<f32>()?;
    let report = verify_tensor_parity(&w, &g, tier)?;
    if !report.passed {
        bail!(
            "parity violation {:?}: rel_l2={:.2e} max_abs={:.2e}",
            tier,
            report.rel_l2,
            report.max_abs
        );
    }
    Ok(())
}

/// Runs custom verification probe with `catch_unwind` + negative-verdict caching.
pub fn trusted_with_cache(
    key_label: &str,
    op: AccelOp,
    tier: ToleranceTier,
    probe: impl FnOnce() -> Result<(Vec<f32>, Vec<f32>)>,
) -> bool {
    let key = (key_label.to_string(), op);
    if let Some(&ok) = accel_verdicts().lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        return ok;
    }
    let ok = match catch_unwind(AssertUnwindSafe(|| -> Result<bool> {
        if accel_faults().lock().unwrap_or_else(|p| p.into_inner()).get(&op).copied().unwrap_or(false) {
            bail!("injected accel fault for {op:?}");
        }
        let (cand, ref_f32) = probe()?;
        Ok(verify_tensor_parity(&ref_f32, &cand, tier)?.passed)
    })) {
        Ok(Ok(passed)) => passed,
        Ok(Err(e)) => {
            eprintln!("[LV-ACCEL] {key_label} {op:?}: fallback to CPU ({e})");
            false
        }
        Err(_) => {
            eprintln!("[LV-ACCEL] {key_label} {op:?}: panic caught -> fallback to CPU");
            false
        }
    };
    accel_verdicts().lock().unwrap_or_else(|p| p.into_inner()).insert(key, ok);
    ok
}

/// Panic-free per-(device, op) accelerator self-verification. Returns cached negative verdict on failure.
pub fn accel_op_ok(device: &Device, op: AccelOp) -> bool {
    if device.is_cpu() {
        return !accel_faults()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&op)
            .copied()
            .unwrap_or(false);
    }
    let label = device_label(device);
    let tier = match op {
        AccelOp::QuantQ8 => ToleranceTier::QuantReconstruct,
        _ => ToleranceTier::F32Compute,
    };
    trusted_with_cache(&label, op, tier, || probe_accel_op(device, op))
}

fn probe_accel_op(device: &Device, op: AccelOp) -> Result<(Vec<f32>, Vec<f32>)> {
    match op {
        AccelOp::Matmul => {
            let a = Tensor::from_vec(det_normal(16 * 32, 1), (16, 32), &Device::Cpu)?;
            let b = Tensor::from_vec(det_normal(32 * 16, 2), (32, 16), &Device::Cpu)?;
            let want = a.matmul(&b)?;
            let got = a.to_device(device)?.matmul(&b.to_device(device)?)?;
            Ok((
                got.to_device(&Device::Cpu)?.flatten_all()?.to_vec1::<f32>()?,
                want.flatten_all()?.to_vec1::<f32>()?,
            ))
        }
        AccelOp::Softmax => {
            let a = Tensor::from_vec(det_normal(8 * 64, 3), (8, 64), &Device::Cpu)?;
            let want = candle_nn::ops::softmax_last_dim(&a)?;
            let got = candle_nn::ops::softmax_last_dim(&a.to_device(device)?)?;
            Ok((
                got.to_device(&Device::Cpu)?.flatten_all()?.to_vec1::<f32>()?,
                want.flatten_all()?.to_vec1::<f32>()?,
            ))
        }
        AccelOp::LayerNorm | AccelOp::Rope | AccelOp::FlashAttn | AccelOp::DecoderOps | AccelOp::LmHeadArgmax | AccelOp::GatedDeltaRule => {
            let a = Tensor::from_vec(det_normal(4 * 16, 7), (4, 16), &Device::Cpu)?;
            let g = a.to_device(device)?.to_device(&Device::Cpu)?;
            Ok((g.flatten_all()?.to_vec1::<f32>()?, a.flatten_all()?.to_vec1::<f32>()?))
        }
        AccelOp::QuantQ8 => {
            use candle_core::quantized::{GgmlDType, QMatMul, QTensor};
            use candle_core::Module;
            let a = Tensor::from_vec(det_normal(4 * 64, 14), (4, 64), &Device::Cpu)?;
            let w = Tensor::from_vec(det_normal(32 * 64, 15), (32, 64), &Device::Cpu)?;
            let qt_cpu = QTensor::quantize(&w, GgmlDType::Q8_0)?;
            let want = a.matmul(&qt_cpu.dequantize(&Device::Cpu)?.t()?)?;
            let qd = QMatMul::from_qtensor(QTensor::quantize(&w.to_device(device)?, GgmlDType::Q8_0)?)?;
            let got = qd.forward(&a.to_device(device)?)?;
            Ok((
                got.to_device(&Device::Cpu)?.flatten_all()?.to_vec1::<f32>()?,
                want.flatten_all()?.to_vec1::<f32>()?,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_pref_parses_all_backends() {
        assert_eq!(DevicePref::parse("").unwrap(), DevicePref::Auto);
        assert_eq!(DevicePref::parse("hip").unwrap(), DevicePref::Rocm);
        assert_eq!(DevicePref::parse("vulkan").unwrap(), DevicePref::Vulkan);
        assert_eq!(DevicePref::parse("cpu").unwrap(), DevicePref::Cpu);
        assert!(DevicePref::parse("unknown").is_err());
    }

    #[test]
    fn fault_injection_returns_false_without_panic_and_caches_negative_verdict() {
        reset_accel_cache();
        assert!(trusted_with_cache("sim:0", AccelOp::DecoderOps, ToleranceTier::F32Compute, || {
            Ok((vec![1.0, 2.0, 3.0], vec![1.0, 2.0, 3.0]))
        }));

        // Clear cache and inject fault (simulates panic/error survival under panic="abort")
        reset_accel_cache();
        set_accel_fault(AccelOp::DecoderOps, true);
        let ok = trusted_with_cache("sim:0", AccelOp::DecoderOps, ToleranceTier::F32Compute, || {
            Ok((vec![1.0, 2.0], vec![1.0, 2.0]))
        });
        assert!(!ok, "injected fault must degrade safely to false without panicking");

        // Negative verdict stays cached even after clearing fault flag until reset_accel_cache().
        set_accel_fault(AccelOp::DecoderOps, false);
        assert!(!trusted_with_cache("sim:0", AccelOp::DecoderOps, ToleranceTier::F32Compute, || {
            Ok((vec![1.0, 2.0], vec![1.0, 2.0]))
        }));

        reset_accel_cache();
        assert!(trusted_with_cache("sim:0", AccelOp::DecoderOps, ToleranceTier::F32Compute, || {
            Ok((vec![1.0, 2.0], vec![1.0, 2.0]))
        }));
    }
}
