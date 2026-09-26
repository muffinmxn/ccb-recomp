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

## Status

Done: WAD extraction; all asset formats (textures, models incl. skinning, bone/visibility
animations, layouts, fonts, sounds); TEV shader emulation; title, arena select, pause, HUD;
duels vs CPU with rounds; bomb/weight/plant; UFO/sea-monster/ghost specials with the special
race; CPU attack/defence from `ki.*`; sound effects.

## How the original plays (from the game's own tutorial/tips text)

- Controls: A (left mouse) draws lines, selects, traces; B (right mouse) is the trigger:
  fire an attack, hit upgrade targets, shoot the Corncobman and the Piñata.
- Turns: each side has an attack time to launch one basic attack (bomb, weight, plant). You
  select, trace the dots (faster = stronger), then press the trigger to launch.
- Defence: draw lines with limited ink on your side; lines vanish after a while. Drawing on the
  opponent's side (sabotage) costs more ink.
- Bomb: explodes after a short time; a line cage contains it; touching it while drawing sets it
  off; rain douses it; lightning detonates it (bigger blast).
- Weight: its shadow shows where it lands; the blocking line must be at least as wide.
- Plant: grows up along lines (only upwards), bites chicks near it, withers without bites;
  lightning destroys it.
- Specials run independently of the attack time and are raced by both sides:
  lightning (a dark cloud over one side, which can be your own; defend with an earthed
  vertical line; lightning hits the highest point), UFO (circles first, abducts a chick for
  good; horizontal line blocks), sea monster (horizontal lines parry), ghost (disintegrates on
  touching a line; drains chick energy to the opponent).
- Upgrades: a fast trace spins a target; hitting green/red with the trigger gives the A/B
  variant (e.g. bomb → Frog Cracker acid / Ladybug Boom mini-bombs; weight → Chick Fixer glue /
  Sumo; plant → Scorpion Fern poison / Fire Flower confusion; UFO → Robo / Barbecufo; ...).
- Corncobman walks in the background; shoot his corn to heal. The Piñata grows as chicks take
  damage, falls, and is shot toward a side: hats (helmet, nurse, blue light), distortions of the
  opponent's next 3 templates (wave, pulse, rotation), line bonuses (breaker, ink filler, long).
- The big chick ("the general") takes more damage.
- Modes: Duel (1-5 wins), Time (most knockouts, chicks replaced), Pro (survive; only the
  opponent's chicks are replaced). Difficulty easy/medium/hard. 1-4 players (offense/defense
  per side). 15 teams (hats) with unlock conditions; save profiles A/B/C; records; a 4-lesson
  tutorial; the Chicken Coop encyclopedia; round-over popup (next round / stats / main menu).

## What's left, in order

1. **Fix mechanics that differ from the original:** lightning as a weather special (cloud over
   a side, earthed-rod defence); sea monster parried by horizontal lines; basic ghost dies on
   lines and drains energy; UFO abducts; weight shadow + width rule; plant climbs lines and
   withers; ink cursor indicator. (Done: attack UI from the real attackIfc + blueprints layouts,
   per-team slots, tracing panel, and arm-then-fire trigger: right mouse / space / fire button.)
2. **Upgrades** (target spin + trigger) and the A/B variants of the three basic attacks.
3. **Corncobman and Piñata** with shooting; hats, distortions and line bonuses.
4. **Line sabotage** on the opponent's side; bomb/line interactions; rain.
5. **Menus:** mode select, battle settings (arena, difficulty, rounds/time), player/team select,
   round-over popup and stats, Chicken Coop, credits, save profiles and records, unlocks.
6. **Tutorial:** the 4 lessons driven by the tutorial texts.
7. **Time and Pro modes**, local multiplayer (gamepads as extra pointers).
8. **Polish:** particles (breff), remaining TEV details (bomb stripes, >2 texture layers,
   lighting), widescreen layout adjustment, prebuilt binaries so players don't need Rust.

## Legal

The repository contains only original code. Users supply their own WAD dump and the Wii
common key (`keys/common-key.bin`, gitignored). `extracted/` is gitignored.
