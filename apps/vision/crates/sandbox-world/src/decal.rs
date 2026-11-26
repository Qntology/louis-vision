//! Oriented surface label/symbol decal sampling for `Obb` boxes without breaking `Obb: Copy`.

use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecalFace {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

#[derive(Clone, Debug)]
pub struct SurfaceDecal {
    pub obb_index: usize,
    pub face: DecalFace,
    pub offset_uv_m: [f32; 2],
    pub size_uv_m: [f32; 2],
    pub tex_w: u16,
    pub tex_h: u16,
    pub luma: Arc<[u8]>,
    pub glossiness: f32,
}

impl SurfaceDecal {
    #[inline]
    pub fn sample_on_hit(
        &self,
        p_local: [f32; 3],
        n_local: [f32; 3],
        view_dir_w: [f32; 3],
        normal_w: [f32; 3],
        sun_dir_w: [f32; 3],
    ) -> Option<[u8; 3]> {
        let (face_ok, u_m, v_m) = match self.face {
            DecalFace::PosX => (n_local[0] > 0.9, -p_local[2], -p_local[1]),
            DecalFace::NegX => (n_local[0] < -0.9, p_local[2], -p_local[1]),
            DecalFace::PosY => (n_local[1] > 0.9, p_local[0], p_local[2]),
            DecalFace::NegY => (n_local[1] < -0.9, p_local[0], -p_local[2]),
            DecalFace::PosZ => (n_local[2] > 0.9, p_local[0], -p_local[1]),
            DecalFace::NegZ => (n_local[2] < -0.9, -p_local[0], -p_local[1]),
        };
        if !face_ok {
            return None;
        }
        let du = u_m - self.offset_uv_m[0];
        let dv = v_m - self.offset_uv_m[1];
        let hw = 0.5 * self.size_uv_m[0].max(1e-4);
        let hh = 0.5 * self.size_uv_m[1].max(1e-4);
        if du.abs() > hw || dv.abs() > hh {
            return None;
        }

        let u_norm = ((du + hw) / (2.0 * hw)).clamp(0.0, 0.999_999);
        let v_norm = ((dv + hh) / (2.0 * hh)).clamp(0.0, 0.999_999);
        let tx = (u_norm * self.tex_w.max(1) as f32) as usize;
        let ty = (v_norm * self.tex_h.max(1) as f32) as usize;
        let base_luma = *self.luma.get(ty * self.tex_w as usize + tx)? as f32;

        let ndotv = -(view_dir_w[0] * normal_w[0] + view_dir_w[1] * normal_w[1] + view_dir_w[2] * normal_w[2]);
        let refl = [
            view_dir_w[0] + 2.0 * ndotv * normal_w[0],
            view_dir_w[1] + 2.0 * ndotv * normal_w[1],
            view_dir_w[2] + 2.0 * ndotv * normal_w[2],
        ];
        let spec_cos = (refl[0] * sun_dir_w[0] + refl[1] * sun_dir_w[1] + refl[2] * sun_dir_w[2]).max(0.0);
        let glare = if self.glossiness > 0.05 {
            255.0 * self.glossiness * spec_cos.powi(24)
        } else {
            0.0
        };
        let out = (base_luma + glare).round().clamp(0.0, 255.0) as u8;
        Some([out, out, out])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_matching_face_and_rejects_opposite_face() {
        let decal = SurfaceDecal {
            obb_index: 0,
            face: DecalFace::PosZ,
            offset_uv_m: [0.0, 0.0],
            size_uv_m: [0.20, 0.14],
            tex_w: 4,
            tex_h: 4,
            luma: Arc::from(vec![80u8; 16]),
            glossiness: 0.0,
        };
        let hit = decal.sample_on_hit(
            [0.01, -0.01, 0.15],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
        );
        assert_eq!(hit, Some([80, 80, 80]));
        assert!(decal
            .sample_on_hit(
                [0.01, -0.01, -0.15],
                [0.0, 0.0, -1.0],
                [0.0, 0.0, -1.0],
                [0.0, 0.0, -1.0],
                [0.0, 1.0, 0.0],
            )
            .is_none());
    }
}
