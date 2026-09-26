//! U8 archives (magic `0x55AA382D`), used for the banner and WiiWare game data.

use anyhow::{ensure, Context, Result};

use crate::be32;

pub const MAGIC: u32 = 0x55AA_382D;

pub struct U8Entry<'a> {
    /// Full path using `/` separators, relative to the archive root.
    pub path: String,
    /// `None` for directories.
    pub data: Option<&'a [u8]>,
}

pub fn is_u8(d: &[u8]) -> bool {
    d.len() >= 4 && be32(d, 0) == MAGIC
}

pub fn parse(d: &[u8]) -> Result<Vec<U8Entry<'_>>> {
    ensure!(is_u8(d), "not a U8 archive");
    let root = be32(d, 4) as usize;
    let count = be32(d, root + 8) as usize;
    let strings = root + count * 12;
    let name = |off: usize| -> Result<String> {
        let s = d.get(strings + off..).context("name out of range")?;
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        Ok(String::from_utf8_lossy(&s[..end]).into_owned())
    };
    let mut out = Vec::new();
    // Stack of (directory path, index one past its last child).
    let mut dirs: Vec<(String, usize)> = vec![(String::new(), count)];
    for i in 1..count {
        while dirs.last().is_some_and(|(_, end)| i >= *end) {
            dirs.pop();
        }
        let n = root + i * 12;
        let is_dir = d[n] == 1;
        let nm = name((be32(d, n) & 0x00FF_FFFF) as usize)?;
        let parent = &dirs.last().context("bad U8 tree")?.0;
        let path = match (parent.is_empty(), nm.as_str()) {
            (_, "." | "") => parent.clone(),
            (true, _) => nm,
            (false, _) => format!("{parent}/{nm}"),
        };
        let (a, b) = (be32(d, n + 4) as usize, be32(d, n + 8) as usize);
        if is_dir {
            if !path.is_empty() {
                out.push(U8Entry { path: path.clone(), data: None });
            }
            dirs.push((path, b));
        } else {
            let data = d.get(a..a + b).with_context(|| format!("{path} out of range"))?;
            out.push(U8Entry { path, data: Some(data) });
        }
    }
    Ok(out)
}
