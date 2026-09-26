//! Loaders for Chick Chick BOOM's own data formats (as opposed to the generic Wii
//! formats in `wii-formats`).

pub mod cfg;
pub mod msgs;

/// The game's text files are Latin-1 (Windows-1252 for the characters it uses).
pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

#[cfg(test)]
mod real_data {
    //! Smoke tests against the user's extracted game data; skipped when it's absent.
    use std::path::Path;

    const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../extracted/files/0002");

    #[test]
    fn loads_all_cfg_and_text() {
        let dir = Path::new(DATA);
        if !dir.exists() {
            return;
        }
        let cfg = crate::cfg::Config::load_dir(&dir.join("cfg")).unwrap();
        let ki = cfg.block("ki.default").unwrap();
        assert_eq!(ki.range("kiAttackCoolDownTimer").unwrap(), (1.0, 1.8));
        let msgs = crate::msgs::Messages::load(&dir.join("text"), "en").unwrap();
        assert!(msgs.len() > 400);
        let mut names: Vec<_> = cfg.blocks.keys().cloned().collect();
        names.sort();
        eprintln!("{} cfg blocks: {names:?}", names.len());
    }
}
