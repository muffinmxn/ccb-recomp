//! Decodes every sound in a BRSAR and prints stats; with an output dir, writes WAVs:
//! `cargo run --example rsar_all <file.brsar> [out_dir]`.
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&a[1])?;
    let ar = wii_formats::rsar::SoundArchive::parse(&data)?;
    let (mut ok, mut bad) = (0, 0);
    for s in &ar.sounds {
        match ar.decode(s) {
            Ok(p) => {
                ok += 1;
                let peak = p.samples.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
                let rms = (p.samples.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / p.samples.len().max(1) as f64).sqrt();
                println!("{:42} {:>6} Hz {}ch {:5.2}s peak {:5} rms {:6.0} vol {}", s.name, p.rate, p.channels, p.samples.len() as f32 / p.channels as f32 / p.rate as f32, peak, rms, s.volume);
                if let Some(dir) = a.get(2) {
                    std::fs::create_dir_all(dir)?;
                    std::fs::write(format!("{dir}/{}.wav", s.name), p.to_wav())?;
                }
            }
            Err(e) => {
                bad += 1;
                println!("{:42} ERROR {e:#}", s.name);
            }
        }
    }
    println!("{ok} decoded, {bad} failed");
    Ok(())
}
