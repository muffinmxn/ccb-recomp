//! Installable WAD (`Is`) parsing and content decryption.

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
use anyhow::{bail, ensure, Context, Result};
use sha1::{Digest, Sha1};

use crate::{be16, be32, be64};

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

fn align64(x: usize) -> usize {
    (x + 63) & !63
}

#[derive(Debug, Clone)]
pub struct ContentRecord {
    pub id: u32,
    pub index: u16,
    pub kind: u16,
    pub size: u64,
    pub sha1: [u8; 20],
}

#[derive(Debug, Clone)]
pub struct Tmd {
    pub ios: u64,
    pub title_id: u64,
    pub boot_index: u16,
    pub contents: Vec<ContentRecord>,
}

impl Tmd {
    pub fn parse(t: &[u8]) -> Result<Self> {
        ensure!(t.len() >= 0x1e4, "TMD too small");
        let n = be16(t, 0x1de) as usize;
        ensure!(t.len() >= 0x1e4 + n * 36, "TMD truncated");
        let contents = (0..n)
            .map(|i| {
                let c = &t[0x1e4 + i * 36..];
                ContentRecord {
                    id: be32(c, 0),
                    index: be16(c, 4),
                    kind: be16(c, 6),
                    size: be64(c, 8),
                    sha1: c[16..36].try_into().unwrap(),
                }
            })
            .collect();
        Ok(Self {
            ios: be64(t, 0x184),
            title_id: be64(t, 0x18c),
            boot_index: be16(t, 0x1e0),
            contents,
        })
    }

    /// The 4-character game code, e.g. `WCKE`.
    pub fn game_code(&self) -> String {
        String::from_utf8_lossy(&(self.title_id as u32).to_be_bytes()).into_owned()
    }
}

pub struct Wad<'a> {
    pub ticket: &'a [u8],
    pub tmd_raw: &'a [u8],
    pub tmd: Tmd,
    data: &'a [u8],
}

impl<'a> Wad<'a> {
    pub fn parse(d: &'a [u8]) -> Result<Self> {
        ensure!(d.len() >= 0x20, "file too small for a WAD");
        let hdr = be32(d, 0) as usize;
        if &d[4..6] != b"Is" && &d[4..6] != b"ib" {
            bail!("not an installable WAD (type {:?})", &d[4..6]);
        }
        let (cert, tik, tmd, data) = (
            be32(d, 8) as usize,
            be32(d, 0x10) as usize,
            be32(d, 0x14) as usize,
            be32(d, 0x18) as usize,
        );
        let mut o = align64(hdr) + align64(cert);
        let ticket = d.get(o..o + tik).context("ticket out of range")?;
        o += align64(tik);
        let tmd_raw = d.get(o..o + tmd).context("TMD out of range")?;
        o += align64(tmd);
        let data = d.get(o..(o + data).min(d.len())).context("data out of range")?;
        Ok(Self { ticket, tmd_raw, tmd: Tmd::parse(tmd_raw)?, data })
    }

    /// Decrypts the title key from the ticket with the Wii common key.
    pub fn title_key(&self, common_key: &[u8; 16]) -> Result<[u8; 16]> {
        ensure!(self.ticket.len() >= 0x1f2, "ticket too small");
        let mut key: [u8; 16] = self.ticket[0x1bf..0x1cf].try_into().unwrap();
        let mut iv = [0u8; 16];
        iv[..8].copy_from_slice(&self.ticket[0x1dc..0x1e4]);
        Aes128CbcDec::new(common_key.into(), &iv.into())
            .decrypt_padded_mut::<NoPadding>(&mut key)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(key)
    }

    /// Decrypts every content in TMD order and verifies its SHA-1.
    pub fn decrypt_contents(&self, title_key: &[u8; 16]) -> Result<Vec<Vec<u8>>> {
        let mut o = 0usize;
        let mut out = Vec::with_capacity(self.tmd.contents.len());
        for c in &self.tmd.contents {
            let size = c.size as usize;
            let padded = (size + 15) & !15;
            let mut buf = self
                .data
                .get(o..o + padded)
                .with_context(|| format!("content {} out of range", c.index))?
                .to_vec();
            let mut iv = [0u8; 16];
            iv[..2].copy_from_slice(&c.index.to_be_bytes());
            Aes128CbcDec::new(title_key.into(), &iv.into())
                .decrypt_padded_mut::<NoPadding>(&mut buf)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            buf.truncate(size);
            let hash: [u8; 20] = Sha1::digest(&buf).into();
            ensure!(hash == c.sha1, "SHA-1 mismatch on content {} (wrong common key?)", c.index);
            out.push(buf);
            o += align64(padded);
        }
        Ok(out)
    }
}
