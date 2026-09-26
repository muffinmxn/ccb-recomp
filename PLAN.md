# Chick Chick BOOM → Rust: plan

Goal: a native Rust port of *Chick Chick BOOM* (WiiWare, `WCKE`), following the
same approach as [skate-3-rust-engine] and [iw4L]: a **new engine built on Bevy + wgpu
that loads the original game's assets** from the user's own WAD. The gameplay is
rewritten from reverse engineering; no Nintendo code or data ships in this repository.

[skate-3-rust-engine]: https://github.com/SK8-ENGINE/skate-3-rust-engine
[iw4L]: https://github.com/vladtrc/iw4L

## What's in the WAD (found so far)

| Content | What it is |
|---|---|
| 0 | Channel banner (IMET) |
| 1 | **Game executable**: LZ11-compressed DOL, 2.8 MB, entry `0x80004050`, no symbols |
| 2 | **Game data** (U8): `cfg/`, `gfx/`, `layouts/`, `models/`, `sounds/`, `text/`, `savegame/` |
| 3–5 | HOME button menu (Nintendo SDK, won't be ported) |
| 6 | Manual images + HOME menu |
| 7 | Wii strap / health-warning screens |
| 8 | NAND loader (boot content) |

The executable links RVL SDK (Dec 2009) plus NW4R G3D (3D models), LYT (2D layouts),
EF (particles) and SND (sound).

Key finding: **`cfg/*.cfg` are commented text files that keep the original C++ field names**
(`kiAttackCoolDownTimer`, `story01BarrierTemplate1Start`, …). Most gameplay tuning (AI,
levels, camera, menus) can be read straight from data instead of reverse engineering.

| Asset | Format | Plan |
|---|---|---|
| `cfg/*.cfg` | commented text | parse into typed Rust structs |
| `text/*.msgs` | `\| id KEY text\|` lines | parse into a string table |
| `gfx/*.tpl`, textures in brres | GX texture formats | TPL / TEX0 decoder → RGBA8 |
| `models/*.brres` | NW4R G3D (MDL0, TEX0, CHR0, CLR0, SRT0, PAT0, VIS0) | brres → Bevy meshes + animations |
| `layouts/*.arc` | U8 of BRLYT/BRLAN/BRFNT/TPL | a 2D layout renderer for menus and HUD |
| `gfx/*.breff/.breft` | NW4R EF particles | approximate with Bevy particles |
| `sounds/*.ogg` | Ogg Vorbis music | play directly |
| `sounds/ccb_ingame.brsar` | NW4R sound archive (RWSD/RBNK/RWAR) | extract ADPCM SFX → PCM |

## Workspace layout

```
crates/
  wii-formats/   WAD, U8, LZ, DOL, TPL, BRRES, BRLYT, BRSAR parsers (no engine deps)
  ccb-extract/   CLI: WAD → extracted/ (decrypts, unpacks, decompresses)
  ccb-assets/    game-specific loaders: cfg, msgs, level definitions → typed structs
  ccb-tools/     dev CLIs: dump textures to PNG, models to glTF, sound to WAV
  ccb-game/      the Bevy game: states, rendering, input, audio, gameplay
docs/            per-subsystem reverse-engineering notes
```

## Milestones

- [x] **M0 Extraction.** Decrypt WAD, verify SHA-1, unpack U8, LZ10/LZ11, find the DOL.
- [x] **M1 Asset decoding.** cfg + msgs parsers, TPL/TEX0 decode (all GX formats incl. CMPR), PNG dump tool.
- [ ] **M2 Models.** BRRES MDL0 → meshes (vertex arrays, display lists, materials), view the level scenes.
- [ ] **M3 Engine shell.** Bevy app, asset loading from `extracted/`, music, main loop and states (boot → title → menu → ingame).
- [ ] **M4 2D layouts.** BRLYT panes/pictures/text, BRLAN animation, BRFNT fonts → menus and HUD.
- [ ] **M5 Core gameplay.** Two chick teams, ink, drawing barriers, gestures (weight, bomb, lightning, plant, UFO, ghost, octopus, mushroom), damage and win/lose. Pointer = mouse or Wii Remote-style gamepad cursor.
- [ ] **M6 AI (`ki.cfg`).** CPU opponent driven by the original parameters.
- [ ] **M7 Story mode, battle settings, unlocks, save data.**
- [ ] **M8 Polish.** Particles, SFX from BRSAR, 2-player local, widescreen and high resolution.

Where the data doesn't say how something behaves, we reverse engineer it from `main.dol`
(Ghidra, or decomp-toolkit's analysis) and record the findings in `docs/`.

## Legal

The repository contains only original code. Users supply their own WAD dump and the Wii
common key (`keys/common-key.bin`, gitignored). `extracted/` is gitignored.
