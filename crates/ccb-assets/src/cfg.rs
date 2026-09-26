//! `cfg/*.cfg`: the game's tuning files.
//!
//! ```text
//! ki.default {
//!     0.70 1.00   /* kiInitCoolDownTimer */
//!     7   0.54    /* GESTURE_DEF_WEIGHT */
//!         1.05    /* GESTURE_DEF_BOMB */
//! }
//! ```
//!
//! A file is a list of named blocks holding a flat stream of whitespace-separated values.
//! The original parser only reads the value stream in order; the comments carry the C++
//! field names, which we keep as labels so gameplay code can look values up by name.
//!
//! Comment rules: a `/*` that starts a line opens a block comment running to `*/` (used for
//! `#define` tables). A `/*` after values on the same line is a trailing label that ends at
//! `*/` or at the end of the line, because a few labels in the shipped files are never closed.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone)]
pub struct Line {
    pub values: Vec<String>,
    /// Text of the trailing comment, e.g. `kiInitCoolDownTimer`.
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Block {
    pub name: String,
    pub lines: Vec<Line>,
}

impl Block {
    /// All values in file order, as the original parser sees them.
    pub fn values(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().flat_map(|l| l.values.iter().map(String::as_str))
    }

    /// Finds the line whose label starts with `name` (labels often carry extra notes
    /// after the field name, like `kiShootPassedTime (%)`).
    pub fn line(&self, name: &str) -> Option<&Line> {
        self.lines.iter().find(|l| {
            l.label.as_deref().is_some_and(|lab| {
                let lab = lab.trim_start_matches("[REMOVE] ").trim_start_matches("[remove] ");
                let lab = lab.strip_prefix("float ").unwrap_or(lab);
                lab.split(|c: char| !(c.is_alphanumeric() || c == '_')).next() == Some(name)
            })
        })
    }

    pub fn floats(&self, name: &str) -> Result<Vec<f32>> {
        let l = self.line(name).with_context(|| format!("{}: no field {name}", self.name))?;
        l.values
            .iter()
            .map(|v| v.parse::<f32>().with_context(|| format!("{}.{name}: bad number {v:?}", self.name)))
            .collect()
    }

    pub fn f32(&self, name: &str) -> Result<f32> {
        Ok(*self.floats(name)?.first().with_context(|| format!("{}.{name} is empty", self.name))?)
    }

    /// A `(min, max)` pair, used for randomized timers.
    pub fn range(&self, name: &str) -> Result<(f32, f32)> {
        match self.floats(name)?.as_slice() {
            [a, b, ..] => Ok((*a, *b)),
            [a] => Ok((*a, *a)),
            [] => bail!("{}.{name} is empty", self.name),
        }
    }

    pub fn vec3(&self, name: &str) -> Result<[f32; 3]> {
        match self.floats(name)?.as_slice() {
            [x, y, z, ..] => Ok([*x, *y, *z]),
            _ => bail!("{}.{name} is not a vector", self.name),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CfgFile {
    pub blocks: Vec<Block>,
}

impl CfgFile {
    pub fn block(&self, name: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.name == name)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let mut blocks = Vec::new();
        let mut current: Option<Block> = None;
        let mut in_block_comment = false;
        for (lineno, raw) in text.lines().enumerate() {
            let mut rest = raw;
            if in_block_comment {
                match rest.find("*/") {
                    Some(i) => {
                        in_block_comment = false;
                        rest = &rest[i + 2..];
                    }
                    None => continue,
                }
            }
            let (code, label) = match rest.find("/*") {
                Some(i) => {
                    let after = &rest[i + 2..];
                    let (comment, tail_closed) = match after.find("*/") {
                        Some(j) => (&after[..j], true),
                        None => (after, false),
                    };
                    if rest[..i].trim().is_empty() {
                        // Comment at line start: a block comment (or a standalone note).
                        if !tail_closed {
                            in_block_comment = true;
                        }
                        (&rest[..0], None)
                    } else {
                        (&rest[..i], Some(comment.trim().to_string()))
                    }
                }
                None => (rest.split("//").next().unwrap_or(""), None),
            };
            let mut tokens: Vec<&str> = code.split_whitespace().collect();
            if tokens.last() == Some(&"{") {
                tokens.pop();
                if current.is_some() {
                    bail!("line {}: nested block", lineno + 1);
                }
                current = Some(Block { name: tokens.join(" "), lines: Vec::new() });
                continue;
            }
            if tokens.first() == Some(&"}") {
                blocks.push(current.take().with_context(|| format!("line {}: stray }}", lineno + 1))?);
                continue;
            }
            if tokens.is_empty() && label.is_none() {
                continue;
            }
            if let Some(b) = current.as_mut() {
                b.lines.push(Line { values: tokens.iter().map(|s| s.to_string()).collect(), label });
            }
        }
        if let Some(b) = current {
            bail!("block {} is never closed", b.name);
        }
        Ok(Self { blocks })
    }
}

/// All cfg files in a directory, keyed by block name.
#[derive(Debug, Default)]
pub struct Config {
    pub blocks: HashMap<String, Block>,
}

impl Config {
    pub fn load_dir(dir: &std::path::Path) -> Result<Self> {
        let mut blocks = HashMap::new();
        for e in std::fs::read_dir(dir)? {
            let p = e?.path();
            if p.extension().is_some_and(|x| x == "cfg") {
                let text = crate::latin1(&std::fs::read(&p)?);
                let f = CfgFile::parse(&text).with_context(|| p.display().to_string())?;
                blocks.extend(f.blocks.into_iter().map(|b| (b.name.clone(), b)));
            }
        }
        Ok(Self { blocks })
    }

    pub fn block(&self, name: &str) -> Result<&Block> {
        self.blocks.get(name).with_context(|| format!("missing cfg block {name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "/*\n#define A 0\n*/\nki.default {\n\t0\t\t/* kiLevel */\n\n\t/* COOLDOWNS */\n\t0.70 1.00\t/* kiInitCoolDownTimer */\n\t0.5\t\t/* kiUpgradeCancelReactionTimeFac\n\t\n\t0.35\t\t/* kiAttackUseUpgradePropability */\n\t0.20 0.25\t/* kiShootPassedTime (%) */\n\t-1 4.4 0\t/* story01SideBoundaryBoxAttractionPoint */\n}\n";

    #[test]
    fn parses_labels_and_values() {
        let f = CfgFile::parse(SAMPLE).unwrap();
        let b = f.block("ki.default").unwrap();
        assert_eq!(b.values().collect::<Vec<_>>().len(), 10);
        assert_eq!(b.range("kiInitCoolDownTimer").unwrap(), (0.70, 1.00));
        // Unterminated label must not swallow the next value.
        assert_eq!(b.f32("kiAttackUseUpgradePropability").unwrap(), 0.35);
        assert_eq!(b.range("kiShootPassedTime").unwrap(), (0.20, 0.25));
        assert_eq!(b.vec3("story01SideBoundaryBoxAttractionPoint").unwrap(), [-1.0, 4.4, 0.0]);
    }
}
