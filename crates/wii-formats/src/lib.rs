//! Parsers for the Wii file formats needed to take a WiiWare WAD apart:
//! installable WADs (ticket, TMD, encrypted contents), U8 archives and DOL executables.

pub mod brres;
pub mod dol;
pub mod gx_texture;
pub mod lz;
pub mod tpl;
pub mod u8arc;
pub mod wad;

pub(crate) fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}

pub(crate) fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub(crate) fn be64(b: &[u8], o: usize) -> u64 {
    u64::from_be_bytes(b[o..o + 8].try_into().unwrap())
}
