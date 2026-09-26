//! `text/*.msgs`: localized strings, one record per `\| <id> <KEY> <text>\|`.
//!
//! `default.msgs` holds every string; a language file (`en.msgs`, `de.msgs`, …) overrides
//! a subset. Inside the text, `\_` marks where a button icon is drawn.

use std::collections::HashMap;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Message {
    pub id: u32,
    pub text: String,
}

#[derive(Debug, Default, Clone)]
pub struct Messages {
    by_key: HashMap<String, Message>,
}

pub const ICON_PLACEHOLDER: &str = "\\_";

impl Messages {
    pub fn parse(text: &str) -> Result<Self> {
        let mut m = Self::default();
        m.merge(text)?;
        Ok(m)
    }

    /// Adds or overrides records from another file.
    pub fn merge(&mut self, text: &str) -> Result<()> {
        for rec in text.split("\\|").map(str::trim).filter(|r| !r.is_empty()) {
            let mut parts = rec.splitn(3, char::is_whitespace);
            let id = parts.next().unwrap();
            let id: u32 = id.parse().with_context(|| format!("bad message id in {rec:?}"))?;
            let key = parts.next().with_context(|| format!("message {id} has no key"))?;
            let text = parts.next().unwrap_or("").trim().to_string();
            self.by_key.insert(key.to_string(), Message { id, text });
        }
        Ok(())
    }

    /// Loads `default.msgs` and overlays `<lang>.msgs` if present.
    pub fn load(dir: &std::path::Path, lang: &str) -> Result<Self> {
        let mut m = Self::parse(&crate::latin1(&std::fs::read(dir.join("default.msgs"))?))?;
        if let Ok(b) = std::fs::read(dir.join(format!("{lang}.msgs"))) {
            m.merge(&crate::latin1(&b))?;
        }
        Ok(m)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.by_key.get(key).map(|m| m.text.as_str())
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_and_overrides() {
        let mut m = super::Messages::parse("\\| 65 CCB_MENU_BACK BACK\\|\r\n\\| 79 CONT_01 CONNECT\\_ TO THE Wii REMOTE.\\|\r\n").unwrap();
        m.merge("\\| 65 CCB_MENU_BACK ZURÜCK\\|").unwrap();
        assert_eq!(m.get("CCB_MENU_BACK"), Some("ZURÜCK"));
        assert_eq!(m.get("CONT_01"), Some("CONNECT\\_ TO THE Wii REMOTE."));
    }
}
