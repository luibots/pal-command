//! Palworld .sav integrity check.
//!
//! A compressed Palworld save has a small header:
//!   bytes 0..4  = uncompressed size (u32 LE)
//!   bytes 4..8  = compressed size   (u32 LE)
//!   bytes 8..11 = magic "PlZ"
//!   byte  11    = save format tag (0x30 / 0x31 / 0x32)
//!
//! The classic corruption from copying Level.sav mid-autosave ("too many null bytes")
//! produces a file that does NOT carry the PlZ magic — so this catches it before we
//! trust the snapshot.

pub fn is_valid_palworld_sav(data: &[u8]) -> bool {
    if data.len() < 12 {
        return false;
    }
    &data[8..11] == b"PlZ"
}

/// Heuristic: a file that is almost entirely zero bytes is a torn/interrupted write.
pub fn looks_like_null_garbage(data: &[u8]) -> bool {
    if data.is_empty() {
        return true;
    }
    let sample = &data[..data.len().min(4096)];
    let nulls = sample.iter().filter(|&&b| b == 0).count();
    nulls * 100 / sample.len() > 90
}
