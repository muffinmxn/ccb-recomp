# ccb-recomp

A native Rust port-in-progress of **Chick Chick BOOM** (WiiWare, 2010), built like
[skate-3-rust-engine](https://github.com/SK8-ENGINE/skate-3-rust-engine) and
[iw4L](https://github.com/vladtrc/iw4L): a new engine that loads the original assets from
your own copy of the game. See [PLAN.md](PLAN.md) for the roadmap.

No game data or keys are included. You need your own WAD dump.

## Usage

```sh
# 1. Put the Wii common key in keys/common-key.bin (16 bytes or 32 hex chars).
# 2. Extract the WAD (decrypts, verifies, unpacks U8, decompresses LZ, pulls out main.dol):
cargo run --release -p ccb-extract -- "Chick Chick BOOM (USA).wad" -o extracted

# Dump all textures (TPL, BRRES, layout archives) to PNG:
cargo run --release -p ccb-tools -- textures extracted extracted/png

# Render a level's models to a PNG (front orthographic view) to check model parsing:
cargo run --release -p ccb-tools -- render city.png extracted/files/0002/models/city.level1.brres --only background,left,right

# List what's inside a BRRES model archive:
cargo run --release -p ccb-tools -- brres extracted/files/0002/models/city.level1.brres
```

## Running the game (work in progress)

```sh
cargo run --release -p ccb-game                        # title screen
cargo run --release -p ccb-game -- --level city.1      # a level: also ship.1, graveyard.1, ...
```

The title screen is the original `menu` layout with its intro animations and menu music.
PLAY starts a duel against the CPU on the city level:

- On your turn, click an attack (bomb, weight, plant, or the level special), then drag
  through the dots on the panel bottom-left in order. Faster is stronger. Right-click or the
  red X cancels.
- On the CPU's turn, draw ink barriers with the mouse on your half to block its attack
  (roofs for weights and lightning, lids for plants, angled walls for bombs).

Debug knobs: `CCB_AUTOPLAY=1` (CPU plays both sides), `CCB_SHOWCASE=1` (places every attack
model on the field; `CCB_SHOWCASE=bomb:bomb__fly` shows one model and clip up close), `RUST_LOG=ccb_game::game=debug` (attack/barrier timeline).
`--screenshot out.png` renders 30 frames (`CCB_SHOT_FRAME=N` to change), saves a screenshot and exits (works headless
under `xvfb-run` with Mesa's software Vulkan driver).

## Crates

| Crate | Purpose |
|---|---|
| `wii-formats` | WAD, U8, LZ10/LZ11, DOL, TPL, BRRES (MDL0, TEX0, CHR0, VIS0), BRLYT/BRLAN layouts, BRFNT fonts, all GX texture formats |
| `ccb-extract` | WAD → `extracted/` |
| `ccb-assets` | the game's `cfg` tuning files and `msgs` string tables |
| `ccb-tools` | developer dump tools |
| `ccb-game` | the game itself (Bevy) |
