//! 21-color cyclic object ID palette and `#rrggbb` formatting.

pub const PALETTE: [[u8; 3]; 21] = [
    [128, 0, 0], [0, 128, 0], [128, 128, 0], [0, 0, 128], [128, 0, 128], [0, 128, 128],
    [128, 128, 128], [64, 0, 0], [191, 0, 0], [64, 128, 0], [191, 128, 0], [64, 0, 128],
    [191, 0, 128], [64, 128, 128], [191, 128, 128], [0, 64, 0], [128, 64, 0], [0, 191, 0],
    [128, 191, 0], [0, 64, 128], [128, 64, 128],
];

pub fn object_color(object_id: u32) -> [u8; 3] {
    PALETTE[object_id as usize % PALETTE.len()]
}

pub fn hex(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}
