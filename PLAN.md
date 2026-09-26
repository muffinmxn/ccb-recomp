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
| `sounds/ccb_ingame.brsar` | NW4R sound archive (RWSD/RWAR/RWAV) | done: all 100 SFX decode (DSP-ADPCM → PCM) |

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
- [x] **M2 Models.** BRRES MDL0 → meshes (vertex arrays, display lists, materials, pixel-engine state); every model in the game parses. `ccb-tools render` draws them on the CPU for checking.
  - [x] CHR0 bone animations (all 216 parse; Hermite I4/I6/I12 + linear tables).
  - [x] TEV stages → one WGSL uber-shader (`gx.wgsl`) that evaluates each material's combiner stages, konst colors, alpha test, blend/cull/depth state, in gamma space like the Wii.
  - [x] VIS0 bone visibility + bone visibility flags (fixes double chick faces, overlapping weight variants, plant parts).
  - [x] Environment-mapped texture layers (effect-matrix map mode 1: chick/bomb/plant shading).
  - [ ] CLR0/SRT0/PAT0 animations; TEV swap tables; lighting channels (currently vertex color or white);
        the bomb's stripe-mask texgen (stripes render mostly black); >2 texture layers per material.
- [ ] **M3 Engine shell.** Bevy app, asset loading from `extracted/`, music, main loop and states (boot → title → menu → ingame).
  - [x] Level scene loads from `levels.cfg`, camera from `view.cfg`, bone hierarchy + intro animation, level music, headless screenshots.
  - [x] Sky dome + horizon from `common.brres` (its fade stage is runtime-driven; zeroed for play). City, ship and graveyard all render.
  - [x] Screens: title (default) and level (`--level`).
- [ ] **M4 2D layouts.** BRLYT panes/pictures/text, BRLAN animation, BRFNT fonts → menus and HUD.
  - [x] BRLYT/BRLAN/BRFNT parsers (all 29 layouts, 141 animations, 21 fonts parse).
  - [x] Layout renderer: pane tree, origins, alpha inheritance, default NW4R combiner via the TEV shader, bitmap text.
  - [x] BRLAN playback with group binding (`pat1`) and timeline windows; title screen matches the original.
  - [x] Pointer (mouse → layout space, game hand cursor), main-menu rollover/click, title → level transition.
  - [x] HUD in levels: health bars (pane offset + texture SRT scroll), rounds stars, team icons, clock state.
  - [ ] Gesture circle (`blueprints`) and attack buttons (`attackIfc`) — wired up with gameplay in M5.
  - [ ] Custom LYT TEV stages (41 materials), window frames, remaining menu screens (battle settings, coop, credits).
- [ ] **M5 Core gameplay.** Two chick teams, ink, drawing barriers, gestures (weight, bomb, lightning, plant, UFO, ghost, octopus, mushroom), damage and win/lose. Pointer = mouse or Wii Remote-style gamepad cursor.
  - [x] Match flow: clock spin picks the first attacker (`hud_clockStart*`), turns alternate (`hud_clockAttack*`), turn timer from `attackModeTimersDuell`, game over when a team has no chicks.
  - [x] Chicks: 5 per team (one big with double health), hopping physics, damage, squash, death.
  - [x] Barriers: pointer drawing on your half, ink account + reload, 1.9 s lifetime, blocking.
  - [x] Gestures: control points from the `blueprints` layout, quality from `ingame.model.gesture` timings.
  - [x] Attacks: bomb (arc, deflects off barriers, explodes), weight (variant by quality, blocked by roofs), plant (grows from below, blocked by lids), lightning (level special, blocked by rods/roofs). Damage from the cfg damage settings.
  - [x] Attack interface like the original 1P screen: basic attacks bottom right, level special on the arc, gesture panel bottom left.
  - [ ] UFO/ghost/octopus specials and the "race" for specials, attack upgrades (A/B targets), piñata + hats, corncob man, drawing on the enemy side.
- [x] **M6 AI (`ki.cfg`).** CPU attacks after `kiAttackStartTimer` + reaction time with drawing skill/quality per gesture, and defends by drawing shields at `kiDefendByShieldHeight` over the predicted impact, with a difficulty-based miss chance. `CCB_AUTOPLAY=1` lets the CPU play both sides.
- [ ] **M7 Story mode, battle settings, unlocks, save data.**
  - [x] Arena select (`arenas` layout: city / ship / haunted wood), pause menu (`ingame_pause`, Esc; gameplay time freezes, menus keep animating), game-over banner (`gameover`), then back to arena select.
  - [ ] Battle settings, rounds, tutorial/story, chicken coop (hats), credits, records, save data.
- [ ] **M8 Polish.** Particles, 2-player local, widescreen and high resolution.
  - [x] Sound effects: BRSAR → RWSD → RWAR → RWAV, DSP-ADPCM decoder; menus, gestures, clock, attacks, chicks wired to the original SFX.

Where the data doesn't say how something behaves, we reverse engineer it from `main.dol`
(Ghidra, or decomp-toolkit's analysis) and record the findings in `docs/`.

## Legal

The repository contains only original code. Users supply their own WAD dump and the Wii
common key (`keys/common-key.bin`, gitignored). `extracted/` is gitignored.
