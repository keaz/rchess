# tui-polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the rchess TUI fill the terminal with a board that scales to it, draw the pieces as real images (Cburnett set) where the terminal supports graphics, and add a debug mode that shows and logs every Jev request and response.

**Architecture:** `src/tui/pieces.rs` composites embedded PNGs onto each square's background and caches the results; `src/tui/graphics.rs` asks the terminal about graphics on the UI thread with a poll deadline and builds a ratatui-image `Picker`; the board draws ratatui-image widgets in each square's image area, with the text glyph styles kept for small squares and non-graphics terminals. The engine records each Jev exchange (key redacted) when `EngineConfig.trace` is on; the TUI keeps the last 50, shows them in a full-screen view (`d`), and a `debug-log` thread appends them to a JSON-lines file.

**Tech Stack:** Rust 2024 (toolchain 1.98), ratatui 0.30, ratatui-image 11.1 (Kitty / iTerm2 / Sixel / half-blocks), image 0.25 (PNG), rustix 1 (`poll`), insta snapshots, Python 3 pty smoke test.

**Spec:** `docs/superpowers/specs/2026-09-26-rchess-redesign-design.md` section 9 (sections 5 and 6 still hold where section 9 does not change them).

**How this plan was made:** every task below was first built in a prototype, reviewed through three lenses with adversarial verification, and fixed; the whole prototype then had a five-lens review and a fix wave (Task 8). Each task's patches were then replayed on a fresh clone of `feat/tui-polish`: every tests patch fails as stated, every implementation patch applies cleanly and reaches the stated test counts with fmt and clippy clean. The patches are the code to write — apply them, do not retype them.

## Global Constraints

- Work on branch `feat/tui-polish`, in place. Never modify `src/core/`, `src/board.rs`, `src/pieces/`, `src/ai.rs`. `src/engine/` changes only as the patches do (spec 9.5).
- New dependencies are exactly `ratatui-image = { version = "11.1", default-features = false, features = ["crossterm"] }`, `image = { version = "0.25", default-features = false, features = ["png"] }` and `rustix = { version = "1", features = ["event"] }`. crossterm only through `ratatui::crossterm`.
- Tests are offline and need no terminal: run every command with `env -u JEV_API_KEY -u TYPESAFE_API_KEY`; nothing calls the Jev API; snapshots use `TestBackend`.
- Never print or log an API key. The `Authorization` header is always recorded as `Bearer <redacted>`, and the key is redacted anywhere it could appear in a recorded body or error.
- Terminal safety from spec section 6 holds: every exit path restores mouse, paste, raw mode, the main screen and the cursor without printing; the graphics query runs after raw mode and the alternate screen and before mouse capture and bracketed paste, and never leaves a thread reading stdin.
- Whole-crate `cargo test` has **3 pre-existing failures** in old code (`test::test_from_index_invalid_upper`, `ai::test::test_generate_move`, `pieces::queen::test::test_possible_moves`). Check this work with `INSTA_UPDATE=no cargo test --lib tui::` and `cargo test --lib engine::`.
- Snapshot files are part of the patches. Never accept snapshots with `INSTA_UPDATE=always` or `cargo insta accept`; a `.snap.new` file is a failure to investigate.
- Apply patches with `git apply`; if one does not apply, stop and report instead of editing it by hand. When a step replaces code that an earlier review fix changed, keep the fix and report it.
- Commit trailers name the model that actually wrote the commit.
- After every task: update `docs/handoff/HANDOFF.md` and run `graphify update .` (steps included).
- Code comments, commit messages and docs are normal English.

## Review Focus

Inputs and failure modes a person will hit, each pinned by tests in the owning task:

1. A terminal that answers the graphics query late (slow SSH) or never (some pty hosts): start-up continues within about 1 s, the late answer never acts as key presses, keys typed afterwards work, and the terminal is restored (Task 4 graphics tests; Task 7 pty scenarios).
2. The API key never appears in the exchange view, the log file, `Debug` output or errors, even when a server echoes it back, JSON-escaped or not (Task 2 transport tests; Task 6 view and log tests; Task 8).
3. Resizing the terminal or changing the font mid-game: the board re-fits, images are rebuilt at the new size (the cache is cleared), and mouse hit-testing matches what is drawn (Task 3 layout tests; Task 5 cache tests).
4. A debug log path that cannot be written (read-only folder, HOME unset, a relative XDG_STATE_HOME): one warning, logging stops, the game and the view keep working (Task 6 debug tests).
5. Odd sizes and fonts: 60×20 up to 300×100, fonts from 8×16 to 16×32 and bogus cell sizes: every cell is used, squares smaller than 5×2 fall back to glyphs, and nothing panics or allocates without bound (Task 3 layout tests; Task 5 render tests; Task 8).

---

### Task 1: Piece images, dependencies and the picture compositor

**Files:**
- Create: `assets/pieces/LICENSE`, `assets/pieces/README.md`, `assets/pieces/bB.png`, `assets/pieces/bK.png`, `assets/pieces/bN.png`, `assets/pieces/bP.png`, `assets/pieces/bQ.png`, `assets/pieces/bR.png`, `assets/pieces/wB.png`, `assets/pieces/wK.png`, `assets/pieces/wN.png`, `assets/pieces/wP.png`, `assets/pieces/wQ.png`, `assets/pieces/wR.png`, `src/tui/pieces.rs`
- Modify: `Cargo.lock`, `Cargo.toml`, `src/tui/mod.rs`

**Interfaces:**
- Consumes: `crate::core::{Piece, Color, PieceKind}`.
- Produces (`src/tui/pieces.rs`): `SOURCE_SIZE: u32`; `composite(piece: Piece, background: [u8; 3], width_px: u32, height_px: u32) -> image::RgbaImage` (fit with aspect kept, centred, blended onto an opaque background of exactly that size, Lanczos3); `ImageKey { piece, background, width_px, height_px }` (`Copy + Hash`, `const fn new`, `composite()`); `ImageCache<T>` (`new`, `Default`, `get`, `get_or_insert_with(key, FnOnce(&ImageKey) -> T) -> &mut T`, `len`, `is_empty`, `clear`).
- Assets: `assets/pieces/{w,b}{K,Q,R,B,N,P}.png` (256×256 RGBA, Cburnett, BSD), `assets/pieces/LICENSE`, `assets/pieces/README.md`. The tests patch carries the PNGs as a binary git patch.
- Cargo: `ratatui-image` 11.1 (no default features, `crossterm`) and `image` 0.25 (`png`).

- [ ] **Step: Confirm the starting point**

Run: `git status --short` (only untracked `.claude/`, `graphify-out/` and user files) and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: 293 passed (engine: `cargo test --lib engine::` 96 passed).

- [ ] **Step: Apply the tests patch (write the failing tests)**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/Cargo.lock b/Cargo.lock
index 8ef4b020b3063509d91533d11964afc75920258b..1a29c9b3580db6b49cd1f7a9764be350262854f3 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -77,6 +77,16 @@ version = "0.23.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "ac07cdecf99051d9a5238b80f35af32cdeba5b336e55d957b318b50137e18da5"
 
+[[package]]
+name = "base64-simd"
+version = "0.8.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "339abbe78e73178762e23bea9dfd08e697eb3f3301cd4be981c0f78ba5859195"
+dependencies = [
+ "outref",
+ "vsimd",
+]
+
 [[package]]
 name = "bit-set"
 version = "0.5.3"
@@ -119,6 +129,18 @@ version = "2.13.2"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "3ded4057c258ba199e2d26386d3af3780957ecaee6c4ef4041c6b4b8b97c0b06"
 
+[[package]]
+name = "bitvec"
+version = "1.1.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "ddcec3d12c579d40898fe0a9a358a803c23e9c52ca3c425707f81c9436211837"
+dependencies = [
+ "funty",
+ "radium",
+ "tap",
+ "wyz",
+]
+
 [[package]]
 name = "block-buffer"
 version = "0.10.4"
@@ -145,6 +167,26 @@ name = "bytemuck"
 version = "1.25.2"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "95832e849adfb21180ccb6826a99da14e5d266ae5c2e668e1602cf234f153797"
+dependencies = [
+ "bytemuck_derive",
+]
+
+[[package]]
+name = "bytemuck_derive"
+version = "1.12.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "6a1f896587b6f2c069c73d2f0913e2d590c3990285cd2f0b6aa02b786b4c679c"
+dependencies = [
+ "proc-macro2",
+ "quote",
+ "syn 3.0.6",
+]
+
+[[package]]
+name = "byteorder-lite"
+version = "0.1.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "8f1fe948ff07f4bd06c30984e69f5b4899c516a3ef74f34df92a2df2ab535495"
 
 [[package]]
 name = "bytes"
@@ -196,11 +238,13 @@ dependencies = [
  "criterion",
  "drawille",
  "env_logger",
+ "image",
  "insta",
  "log",
  "mockall",
  "proptest",
  "ratatui",
+ "ratatui-image",
  "serde",
  "serde_json",
  "signal-hook 0.4.4",
@@ -652,6 +696,15 @@ version = "2.5.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "da7c62ceae207dd37ea5b845da6a0696c799f85e97da1ab5b7910be3c1c80223"
 
+[[package]]
+name = "fdeflate"
+version = "0.3.7"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "1e6853b52649d4ac5c0bd02320cddc5ba956bdb407c4b75a2c6b75bf51500f8c"
+dependencies = [
+ "simd-adler32",
+]
+
 [[package]]
 name = "filedescriptor"
 version = "0.8.3"
@@ -688,7 +741,7 @@ source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "6e634e2e0ebac1ee034020da1ca582e17ffe4e0f5e985823721e168928136dcb"
 dependencies = [
  "crc32fast",
- "miniz_oxide",
+ "miniz_oxide 0.9.1",
  "zlib-rs",
 ]
 
@@ -719,6 +772,12 @@ version = "2.0.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "6c2141d6d6c8512188a7891b4b01590a45f6dac67afb4f255c4124dbb86d4eaa"
 
+[[package]]
+name = "funty"
+version = "2.0.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "e6d5a32815ae3f33302d95fdcb2ce17862f8c65363dcfd29360480ba1001fc9c"
+
 [[package]]
 name = "futures-core"
 version = "0.3.34"
@@ -941,6 +1000,16 @@ dependencies = [
  "zerovec",
 ]
 
+[[package]]
+name = "icy_sixel"
+version = "0.5.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "4bfb5a63225620b59df34a235d1fb56ff7b766909c3212a8ff927511a22b181d"
+dependencies = [
+ "quantette",
+ "thiserror 2.0.21",
+]
+
 [[package]]
 name = "ident_case"
 version = "1.0.1"
@@ -968,6 +1037,19 @@ dependencies = [
  "icu_properties",
 ]
 
+[[package]]
+name = "image"
+version = "0.25.10"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "85ab80394333c02fe689eaf900ab500fbd0c2213da414687ebf995a65d5a6104"
+dependencies = [
+ "bytemuck",
+ "byteorder-lite",
+ "moxcms",
+ "num-traits",
+ "png",
+]
+
 [[package]]
 name = "indexmap"
 version = "2.14.2"
@@ -1187,6 +1269,16 @@ version = "0.2.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "68354c5c6bd36d73ff3feceb05efa59b6acb7626617f4962be322a825e61f79a"
 
+[[package]]
+name = "miniz_oxide"
+version = "0.8.9"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "1fa76a2c86f704bdb222d66965fb3d63269ce38518b83cb0575fca855ebb6316"
+dependencies = [
+ "adler2",
+ "simd-adler32",
+]
+
 [[package]]
 name = "miniz_oxide"
 version = "0.9.1"
@@ -1235,6 +1327,16 @@ dependencies = [
  "syn 2.0.99",
 ]
 
+[[package]]
+name = "moxcms"
+version = "0.8.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "bb85c154ba489f01b25c0d36ae69a87e4a1c73a72631fc6c0eb6dde34a73e44b"
+dependencies = [
+ "num-traits",
+ "pxfm",
+]
+
 [[package]]
 name = "nix"
 version = "0.29.0"
@@ -1282,6 +1384,7 @@ source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "071dfc062690e90b734c0b2273ce72ad0ffa95f0c74596bc250dcfd960262841"
 dependencies = [
  "autocfg",
+ "libm",
 ]
 
 [[package]]
@@ -1314,6 +1417,21 @@ dependencies = [
  "num-traits",
 ]
 
+[[package]]
+name = "ordered-float"
+version = "5.5.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "8c7c9e0d9b23589f26070720bac724174bfec1083e82f7854cdd0267518343c0"
+dependencies = [
+ "num-traits",
+]
+
+[[package]]
+name = "outref"
+version = "0.5.2"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "1a80800c0488c3a21695ea981a54918fbb37abf04f4d0720c453632255e2ff0e"
+
 [[package]]
 name = "palette"
 version = "0.7.7"
@@ -1321,6 +1439,7 @@ source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "ddeed8580d347d2abf3dcf06a5f0b3dc020258338526b277847cd4248a70fc64"
 dependencies = [
  "approx",
+ "bytemuck",
  "libm",
  "palette_derive",
  "palette_math",
@@ -1504,6 +1623,19 @@ dependencies = [
  "plotters-backend",
 ]
 
+[[package]]
+name = "png"
+version = "0.18.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "60769b8b31b2a9f263dae2776c37b1b28ae246943cf719eb6946a1db05128a61"
+dependencies = [
+ "bitflags 2.13.2",
+ "crc32fast",
+ "fdeflate",
+ "flate2",
+ "miniz_oxide 0.8.9",
+]
+
 [[package]]
 name = "portable-atomic"
 version = "1.15.0"
@@ -1580,7 +1712,7 @@ dependencies = [
  "bitflags 2.13.2",
  "num-traits",
  "rand 0.9.5",
- "rand_chacha",
+ "rand_chacha 0.9.0",
  "rand_xorshift",
  "regex-syntax",
  "rusty-fork",
@@ -1588,6 +1720,30 @@ dependencies = [
  "unarray",
 ]
 
+[[package]]
+name = "pxfm"
+version = "0.1.30"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "d55d956fa96f5ec02be2e13af0e20391a5aa83d6a074e3ad368959d0fab299ea"
+
+[[package]]
+name = "quantette"
+version = "0.6.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "ba5d37e94c17b8870a5b936001d2845a6782a22ebdb1706c84294eca777f6729"
+dependencies = [
+ "bitvec",
+ "bytemuck",
+ "libm",
+ "num-traits",
+ "ordered-float 5.5.0",
+ "palette",
+ "rand 0.10.3",
+ "rand_xoshiro",
+ "ref-cast",
+ "wide",
+]
+
 [[package]]
 name = "quick-error"
 version = "1.2.3"
@@ -1615,12 +1771,20 @@ version = "6.0.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "f8dcc9c7d52a811697d2151c701e0d08956f92b0e24136cf4cf27b57a6a0d9bf"
 
+[[package]]
+name = "radium"
+version = "0.7.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "dc33ff2d4973d518d823d61aa239014831e521c75da58e3df4840d3f47749d09"
+
 [[package]]
 name = "rand"
 version = "0.8.8"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "e058c7de0b26af77780c769414d6257830bb240f3c38477dbc2c16e5f54d6d4c"
 dependencies = [
+ "libc",
+ "rand_chacha 0.3.1",
  "rand_core 0.6.4",
 ]
 
@@ -1630,10 +1794,29 @@ version = "0.9.5"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "b9ef1d0d795eb7d84685bca4f72f3649f064e6641543d3a8c415898726a57b41"
 dependencies = [
- "rand_chacha",
+ "rand_chacha 0.9.0",
  "rand_core 0.9.5",
 ]
 
+[[package]]
+name = "rand"
+version = "0.10.3"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "65c9fb96cbc91e3478eaae79a69fcd3f1ae4ad052e471fe6732fff548984b4af"
+dependencies = [
+ "rand_core 0.10.1",
+]
+
+[[package]]
+name = "rand_chacha"
+version = "0.3.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "e6c10a63a0fa32252be49d21e7709d4d4baf8d231c2dbce1eaa8141b9b127d88"
+dependencies = [
+ "ppv-lite86",
+ "rand_core 0.6.4",
+]
+
 [[package]]
 name = "rand_chacha"
 version = "0.9.0"
@@ -1649,6 +1832,9 @@ name = "rand_core"
 version = "0.6.4"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "ec0be4795e2f6a28069bec0b5ff3e2ac9bafc99e6a9a7dc3547996c5c816922c"
+dependencies = [
+ "getrandom 0.2.17",
+]
 
 [[package]]
 name = "rand_core"
@@ -1659,6 +1845,12 @@ dependencies = [
  "getrandom 0.3.4",
 ]
 
+[[package]]
+name = "rand_core"
+version = "0.10.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "63b8176103e19a2643978565ca18b50549f6101881c443590420e4dc998a3c69"
+
 [[package]]
 name = "rand_xorshift"
 version = "0.4.0"
@@ -1668,6 +1860,15 @@ dependencies = [
  "rand_core 0.9.5",
 ]
 
+[[package]]
+name = "rand_xoshiro"
+version = "0.8.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "662effc7698e08ea324d3acccf8d9d7f7bf79b9785e270a174ea36e56900c91d"
+dependencies = [
+ "rand_core 0.10.1",
+]
+
 [[package]]
 name = "ratatui"
 version = "0.30.2"
@@ -1718,6 +1919,24 @@ dependencies = [
  "ratatui-core",
 ]
 
+[[package]]
+name = "ratatui-image"
+version = "11.1.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "9593d10c61ec2007f429d1fafc4f4a19fecdbae4d06b531a2d505227fb82f5ea"
+dependencies = [
+ "base64-simd",
+ "flate2",
+ "icy_sixel",
+ "image",
+ "rand 0.8.8",
+ "ratatui",
+ "rustix 0.38.25",
+ "self_cell",
+ "thiserror 1.0.69",
+ "windows",
+]
+
 [[package]]
 name = "ratatui-macros"
 version = "0.7.2"
@@ -1798,6 +2017,26 @@ dependencies = [
  "bitflags 2.13.2",
 ]
 
+[[package]]
+name = "ref-cast"
+version = "1.0.27"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "7e440fb4e4b4147295338efb76001ab9e4efc0e5839df2c47fc5ac2381d365c3"
+dependencies = [
+ "ref-cast-impl",
+]
+
+[[package]]
+name = "ref-cast-impl"
+version = "1.0.27"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "92ecd8964f8453721699a1ed72037b0db49ce2f5a5138486ee89bed6f67cdf3a"
+dependencies = [
+ "proc-macro2",
+ "quote",
+ "syn 3.0.6",
+]
+
 [[package]]
 name = "regex"
 version = "1.10.2"
@@ -1935,6 +2174,15 @@ version = "1.0.23"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "9774ba4a74de5f7b1c1451ed6cd5285a32eddb5cccb8cc655a4e50009e06477f"
 
+[[package]]
+name = "safe_arch"
+version = "1.2.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "42c6efa15875e6ecb39ca61fb0b0c1a40b84fac5a5ffe71eef7d1000c8eb3f5f"
+dependencies = [
+ "bytemuck",
+]
+
 [[package]]
 name = "same-file"
 version = "1.0.6"
@@ -1950,6 +2198,12 @@ version = "1.2.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "94143f37725109f92c262ed2cf5e59bce7498c01bcc1502d7b9afe439a4e9f49"
 
+[[package]]
+name = "self_cell"
+version = "1.3.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "2ab42ca02749e120097e328d91d415325bdf43b1c72c4c8badf37375fe40a813"
+
 [[package]]
 name = "semver"
 version = "1.0.28"
@@ -2176,6 +2430,12 @@ dependencies = [
  "syn 2.0.99",
 ]
 
+[[package]]
+name = "tap"
+version = "1.0.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "55937e1799185b12863d447f42597ed69d9928686b8d88a1df17376a097d8369"
+
 [[package]]
 name = "tempfile"
 version = "3.27.0"
@@ -2259,7 +2519,7 @@ dependencies = [
  "nix",
  "num-derive",
  "num-traits",
- "ordered-float",
+ "ordered-float 4.6.0",
  "pest",
  "pest_derive",
  "phf",
@@ -2505,6 +2765,12 @@ version = "0.9.5"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "0b928f33d975fc6ad9f86c8f283853ad26bdd5b10b7f1542aa2fa15e2289105a"
 
+[[package]]
+name = "vsimd"
+version = "0.8.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "5c3082ca00d5a5ef149bb8b555a72ae84c9c59f7250f013ac822ac2e49b19c64"
+
 [[package]]
 name = "vtparse"
 version = "0.6.2"
@@ -2654,7 +2920,7 @@ source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "5f2ab60e120fd6eaa68d9567f3226e876684639d22a4219b313ff69ec0ccd5ac"
 dependencies = [
  "log",
- "ordered-float",
+ "ordered-float 4.6.0",
  "strsim",
  "thiserror 1.0.69",
  "wezterm-dynamic-derive",
@@ -2684,6 +2950,16 @@ dependencies = [
  "wezterm-dynamic",
 ]
 
+[[package]]
+name = "wide"
+version = "1.7.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "d920ac99c3c8edce110cb8d07dbb324d6d026011dce85b1e9355b70f0adacc4f"
+dependencies = [
+ "bytemuck",
+ "safe_arch",
+]
+
 [[package]]
 name = "winapi"
 version = "0.3.9"
@@ -2715,12 +2991,76 @@ version = "0.4.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "712e227841d057c1ee1cd2fb22fa7e5a5461ae8e48fa2ca79ec42cfc1931183f"
 
+[[package]]
+name = "windows"
+version = "0.58.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "dd04d41d93c4992d421894c18c8b43496aa748dd4c081bac0dc93eb0489272b6"
+dependencies = [
+ "windows-core",
+ "windows-targets 0.52.6",
+]
+
+[[package]]
+name = "windows-core"
+version = "0.58.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "6ba6d44ec8c2591c134257ce647b7ea6b20335bf6379a27dac5f1641fcf59f99"
+dependencies = [
+ "windows-implement",
+ "windows-interface",
+ "windows-result",
+ "windows-strings",
+ "windows-targets 0.52.6",
+]
+
+[[package]]
+name = "windows-implement"
+version = "0.58.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "2bbd5b46c938e506ecbce286b6628a02171d56153ba733b6c741fc627ec9579b"
+dependencies = [
+ "proc-macro2",
+ "quote",
+ "syn 2.0.99",
+]
+
+[[package]]
+name = "windows-interface"
+version = "0.58.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "053c4c462dc91d3b1504c6fe5a726dd15e216ba718e84a0e46a88fbe5ded3515"
+dependencies = [
+ "proc-macro2",
+ "quote",
+ "syn 2.0.99",
+]
+
 [[package]]
 name = "windows-link"
 version = "0.2.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "f0805222e57f7521d6a62e36fa9163bc891acd422f971defe97d64e70d0a4fe5"
 
+[[package]]
+name = "windows-result"
+version = "0.2.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "1d1043d8214f791817bab27572aaa8af63732e11bf84aa21a45a78d6c317ae0e"
+dependencies = [
+ "windows-targets 0.52.6",
+]
+
+[[package]]
+name = "windows-strings"
+version = "0.1.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "4cd9b125c486025df0eabcb585e62173c6c9eddcec5d117d3b6e8c30e2ee4d10"
+dependencies = [
+ "windows-result",
+ "windows-targets 0.52.6",
+]
+
 [[package]]
 name = "windows-sys"
 version = "0.48.0"
@@ -2881,6 +3221,15 @@ version = "0.6.4"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "3ad82d2a33cdc9674dc7465672f271e096168fcdbe0f799d9e6db8c5892679dc"
 
+[[package]]
+name = "wyz"
+version = "0.5.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "05f360fc0b24296329c78fda852a1e9ae82de9cf7b27dae4b7f62f118f77b9ed"
+dependencies = [
+ "tap",
+]
+
 [[package]]
 name = "yoke"
 version = "0.8.1"
diff --git a/Cargo.toml b/Cargo.toml
index 970b34c1357a56158e4ce68d86068fa05147761a..817157f00112a10acaef9ab910c24644ca1f284d 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -16,6 +16,8 @@ serde_json = "1"
 ureq = { version = "3", features = ["json"] }
 ratatui = "0.30"
 signal-hook = "0.4"
+ratatui-image = { version = "11.1", default-features = false, features = ["crossterm"] }
+image = { version = "0.25", default-features = false, features = ["png"] }
 
 [dev-dependencies]
 env_logger = "0.10.1"
diff --git a/assets/pieces/LICENSE b/assets/pieces/LICENSE
new file mode 100644
index 0000000000000000000000000000000000000000..e2c0ae548a0fb4eea12e4acbbc2118a9df47cb95
--- /dev/null
+++ b/assets/pieces/LICENSE
@@ -0,0 +1,31 @@
+Chess piece images (the Cburnett set) in this directory.
+
+Copyright (c) 2006 Colin M.L. Burnett (Wikimedia Commons user Cburnett).
+All rights reserved.
+
+The author offers these images under a choice of licences (GFDL, CC BY-SA 3.0,
+BSD and GPL); rchess uses them under the BSD licence below, whose text is as
+given on the Wikimedia Commons file pages.
+
+Redistribution and use in source and binary forms, with or without
+modification, are permitted provided that the following conditions are met:
+
+  * Redistributions of source code must retain the above copyright notice, this
+    list of conditions and the following disclaimer.
+  * Redistributions in binary form must reproduce the above copyright notice,
+    this list of conditions and the following disclaimer in the documentation
+    and/or other materials provided with the distribution.
+  * Neither the name of the author nor the names of its contributors may be
+    used to endorse or promote products derived from this software without
+    specific prior written permission.
+
+THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS "AS IS" AND ANY EXPRESS
+OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF
+MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT
+SHALL THE AUTHOR AND CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT,
+INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
+LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
+PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
+LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE
+OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF
+ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
diff --git a/assets/pieces/README.md b/assets/pieces/README.md
new file mode 100644
index 0000000000000000000000000000000000000000..606e3f48d3b825cd0e0013eaeecaefc6ad7700d8
--- /dev/null
+++ b/assets/pieces/README.md
@@ -0,0 +1,53 @@
+# Piece images
+
+The twelve PNGs here are the Cburnett chess pieces by Colin M.L. Burnett
+(Wikimedia Commons user [Cburnett](https://commons.wikimedia.org/wiki/User:Cburnett)),
+rasterized once from the original SVGs and embedded in the binary by
+`src/tui/pieces.rs` with `include_bytes!`, so the program needs no files at run time.
+
+## Licence
+
+Every file page offers the images under a choice of licences
+(`{{self|GFDL|migration=relicense|BSD|GPL}}`: GFDL 1.2 or later, CC BY-SA 3.0,
+3-clause BSD, GPL 2 or later; "You may select the license of your choice").
+rchess uses them under the **BSD licence**; see [`LICENSE`](LICENSE) for the text
+and attribution. The Commons API's `extmetadata` reports only the first listed
+licence (`LicenseShortName` "CC BY-SA 3.0", `License` "cc-by-sa-3.0", `UsageTerms`
+"Creative Commons Attribution-Share Alike 3.0"); the full list, BSD included, is
+in the licence section of each file page. Checked on 2026-09-27.
+
+## Sources
+
+| PNG | Commons file page | Original SVG | SHA-1 of the SVG |
+| --- | --- | --- | --- |
+| `wK.png` | <https://commons.wikimedia.org/wiki/File:Chess_klt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/4/42/Chess_klt45.svg> | `2c7569b837971207e40f7148e2b2086aaf8e4bbd` |
+| `bK.png` | <https://commons.wikimedia.org/wiki/File:Chess_kdt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/f/f0/Chess_kdt45.svg> | `b1165ef85a3df6f1af2549ab0af78ab21b540e8a` |
+| `wQ.png` | <https://commons.wikimedia.org/wiki/File:Chess_qlt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/1/15/Chess_qlt45.svg> | `e638eb28ec25007b9f8fac8476bdf6ae1fc5a0ee` |
+| `bQ.png` | <https://commons.wikimedia.org/wiki/File:Chess_qdt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/4/47/Chess_qdt45.svg> | `ed72e75b7bdbf880a3c9bee053c8786cfab8bdb9` |
+| `wR.png` | <https://commons.wikimedia.org/wiki/File:Chess_rlt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/7/72/Chess_rlt45.svg> | `126b7779885b87e87acc713474587732711bc8d3` |
+| `bR.png` | <https://commons.wikimedia.org/wiki/File:Chess_rdt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/f/ff/Chess_rdt45.svg> | `bd0e866f1e6da8e3d6f9c0b356b60fc58391aff6` |
+| `wB.png` | <https://commons.wikimedia.org/wiki/File:Chess_blt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/b/b1/Chess_blt45.svg> | `35dd477dc22636bfcc559a01f75b00369205ffda` |
+| `bB.png` | <https://commons.wikimedia.org/wiki/File:Chess_bdt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/9/98/Chess_bdt45.svg> | `da6dd1b5ef629bacebbd2ce26c7d81ba8a205587` |
+| `wN.png` | <https://commons.wikimedia.org/wiki/File:Chess_nlt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/7/70/Chess_nlt45.svg> | `3a2253429c0e39863b3f5ecf447209dccecdc337` |
+| `bN.png` | <https://commons.wikimedia.org/wiki/File:Chess_ndt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/e/ef/Chess_ndt45.svg> | `3c79cbbda76bcf1d4e4062cef6dd3b58a250a562` |
+| `wP.png` | <https://commons.wikimedia.org/wiki/File:Chess_plt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/4/45/Chess_plt45.svg> | `09d59e2770fcee23722ac53b26e875f76d2c1eb1` |
+| `bP.png` | <https://commons.wikimedia.org/wiki/File:Chess_pdt45.svg> | <https://upload.wikimedia.org/wikipedia/commons/c/c7/Chess_pdt45.svg> | `0a6d2f3dc6327a02ca591bc489d701fbff228138` |
+
+## Conversion
+
+Each SVG (45×45 user units) was rasterized to a 256×256 RGBA PNG with a
+transparent background, using rsvg-convert version 2.63.2 (librsvg, with cairo
+1.18.6):
+
+```sh
+for k in k q r b n p; do
+  for c in l d; do
+    case $c in l) side=w ;; d) side=b ;; esac
+    K=$(echo "$k" | tr a-z A-Z)
+    rsvg-convert -w 256 -h 256 -o "assets/pieces/$side$K.png" "Chess_$k${c}t45.svg"
+  done
+done
+```
+
+`w` is White (`lt`, light), `b` is Black (`dt`, dark); the letter is the piece
+(`K`ing, `Q`ueen, `R`ook, `B`ishop, k`N`ight, `P`awn).
diff --git a/assets/pieces/bB.png b/assets/pieces/bB.png
new file mode 100644
index 0000000000000000000000000000000000000000..3ec056435171f9e31ac769e3da5266c221efae3b
GIT binary patch
literal 4713
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

literal 0
HcmV?d00001

diff --git a/assets/pieces/bK.png b/assets/pieces/bK.png
new file mode 100644
index 0000000000000000000000000000000000000000..50db6ee58285ee68f7e25c1f6c51699329bf6c9e
GIT binary patch
literal 10765
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
NB{_B3FVdz#{{u`KaaRBU

literal 0
HcmV?d00001

diff --git a/assets/pieces/bN.png b/assets/pieces/bN.png
new file mode 100644
index 0000000000000000000000000000000000000000..f926f8d7de01d70b6f29d48ddfb2572ce34c3554
GIT binary patch
literal 6672
zcmb_hhgTEb)4mCW7J8Q&igZOp=_T|cA_xLXM-ZeKN)V71m7>x`qzKZb6Hq#WL0Uuw
zkt#Jv00HSpM}PbN3*Y&6&)MCxd(WNOnYqvN%<fY&Qv(J%9y$O3gOQ=`bpT+{B@EC|
zK_@%^(mT+J%H@iIF8KG~p4V2A3c#6eBV8@a;Ow>OQ0$G79O_NXJ=UhEWHg27!#oK}
zW@eZL9gRy>9bA~BtaRA4io>l^mb*;8SY%D9f@E?<vO=Ulr=!;r`S{rZkbH7o?{lX?
zi0<I__b6t1Lv2?7fc=x&e4X3<Z9%zXGfPt&?p3q?Z<b~PBFMi_TbG&@(YpUn+l$4E
zSaJ00CQ5=P_{HcXj^hUoRK6ze*LBHNra%D3OpUswdlt9xP%oF*;7v5$bp$h1HCzN9
zumiIzm6HWctZ0t*sV&<cJoc<1yXz^;J36rU8HEisNjP_>4{Qd2ueAPNXZDY>Ma=WA
z!pxF}RjfsDxp3mH5MBmcvG}t3yOi3uo#~JT;a~tRn{BubP<U)OJD7uk3F4}W+MM$}
z-F!dw-$8HB0hQon3q~9R<2EZWg%kHtiUnqw4DS4ENy#5Z?>!~_kp1^#ONVPmn69|l
zcV_VQV>QR1PB_=LG8?}21)I6Y*I3mYpE6|g39;C&Uzd%%Ov!T7hGt)7Dv6K0bpw>x
zeOWNETS1A_H9c9Ugy&Votsb4l%_+2}BT14(>XfNF;(~OvoH+7S7`A}{2gR%Mpe-60
z+kct&<_fKIpaamesL!jbu=-V)H8^<qSvk8ZJ$6-$?tQoKzxTGNdednt!o8%1NU_B7
ze@0Kf_jYc+ndhhWz4zQ_V9R6jTw!63COty%8ti$+zDHXK5V%_NUY@LD7$LEJSJjof
zQNWb;-D833P#Ii07;bo5DgHhrzUgv0x_!fUyksp!NJ&Z8KNWU@Ul$F#PMvpx{jG93
zt6oxQblZW<0=f18DCMcre&qr0shxPq88nk9q%M?bG9MD{x%a5?^DPzF;rR|Ppa2i%
z@;75G__noTiq0sYB1jP@U)ldY&3yU=E4Ld|gVLiX@PfazXbf*8MtJ@`!zuq2uR5<W
zQ`b9pCJxta!5wtsEfcD(+fywyN_0-;>!R2Pn;q1(o+DBw85dxWshRZm*QZ&Y>Y7<t
zJhqXU`$J#_Y+#B)*<-~B2<mWB7Z47l=?~71a<m3*a8^C-cKP|3NrO3#^Bn8!VSkXW
zv$M0*>M;!=CmEx{dIpaij+tS+@#5vnhKa+f-&4&NUtTF*i|4s)?CR<&dTU<}{G%oD
z9k<31L-#-85=*7;47{$g1pRw+v4YQ(0~VhMs6P;$&=(;`Nl&kB2n%nVAgHjqT!r<^
zoig*+B9Btn;J*3+0q<-Y@8vWtW_js~ver&msgr-}+4KLF7Pd}!_?NNb932v=2MwBs
z`!*zL{<WD3-=8iBKi;#NtcuQd>i)`-_SE|aJ%vg^b<>|wTl=*W_lojzR(bb+_Cb$f
z&*M;-3HI&Ilk6r=Dy()+?VYbY19XHl)Jy{GjWZ7_h<$x;`))HFC@CDu&>iY$@0pg$
z>nHL_E)L|%-d~$o2oVb2Xxr~9H7~66FQf3KK43&1x(d+$!inkq`+M3w)f}J~64LCh
zhZ0DN>lKVt8{DD9Yiro)k~8sI8bj4>Lzz4jRP@X{i(lDt!}l`9qx0Up(KmUe@YVA;
zm!heoC*rKJ6xR$>&zUUZmoMzMdlNYZH2=L{4MIjS|9mR`n)&eX&}uT053M@FZ}Nh;
z;f(I3bJ<-PxgujzgXYtWm>LnTiToEK7k1!VvDS{zK<l3O3;o+x7thGA@}zaW<x1G@
zpg}dq!=4^}x!XPT6+Qwm-;kBo%wM0+M~8{t2&JaGqASeeH*{6;X<O)iixthwbGBj=
z^<HBWd#131h-b%HzczNG-`5ni4MoYlefO?=cX>$9&8=8kkMvb>mK}jWRQX6Vl6ocA
z{0fFzD8{KNRE)<9q2#!%Pc;|h=XdzAjJF2iCC;6zpBTjuLS!Wn&0r=LnQ;95iY&)}
zRfqX-+A!W6P-4^UUz~B#&G_w@^U%ffO(eLr!-ewFQf8eP`r3&^Yk4V)CrYo6@ijGO
z?>9dpUncC9mAq#4nQR!DSvHA_kEfRlKf?O`?4mW$BaPSIb{Vg6wwk<2Yl_q1jQKd%
zsX;-_$R5kcZ#-d`E_1V>tuvDca*|`C{kXbN7+NYKeSLkE4Yhn~{CS)gNt@?B><nmX
zE|iddZUw)E$$eKT7%a7E^35+4P9__YST_0LDxa_VeshY8i%Y}!OouGxOsHlp%lS-P
zGIw;uw9;5wTgOG59^I^p)G|KzSQ}4)8*qA;qA$!+8?tj}q33Y?cGBGqC#@%Qow2Gd
z_Q9K^4qXG=pf}&q5>o<?9#g3VESlId?0gc>9iC}^#Ht>;huuBeO5|77b#QR-T1|v;
z=W6rjC$BIl|0u4C`O%1R^xGyI*(UgRJVH#US|t};Nk8v$ytmd6M}Tr!C3f);PRJu)
zjsE!StdhqIUIp)(X%wb)O<^rZ$|EH=7qg08jgJ+0K=hnG2JB*YrP(j0lJevn?*DLW
z;4NU*-u?Z<A~crHg}k`<3|cdvM`zWbYG18x?f0DK?f6mqUEh`n6Z28*`m;JVxqdR)
zh#8wndBg{mZo@TdPcN@-!b6Ix(NjFFEkktAsqZ&nkjQtpbq6WbMZu(&*T#lWfA;KI
zSFB)!$LfZogTuYclp*psjHtIVdvG*J?9`gKee3#kt$+60H<Yolaa~Z}g)S)K;^pP#
z-fOW20wG<VZE4_H<trw{^mQc<Y>J_-rKRQgc@W1XJ38a{k-g9$i8CP~p<%E4g``F0
z=<%b#oqQB8U2N)uL~gl{_m?ySSJ-9$hA=ly`NvhAn%}r_7V@WZG&J>wNC9brGB&}d
zKU=v0N$6zKtZaK1b(vGv!P_gh(xGj_GsmFA_Hq72j`Nr57Cxs>dU_o1Z!|vONtn~t
zEi|w7nN+Yn7V9V@JhQT8MVP9B$Y>;kPxmv9J_Z>mg3{GZ+M(jKp}XtTwtTAhO+x-b
zem{ZIe5-1ILRL|nE-UwAv3`U1kBbXb4IYESYdIqJKYMsFmNS~#qH5~L^xM&Lbfx!z
zDWRC^{?1oL>C2TP71rWe<mYo*m3Gb7o53YTMe!^z(yZ3QKdMdRO>}g0U1b&}*F7!q
zZF^ZG$s_bkg#k<b4GuyxZ{6ZXG7=a_-dzu|EPOM2UgxfdAPyGN9T)X96a4;+SW9yO
zp{S<6N2^t%hx8J@4$%e-zh8&0-wj-EU0Lz;gFy0`p=d!HTT$q^GKZ2c2}k_&>8?+>
zA1GvTapo2A;UllMzg`|Jq<&+P`Dmssv@ky(X4~N1(Gs}odvf3>t9V6HN>Z|8eY!Q}
z)vJo#w&RTtf;&qC7@t8^5TnCMdC^7;_kn<O6pES2xa{?E+ni8)yH~fwG^cVjgo7A7
z<cyxb{VS=mQpUD3ejrD6%rhra{~+<D&RH%rg5D#HUjAc0>D+QKJM+syvqg{rk@dyi
z_V8mfE2|id@S~!W9FI%Sj*cF9Pt=vwcZUPDMXLz%>vgc=M8Gq4R{k2p=X!X0{xnC!
zot5AR3^@FX>7y?e_Vx+)rd#qMs|y=Y!XZHBd!>lE=i^6<b30#baDJ1VFQ^|}P+Dp@
zJTfB5kKj|m8oIi>+g8c{@ni*Bn($yg|Ln*DtC7*sg1kH+q91Q~s2UOyA|Wf=OD!1o
zZQ^GGcpV_E`4&S@e-Q1QAs8JU-Dd!sZ18z^%aKC+l80e(5_lb>CN5%JT3yX(-+I4$
zbFS0kuc@D(oPzf_4nn;;yBCO%!Nbe@7e#N3LGk0%_bq5@vI!ZT<B+zpslv0Od6PLa
z!Tk=Lem9L(X`;~h__##N{WZ^ejYOWJPv<V=$;mz9`VTC(g6+y^zGPkg>>2KbZjXbH
zGg0uciDcB?U%OQduW=s`sDSBYp$uLN3r-^`cvwW5<RVUji{Yp!W)XwLx?8hJTJ6h#
zbb;POH=d-guOBo1&OPZm%%%lfHgeS0(MTD-BhUQ}$K8=+855*B$>kW9;*s5(f*J^f
zZ9*ATizgY3Rohz#kpIMc;oC+%!{eBpSf*)<JQ$TsRhx^-%jsm>iWK~(hRZCfIIn8?
z9+{)k&>9hCr|FMbUT0jo`}{i#+TK2g`1Nc3t=Y3Qr6|klAIY*#%*~74T#mYKZf<?K
z1D`&1bUkIO;-ucoM-3s-EI2_Z%^$@%<K{Bmhht8!`STE3VFCwd>6PqUqvZ7yc#5$+
zz7X_Q98yr?oKe<+h}a{SU~UM3!j6_pjP;y;c0ILPJx0KZWDDF);Exs8n&U|7{ZFkG
zZ<u5_DewJO>ZKi=WykHGp3&&akh}dN5onz5T2us4{3Nx)szaEmE-~sKh{(%Zgjmf9
z>BQJnZ5-2Zvt5I!zz$N@MJ3dsC4PHtqTYD6c}42;V8@3AC869*8?a85j8}bnX{&Ob
z(qyww8ZQz47dqD6+Pz8{nxVe74Z;{H7#W68qv)BI?>Vu1zaS-3G1P8**VB`gUj<up
z%W;K*iHA@pf{Tn&L!dBKkHdnC->vJKm_apPW!w1X<PfTQJO8H(Lmy_bXC-qpEWumY
zR6b;LQwoRCX?HX)Qs%l{>Og&PH1^?My+_@=YnbbGSd#$X;-nW|C$ZEn^5`Ga*MmJ%
zXg1U3opi^D=8*!HWX(H89@6wmg7kjxpUG<4)PKXhkAKqOGbzQuD-hSsdR3SwqxaEO
zWjHql$z|FoYFvt#>2s~&V0K_EIWC2Vmv^gQb))Fy@XHsIcmbb~I?cYcqA(|8Sksjt
zxX_*|7=w9uyWP~x4Uo&~jwig-Ppso(khw4hGm!w%K*q67%z#s?-E?}Y@2vuOaFmrW
zHxoS1P&^=>;pxqO@W8k4PUqaA?JEVJu5bA|-H_1G)6#mlw_`*h%t&e%joklBMZRlE
zURZc)BlB`Q5H~S#D<v(>t-c-k{N$AOt?~N9ulx$WBy_Fcy#C<$_(72BV(K*wmY^xW
z*pVx|&Ph~6RjE%V3MlJ&m)=Bv$^RgiTP?U|q@_LmqlRH<p>u%2*VpQ{mo8lrol;&M
z!*l8B=}qvaC2c8_5hIpg6j6@zKL-s2Z%Bd0>HDunS~%Y{34WizINuh(nxk@gfxiwC
zC0J$F18z^QKMsm0_j#9|_%p@`R7a7Jr#1V~u!h8XUO|DdADPHfoP#}HYxKg{2=s~(
zSzcUEV+gro7Bth-UE|#OVq&z~zO{1n1~ux6b8b;iM@{YH_0|oIVt&ymW~i?iovBfa
zchqUi-#Uwmd!k=xoEp>RQmZ>p$!Inu+ddS+>-#)MmVsZz@=!JWXjjX@;dRc}%t{vj
z^4(C-#r{Ec^c4FsAL0jSD$~QA%*;&g3xv11y#IV(^}{dp^rurX2~?is7>c^AhEg5V
z6WqYdiIesP!hK5b*>?No9WHxp%7`#0j?()N5#lCWUR8hcld6p~9!MPi^<O-{J#(J>
zNMP{4)BQ9JO^<J=3OyY@@p(CWL>`5A`#%0w4vYk$0EEGf@05qs5F~%12x#qjSU4(L
z<WC1g)!?Md0Li}=;Wyut&^P;|?k?NUt)}U#S$Qx5J(%tO_o2^gEU<rwwh<C;65$cw
z^0bIVk3XMg@i@F)V>vIMU73Bo=Z!w=yQ?cJx_9rE?NxkcEZ|ePmVV2eChrq2GIbE@
z-*kO^s=nvE2^dfx^9+9}5?k~x(YNODv$(i!NHkhZ$iX71NJdSn=Z?auPF}%TAkk$l
z1t5ps-AW%Uv|Cm|_+q(j&CMSoW#2OoXn3iY`nb8B6}y@h)g8yB`s2lUhUCWmb|fk4
z%IX(1=&c&l`=f$}d>JYDo0z(lTrHl)BjjjaA1()}EV@+jYdr^B^Ky5-aKA`P%ZHc>
zVxZ2^D%-8yp=*V2-t@G^pIR`6yso}4snU$;*zECm4pEV0c>A?n4Xd~Vt2~71>7NA~
zy1cejO;32_CDJTcM$=pj?+a5HR7hU+)y50z3nDoKFz(aXtsgAJv4BW$NZ&kpKIVP6
zq2xp(CinXi8y}2adxjy8bMzz57=wNfHZUCO7{Uj6Gjlt;N-X8KsVNyqr9)I=8I<9O
z6@Y~m_)UtHA};XzE7f%h+*`U1tV5grnw3{p>V}7}jFpR|&^-9F;#Pm4X8n8{AaMd1
z?U~8Q;rTUJkg>F-plw9+@bq|JI$XTtF0Qsvc2x$1UcP6W&W)o+MW$))Ifp80n_6lq
zKnVBi$Gegd;oC@DKMXk&rKV||K4`HI%b+^k*z2?gCw(P8(a$|$zXl6qM}%L)>waD!
zt8ZX8dC1YI4=S4AgnC$?wty3A*ot-!1NxPFJz0>r=&M<c8?gpL?(ym?^9mmjq&tF*
zT*hf>o<ZeMAhMA<vxH%L@V7MN*ddg8+c+{cX;Ox=hJHD~b0PEDGdc*cl;A`vr<?rW
zZ?bQvn;(ZeGPyLwMV45)f#4r^+T%Ipu)oZT-C<CFQ{WDZ&pvOm&w{4FiC<bGMMokw
zlVjHYQWDa58hDM%OTd{kXCx$Rv*+4d=cCl}S^=1uMea}HVJEL*)*{#lb_|mYRpsSU
z3lCjc?Wlh(DhV-Vn1TE7nh3n2%_ztyc5V%asZr?2UP9cE;j|FA^0ujI+&=}H%Sjss
zaGAau!)f<gg*JAM)%R=^0iMut!OL}R)49H+t`)oNQvbvN(Efg$U2(qD6Sbs#@NsEv
zks_ESdvQ|UAgO%1ZEa#?dl1Qq6ODj5Jzf{}exlRgbDT0T=|g=e=3FYKb|GOU@1hLY
z%Yplz1NHefyP?bkmE6WmT4jg4l*X4IqYwngUX**4IdA&S+yoG@zM~+L9AqkX{C>Kw
z=oX+uo{POtL^w^IVBc^6v;)%t4{qtwQpy)+VqC;mjeGKNC)$GyJjPB94#<@z`f{}Z
zY`aXjf<_9nP=O)AtI7C@qfA>$LZpr<jj@K`77YM&d}rrj7D#{+%nhSNOX4CQ-khhN
zLPW~i+|g#_J_;9XG%}L~%F4<fW7Rn$Kl>Be{4@Kv!1Wu{A+aQ`<j1y&vDsL^3*=6Y
zLC#!ERM>~SquoeNYF`dq!}*1GG`QSUhV12&*&V-E*sPrVg`J(9su(u;bs7{6g5*7E
z+WTXI!Xx15-@QkCUiji!otg50m9xaymnY8=ht*NM&8ZCa^;MTky0*8sD`!M5RD+B#
zy5w>dBDJ6Aw+Bnq<gzG>0d0r$C(Sk0maY`RJikwF<*0>RKU9S}7mXL6;CY|KK!s0)
zBIDtUpMl9GToE1=B&Os6y=48sz?Gp(<fSFsb1txNy6FCQjTT#>tsoxZt9f#~fI&JT
zaG2vdbz{N;+-XXWfRzNeY}Wuj+*oQ@HH`28SO7bkNc`RsT4Uz3-wA{B5<AM!0BE(+
z{2JB*w*fsaEV_XiElxCzWG4s_AF2kv&*Y9)yLuQC<CBU=PXa2+#yu3vR-k{J_RkPZ
z)=fuX@7)gXA|x3haj$*<{{6mLBQxM{Z`7x^jD^0gnSdl6jS4i?b#(^f*ZSW}{18uE
z<OFROQG!xGt27>$ePN*igC3vVhyKPGfL+BiAOcB>ZDGsr%6c_2vu|t$COT_0+$}HO
zy~Bt*(~_D;&dM5o6KH4#E~4UaayDSMNq{f_slu|S-NOPf$TsM#rl<i27$w~7L-~D5
zDkNxLkWQ8rJL@gv$~j_zqr_8C#Mslg!Zxo_68_;t4yYKD0Fhx|r+?xKZ2qSn8lAZ4
zP0tA;?I=mGX`^KIazvibWYq0}I-|O87#T40`4Qjefl~f!Wo+6K;To()<5g2UnSC74
zlVf*c{uhb7=U9)C0p37Z27+Ka4Lh+3VkMjC$4u}~iXsT5B3hgP^EKA_T11M<2t1j*
zqlPeK>qIRv=HUVkX>bVC)i1EzJtTq?TptniPt>7<L05m+HZi<B?`aIB=!9aKOOkz?
zL6Js729ph#+P5#-u!+lu*4cA4CD-9VAx^OG$M_-n?vVaksrskFP8lGiz)nCxZ_;Xs
zL_cq&PFM-hKg$MBg+8pWU)lB_NEej^YH*rb=|Jelyd)?tkjGtwh93vhq06|Df7Nw9
zEx-vPxMZGuRoFHZ&tjLFbxI<y)D9eOgokmDEX5m(5rmy!G{wJvdoRGWf+>?D9(2|W
zI(*_UZv-&qLSMM&PJBo_g1{dR*_iQ|bqH5WZ)s`Kjk^i7OLc?b<b>>OzJ(3ZOjA3b
z=dh$&&{P=vpNad4zy^G0LZS_SNP<6q{ygQ}VMj%^ESuuq&vi%wB${T65p5XaOG&W6
zF<<|enTLjig>kg`ySC)e|GdB8*<<a`lmsL=^ci*y*SAPoC{7%+N-6Xn`sU^-+d)#&
z(ro8u+!eWGYe3Vy(KKzcVoRXKhfaw1o$G&Hzy+Fa`@J&9h#_nga7!S5EQhAq>*Qcd
zLef0+d+6czf`_-a_o3z(#esqi(3WN+FrmZ|%GBAsxFT6_a)y8hzX!9CEdRHy`S(B2
YW>Tga_b#pr`o#l`^h|ZDwVfXQ4-F(wegFUf

literal 0
HcmV?d00001

diff --git a/assets/pieces/bP.png b/assets/pieces/bP.png
new file mode 100644
index 0000000000000000000000000000000000000000..790d74f47fa14e92c66ce03073b55615afcfb077
GIT binary patch
literal 2941
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
E04=aeuK)l5

literal 0
HcmV?d00001

diff --git a/assets/pieces/bQ.png b/assets/pieces/bQ.png
new file mode 100644
index 0000000000000000000000000000000000000000..befb1ebf154f267f2ff6115a67bb5d0d9a4a53cf
GIT binary patch
literal 10142
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
R5Pabdkds!DDwl)>{~s)TY)=3H

literal 0
HcmV?d00001

diff --git a/assets/pieces/bR.png b/assets/pieces/bR.png
new file mode 100644
index 0000000000000000000000000000000000000000..d048e10eedec9ada887b981c2de4ed244168a233
GIT binary patch
literal 2478
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

literal 0
HcmV?d00001

diff --git a/assets/pieces/wB.png b/assets/pieces/wB.png
new file mode 100644
index 0000000000000000000000000000000000000000..1e194101dd175d357f7f7fe06cdfcb55acf06c42
GIT binary patch
literal 7997
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
IR<@7+A2JV;pa1{>

literal 0
HcmV?d00001

diff --git a/assets/pieces/wK.png b/assets/pieces/wK.png
new file mode 100644
index 0000000000000000000000000000000000000000..5540cabd623675dad5ea33ca0842e8710f0da858
GIT binary patch
literal 9715
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
RPjNvMU#Y$<l`(t&zW}w71QGxM

literal 0
HcmV?d00001

diff --git a/assets/pieces/wN.png b/assets/pieces/wN.png
new file mode 100644
index 0000000000000000000000000000000000000000..2e39b8b6f46ffd74069692d6a92f3ca0a73f8e45
GIT binary patch
literal 8861
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
WCIKTx+gk9v38bN-t6YY#eDgnw`nyg5

literal 0
HcmV?d00001

diff --git a/assets/pieces/wP.png b/assets/pieces/wP.png
new file mode 100644
index 0000000000000000000000000000000000000000..9fc16f116fdd75930db3883da4c472324420fa33
GIT binary patch
literal 5514
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

literal 0
HcmV?d00001

diff --git a/assets/pieces/wQ.png b/assets/pieces/wQ.png
new file mode 100644
index 0000000000000000000000000000000000000000..b886c4380fdc55a78feea6c7a78c7df841f1857b
GIT binary patch
literal 12845
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%0AQPfl$~#ar3=ibYI2X=5GRS
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
EKU*E2#sB~S

literal 0
HcmV?d00001

diff --git a/assets/pieces/wR.png b/assets/pieces/wR.png
new file mode 100644
index 0000000000000000000000000000000000000000..efe2110fd80b251bcdb89b3238bffc0dea28f2e1
GIT binary patch
literal 2646
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
C5@ap_

literal 0
HcmV?d00001

diff --git a/src/tui/mod.rs b/src/tui/mod.rs
index c902e0358c05dc442a15b62dae78f91ddb0a05cf..44902a976aa02cc8b758e78188de5c5eabef0039 100644
--- a/src/tui/mod.rs
+++ b/src/tui/mod.rs
@@ -14,6 +14,7 @@ pub mod glyphs;
 pub mod input;
 pub mod movetext;
 pub mod panels;
+pub mod pieces;
 pub mod terminal;
 #[cfg(test)]
 mod test_support;
diff --git a/src/tui/pieces.rs b/src/tui/pieces.rs
new file mode 100644
index 0000000000000000000000000000000000000000..7fed95e193bae1c482054e9877235bce9de6c603
--- /dev/null
+++ b/src/tui/pieces.rs
@@ -0,0 +1,272 @@
+//! Piece images for the Image glyph style (spec 9.3): the Cburnett PNGs from
+//! `assets/pieces/` embedded in the binary, [`composite`] to draw one piece onto an
+//! opaque square background of an exact pixel size, and [`ImageCache`] to keep what
+//! is built from those composites between frames.
+//!
+//! Nothing here touches the terminal: the cache is generic over its values, so the
+//! board keeps ratatui-image protocols in it and the tests keep plain numbers.
+
+#[cfg(test)]
+mod tests {
+    use std::cell::Cell;
+
+    use image::Rgba;
+
+    use super::*;
+    use crate::core::{Color as Side, PieceKind};
+
+    /// The truecolor palette's light and dark squares and its selection tint.
+    const LIGHT: [u8; 3] = [0xB5, 0x88, 0x63];
+    const DARK: [u8; 3] = [0x7A, 0x56, 0x34];
+    const SELECTED: [u8; 3] = [0x5E, 0x9B, 0x4A];
+
+    const WHITE_KING: Piece = Piece::new(Side::White, PieceKind::King);
+
+    fn all_pieces() -> impl Iterator<Item = Piece> {
+        Side::ALL
+            .into_iter()
+            .flat_map(|color| PieceKind::ALL.map(|kind| Piece::new(color, kind)))
+    }
+
+    fn opaque([r, g, b]: [u8; 3]) -> Rgba<u8> {
+        Rgba([r, g, b, 255])
+    }
+
+    /// Mean of the red, green and blue channels over the whole image.
+    fn mean_brightness(image: &RgbaImage) -> f64 {
+        let sum: u64 = image
+            .pixels()
+            .map(|p| u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]))
+            .sum();
+        sum as f64 / (3.0 * f64::from(image.width() * image.height()))
+    }
+
+    #[test]
+    fn embedded_images_decode_once_to_square_rgba() {
+        for piece in all_pieces() {
+            let image = source(piece);
+            assert_eq!(image.dimensions(), (SOURCE_SIZE, SOURCE_SIZE), "{piece:?}");
+            assert_eq!(
+                image.get_pixel(0, 0)[3],
+                0,
+                "{piece:?} corner is not transparent"
+            );
+            assert!(
+                image.pixels().any(|p| p[3] == 255),
+                "{piece:?} has no opaque pixel"
+            );
+            assert!(
+                std::ptr::eq(image, source(piece)),
+                "{piece:?} decoded twice"
+            );
+        }
+    }
+
+    #[test]
+    fn every_piece_has_its_own_image() {
+        let pieces: Vec<Piece> = all_pieces().collect();
+        for (i, a) in pieces.iter().enumerate() {
+            for b in &pieces[i + 1..] {
+                assert!(source(*a) != source(*b), "{a:?} and {b:?} share an image");
+            }
+        }
+    }
+
+    #[test]
+    fn composite_has_exactly_the_requested_size() {
+        for (width, height) in [
+            (48, 48),
+            (50, 60),
+            (60, 20),
+            (20, 60),
+            (1, 1),
+            (3, 7),
+            (256, 256),
+            (512, 300),
+        ] {
+            for piece in [WHITE_KING, Piece::new(Side::Black, PieceKind::Knight)] {
+                let image = composite(piece, LIGHT, width, height);
+                assert_eq!(image.dimensions(), (width, height), "{piece:?}");
+            }
+        }
+    }
+
+    #[test]
+    fn composite_is_opaque_with_the_background_in_the_corners() {
+        for piece in all_pieces() {
+            for background in [LIGHT, DARK, SELECTED] {
+                for (width, height) in [(48, 48), (50, 60), (60, 20), (20, 60), (160, 160)] {
+                    let image = composite(piece, background, width, height);
+                    assert!(
+                        image.pixels().all(|p| p[3] == 255),
+                        "{piece:?} {width}x{height} has a see-through pixel"
+                    );
+                    for (x, y) in [
+                        (0, 0),
+                        (width - 1, 0),
+                        (0, height - 1),
+                        (width - 1, height - 1),
+                    ] {
+                        assert_eq!(
+                            *image.get_pixel(x, y),
+                            opaque(background),
+                            "{piece:?} {width}x{height} corner ({x}, {y})"
+                        );
+                    }
+                }
+            }
+        }
+    }
+
+    #[test]
+    fn composite_draws_the_piece_in_the_centre() {
+        for piece in all_pieces() {
+            for background in [LIGHT, DARK, SELECTED] {
+                for (width, height) in [(48, 48), (50, 60), (90, 30)] {
+                    let image = composite(piece, background, width, height);
+                    assert_ne!(
+                        *image.get_pixel(width / 2, height / 2),
+                        opaque(background),
+                        "{piece:?} {width}x{height} centre shows the background"
+                    );
+                }
+            }
+        }
+    }
+
+    #[test]
+    fn composite_keeps_the_aspect_ratio_and_centres_the_piece() {
+        let wide = composite(WHITE_KING, DARK, 90, 30);
+        for (x, y, pixel) in wide.enumerate_pixels() {
+            if !(30..60).contains(&x) {
+                assert_eq!(
+                    *pixel,
+                    opaque(DARK),
+                    "wide: ({x}, {y}) is outside the piece"
+                );
+            }
+        }
+        let tall = composite(WHITE_KING, DARK, 30, 90);
+        for (x, y, pixel) in tall.enumerate_pixels() {
+            if !(30..60).contains(&y) {
+                assert_eq!(
+                    *pixel,
+                    opaque(DARK),
+                    "tall: ({x}, {y}) is outside the piece"
+                );
+            }
+        }
+        // The fitted piece is the same picture either way.
+        let square = composite(WHITE_KING, DARK, 30, 30);
+        for (x, y, pixel) in square.enumerate_pixels() {
+            assert_eq!(wide.get_pixel(x + 30, y), pixel);
+            assert_eq!(tall.get_pixel(x, y + 30), pixel);
+        }
+    }
+
+    #[test]
+    fn white_pieces_are_lighter_than_black_pieces() {
+        for kind in PieceKind::ALL {
+            let white = composite(Piece::new(Side::White, kind), LIGHT, 64, 64);
+            let black = composite(Piece::new(Side::Black, kind), LIGHT, 64, 64);
+            assert!(white != black, "{kind:?}");
+            assert!(
+                mean_brightness(&white) > mean_brightness(&black) + 10.0,
+                "{kind:?}: white {} vs black {}",
+                mean_brightness(&white),
+                mean_brightness(&black)
+            );
+        }
+    }
+
+    #[test]
+    fn composite_is_deterministic() {
+        for piece in all_pieces() {
+            assert!(
+                composite(piece, LIGHT, 50, 60) == composite(piece, LIGHT, 50, 60),
+                "{piece:?}"
+            );
+        }
+    }
+
+    #[test]
+    fn zero_sizes_give_empty_images() {
+        for (width, height) in [(0, 0), (0, 40), (40, 0)] {
+            let image = composite(WHITE_KING, LIGHT, width, height);
+            assert_eq!(image.dimensions(), (width, height));
+        }
+    }
+
+    #[test]
+    fn a_key_composites_its_own_fields() {
+        let key = ImageKey::new(WHITE_KING, SELECTED, 50, 60);
+        assert_eq!(key.piece, WHITE_KING);
+        assert_eq!(key.background, SELECTED);
+        assert_eq!((key.width_px, key.height_px), (50, 60));
+        assert!(key.composite() == composite(WHITE_KING, SELECTED, 50, 60));
+    }
+
+    #[test]
+    fn cache_builds_each_key_once() {
+        let mut cache = ImageCache::new();
+        assert!(cache.is_empty());
+        let calls = Cell::new(0);
+        let make = |key: &ImageKey| {
+            calls.set(calls.get() + 1);
+            (key.width_px, key.height_px)
+        };
+
+        let key = ImageKey::new(WHITE_KING, LIGHT, 50, 60);
+        assert_eq!(cache.get(&key), None);
+        assert_eq!(*cache.get_or_insert_with(key, make), (50, 60));
+        assert_eq!(*cache.get_or_insert_with(key, make), (50, 60));
+        assert_eq!(calls.get(), 1, "the second lookup was a miss");
+        assert_eq!(cache.get(&key), Some(&(50, 60)));
+        assert_eq!(cache.len(), 1);
+    }
+
+    #[test]
+    fn cache_keys_on_piece_background_and_pixel_size() {
+        let mut cache = ImageCache::new();
+        let calls = Cell::new(0);
+        let make = |_: &ImageKey| calls.set(calls.get() + 1);
+
+        let key = ImageKey::new(WHITE_KING, LIGHT, 50, 60);
+        let variants = [
+            key,
+            ImageKey::new(Piece::new(Side::Black, PieceKind::King), LIGHT, 50, 60),
+            ImageKey::new(Piece::new(Side::White, PieceKind::Queen), LIGHT, 50, 60),
+            ImageKey::new(WHITE_KING, DARK, 50, 60),
+            ImageKey::new(WHITE_KING, SELECTED, 50, 60),
+            ImageKey::new(WHITE_KING, LIGHT, 51, 60),
+            ImageKey::new(WHITE_KING, LIGHT, 50, 61),
+        ];
+        for variant in variants {
+            cache.get_or_insert_with(variant, make);
+        }
+        assert_eq!(calls.get(), variants.len());
+        assert_eq!(cache.len(), variants.len());
+
+        // A piece that moves to another square of the same colour reuses its entry.
+        cache.get_or_insert_with(ImageKey::new(WHITE_KING, LIGHT, 50, 60), make);
+        assert_eq!(calls.get(), variants.len());
+    }
+
+    #[test]
+    fn clear_empties_the_cache() {
+        let mut cache = ImageCache::default();
+        let calls = Cell::new(0);
+        let make = |_: &ImageKey| calls.set(calls.get() + 1);
+        let key = ImageKey::new(WHITE_KING, LIGHT, 50, 60);
+
+        cache.get_or_insert_with(key, make);
+        cache.get_or_insert_with(ImageKey::new(WHITE_KING, DARK, 50, 60), make);
+        assert_eq!(cache.len(), 2);
+
+        cache.clear();
+        assert!(cache.is_empty());
+        assert_eq!(cache.get(&key), None);
+        cache.get_or_insert_with(key, make);
+        assert_eq!(calls.get(), 3, "a cleared entry was still found");
+    }
+}
````

- [ ] **Step: Run the tests to verify they fail**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::pieces`
Expected: FAIL. The test build does not compile: the tests use names the implementation patch adds, for example "cannot find function composite in this scope"; "cannot find function source in this scope"; "cannot find type ImageKey in this scope"; "cannot find type Piece in this scope". Any other failure (a patch that does not apply, a test that fails at run time) is not the expected RED: stop and report.

- [ ] **Step: Apply the implementation patch**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/tui/pieces.rs b/src/tui/pieces.rs
index 7fed95e193bae1c482054e9877235bce9de6c603..f5b200e26a5928cf4866ae26f05dd97384d2a75e 100644
--- a/src/tui/pieces.rs
+++ b/src/tui/pieces.rs
@@ -1,11 +1,214 @@
 //! Piece images for the Image glyph style (spec 9.3): the Cburnett PNGs from
 //! `assets/pieces/` embedded in the binary, [`composite`] to draw one piece onto an
-//! opaque square background of an exact pixel size, and [`ImageCache`] to keep what
-//! is built from those composites between frames.
+//! opaque background of an exact pixel size, and [`ImageCache`] to keep what is
+//! built from those composites between frames.
 //!
 //! Nothing here touches the terminal: the cache is generic over its values, so the
 //! board keeps ratatui-image protocols in it and the tests keep plain numbers.
 
+use std::collections::HashMap;
+use std::sync::OnceLock;
+
+use image::imageops::{self, FilterType};
+use image::{ImageFormat, Rgba, RgbaImage};
+
+use crate::core::Piece;
+
+/// Width and height in pixels of every embedded piece image.
+pub const SOURCE_SIZE: u32 = 256;
+
+/// The embedded PNGs (see `assets/pieces/README.md`), indexed by
+/// `Color::index` and then `PieceKind::index`: pawn, knight, bishop, rook,
+/// queen, king.
+const PNGS: [[&[u8]; 6]; 2] = [
+    [
+        include_bytes!("../../assets/pieces/wP.png"),
+        include_bytes!("../../assets/pieces/wN.png"),
+        include_bytes!("../../assets/pieces/wB.png"),
+        include_bytes!("../../assets/pieces/wR.png"),
+        include_bytes!("../../assets/pieces/wQ.png"),
+        include_bytes!("../../assets/pieces/wK.png"),
+    ],
+    [
+        include_bytes!("../../assets/pieces/bP.png"),
+        include_bytes!("../../assets/pieces/bN.png"),
+        include_bytes!("../../assets/pieces/bB.png"),
+        include_bytes!("../../assets/pieces/bR.png"),
+        include_bytes!("../../assets/pieces/bQ.png"),
+        include_bytes!("../../assets/pieces/bK.png"),
+    ],
+];
+
+/// The decoded images, laid out like [`PNGS`] and each filled on first use.
+static DECODED: [[OnceLock<RgbaImage>; 6]; 2] = [const { [const { OnceLock::new() }; 6] }; 2];
+
+/// The image of `piece`: [`SOURCE_SIZE`] pixels square, RGBA, transparent around
+/// the piece. It is decoded on first use and kept for the life of the process.
+fn source(piece: Piece) -> &'static RgbaImage {
+    let (color, kind) = (piece.color.index(), piece.kind.index());
+    DECODED[color][kind].get_or_init(|| {
+        image::load_from_memory_with_format(PNGS[color][kind], ImageFormat::Png)
+            .expect("the embedded piece images are valid PNGs")
+            .into_rgba8()
+    })
+}
+
+/// `piece` drawn on a solid `background` (RGB) of exactly `width_px` × `height_px`
+/// pixels, every one of them opaque.
+///
+/// The piece is scaled with [`FilterType::Lanczos3`] to the largest size that fits
+/// with its aspect ratio kept, and centred; the rest of the longer side is
+/// background. It is blended onto the background before it is scaled, so its
+/// anti-aliased edges are scaled against the colour they are shown on, and nothing
+/// depends on how a terminal treats transparency. A zero width or height gives an
+/// empty image.
+#[must_use]
+pub fn composite(piece: Piece, background: [u8; 3], width_px: u32, height_px: u32) -> RgbaImage {
+    let [r, g, b] = background;
+    let mut canvas = RgbaImage::from_pixel(width_px, height_px, Rgba([r, g, b, 255]));
+    let source = source(piece);
+    let (fit_w, fit_h) = fit(source.dimensions(), (width_px, height_px));
+    if fit_w == 0 || fit_h == 0 {
+        return canvas;
+    }
+
+    let mut flat = source.clone();
+    for pixel in flat.pixels_mut() {
+        *pixel = alpha_over(background, *pixel);
+    }
+    let scaled = imageops::resize(&flat, fit_w, fit_h, FilterType::Lanczos3);
+    let x = (width_px - fit_w) / 2;
+    let y = (height_px - fit_h) / 2;
+    imageops::replace(&mut canvas, &scaled, i64::from(x), i64::from(y));
+    canvas
+}
+
+/// The largest size with the aspect ratio of `source` that fits in `area`, each
+/// side rounded to the nearest pixel and at least 1; `(0, 0)` when either has a
+/// zero side.
+fn fit((src_w, src_h): (u32, u32), (area_w, area_h): (u32, u32)) -> (u32, u32) {
+    if src_w == 0 || src_h == 0 || area_w == 0 || area_h == 0 {
+        return (0, 0);
+    }
+    let (src_w, src_h) = (u64::from(src_w), u64::from(src_h));
+    // Rounds `a × b / c` and keeps it within 1..=max, so it fits in a u32.
+    let scale = |a: u64, b: u32, c: u64, max: u32| {
+        let scaled = (a * u64::from(b) + c / 2) / c;
+        scaled.clamp(1, u64::from(max)) as u32
+    };
+    if src_w * u64::from(area_h) <= src_h * u64::from(area_w) {
+        // The height limits the size.
+        (scale(src_w, area_h, src_h, area_w), area_h)
+    } else {
+        (area_w, scale(src_h, area_w, src_w, area_h))
+    }
+}
+
+/// `pixel` blended over the opaque colour `background` with its straight alpha, in
+/// sRGB values as terminals and image viewers blend them. The result is opaque.
+fn alpha_over(background: [u8; 3], pixel: Rgba<u8>) -> Rgba<u8> {
+    let alpha = u16::from(pixel[3]);
+    // At most (255 × 255 + 127) / 255 = 255, so the result fits in a u8.
+    let mix = |fg: u8, bg: u8| {
+        ((u16::from(fg) * alpha + u16::from(bg) * (255 - alpha) + 127) / 255) as u8
+    };
+    Rgba([
+        mix(pixel[0], background[0]),
+        mix(pixel[1], background[1]),
+        mix(pixel[2], background[2]),
+        255,
+    ])
+}
+
+/// Everything a piece composite depends on, and so the key of [`ImageCache`].
+#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
+pub struct ImageKey {
+    /// The piece drawn.
+    pub piece: Piece,
+    /// The square's background colour behind the piece, as RGB: light, dark, or a
+    /// highlight tint.
+    pub background: [u8; 3],
+    /// Width of the image in pixels.
+    pub width_px: u32,
+    /// Height of the image in pixels.
+    pub height_px: u32,
+}
+
+impl ImageKey {
+    /// The key for `piece` on `background` at `width_px` × `height_px` pixels.
+    pub const fn new(piece: Piece, background: [u8; 3], width_px: u32, height_px: u32) -> Self {
+        ImageKey {
+            piece,
+            background,
+            width_px,
+            height_px,
+        }
+    }
+
+    /// The [`composite`] this key describes.
+    #[must_use]
+    pub fn composite(&self) -> RgbaImage {
+        composite(self.piece, self.background, self.width_px, self.height_px)
+    }
+}
+
+/// Values built from piece composites, one per [`ImageKey`], so a piece is scaled
+/// and encoded for the terminal once rather than every frame. The board keeps
+/// ratatui-image protocols here; a piece that moves to a square with the same
+/// background reuses its entry.
+///
+/// The cache never evicts on its own. Its owner calls [`ImageCache::clear`] when
+/// the square size or the font size changes, which keeps it to one entry per piece
+/// and background in use.
+#[derive(Debug)]
+pub struct ImageCache<T> {
+    entries: HashMap<ImageKey, T>,
+}
+
+impl<T> ImageCache<T> {
+    /// An empty cache.
+    pub fn new() -> Self {
+        ImageCache {
+            entries: HashMap::new(),
+        }
+    }
+
+    /// The value stored for `key`, if any.
+    pub fn get(&self, key: &ImageKey) -> Option<&T> {
+        self.entries.get(key)
+    }
+
+    /// The value for `key`, first built with `make` and stored when it is missing.
+    pub fn get_or_insert_with(
+        &mut self,
+        key: ImageKey,
+        make: impl FnOnce(&ImageKey) -> T,
+    ) -> &mut T {
+        self.entries.entry(key).or_insert_with_key(make)
+    }
+
+    /// Number of stored values.
+    pub fn len(&self) -> usize {
+        self.entries.len()
+    }
+
+    /// True when nothing is stored.
+    pub fn is_empty(&self) -> bool {
+        self.entries.is_empty()
+    }
+
+    /// Drops every stored value.
+    pub fn clear(&mut self) {
+        self.entries.clear();
+    }
+}
+
+impl<T> Default for ImageCache<T> {
+    fn default() -> Self {
+        ImageCache::new()
+    }
+}
+
 #[cfg(test)]
 mod tests {
     use std::cell::Cell;
````

- [ ] **Step: Run the tests to verify they pass**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::` and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test --lib engine::`
Expected: tui:: 306 passed, engine:: 96 passed; `find src -name '*.snap.new'` prints nothing.

Run: `cargo fmt --check` and `cargo clippy --all-targets 2>&1 | grep -A5 -E "src/tui|src/engine|src/main.rs"`
Expected: no output from either.

- [ ] **Step: Update the handoff file**

Edit `docs/handoff/HANDOFF.md`. In `## Current`, set:

```
Sub-project: tui-polish | Plan: docs/superpowers/plans/2026-09-27-tui-polish.md
Branch: feat/tui-polish
Last completed task: tui-polish task 1 (Piece images, dependencies and the picture compositor)
Next task: tui-polish task 2 (engine exchange recording)
State: green (except 3 pre-existing old-code test failures)
```

Set `## Verify before continuing` to:

```
env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui:: && cargo test --lib engine::
```

Add at the top of `## Log (newest first)`: `- <today's date> tui-polish task 1 done: Piece images, dependencies and the picture compositor`. Keep every other section. Record under `## Notes / decisions made during work` any decision this plan did not specify.

- [ ] **Step: Refresh the knowledge graph**

Run: `graphify update .`
Expected: completes without error. Never commit `graphify-out/`, `.claude/` or `.superpowers/`.

- [ ] **Step: Commit**

```bash
git add Cargo.lock Cargo.toml assets/pieces/LICENSE assets/pieces/README.md assets/pieces/bB.png assets/pieces/bK.png assets/pieces/bN.png assets/pieces/bP.png assets/pieces/bQ.png assets/pieces/bR.png assets/pieces/wB.png assets/pieces/wK.png assets/pieces/wN.png assets/pieces/wP.png assets/pieces/wQ.png assets/pieces/wR.png docs/handoff/HANDOFF.md src/tui/mod.rs src/tui/pieces.rs
git commit -m "feat(tui): embed the Cburnett piece images and composite them onto square colours

Co-Authored-By: <the model you are> <noreply@anthropic.com>"
```

Replace `<the model you are>` with your own model name.

---

### Task 2: Engine: record the Jev exchange for debug mode

**Files:**
- Modify: `src/engine/config.rs`, `src/engine/jev.rs`, `src/engine/mod.rs`, `src/engine/player.rs`, `src/tui/app.rs`, `src/tui/panels.rs`, `src/tui/test_support/engine.rs`, `src/tui/worker.rs`

**Interfaces:**
- Consumes: `engine::jev::{ChoiceRequest, ChoiceAnswer, JevError, MoveChooser, JevClient}`, `engine::player::ComputerMove`, `engine::config::EngineConfig`.
- Produces (`engine::jev`, re-exported from `engine`): `JevExchange { method: String, url: String, headers: Vec<(String, String)>, body: serde_json::Value, attempts: Vec<JevAttempt> }`, `JevAttempt { status: Option<u16>, response: Option<String>, error: Option<String>, elapsed: Duration }`; `MoveChooser::choose_traced(&self, request: &ChoiceRequest, trace: &mut Option<JevExchange>) -> Result<ChoiceAnswer, JevError>` (default calls `choose`); `JevClient` overrides it and records every attempt with the key redacted. `EngineConfig.trace: bool` (default false, never read from the environment); `ComputerMove.exchange: Option<Box<JevExchange>>`.
- Existing `ComputerMove` literals in TUI tests gain `exchange: None` (in the implementation patch).

- [ ] **Step: Confirm the starting point**

Run: `git status --short` (only untracked `.claude/`, `graphify-out/` and user files) and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: 306 passed (engine: `cargo test --lib engine::` 96 passed).

- [ ] **Step: Apply the tests patch (write the failing tests)**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/engine/config.rs b/src/engine/config.rs
index 146eb607d39adff67905dd8709783c2ccb6d6b30..4d25ea64d28ff6c0086677736aebbb79d6efc0ec 100644
--- a/src/engine/config.rs
+++ b/src/engine/config.rs
@@ -207,6 +207,24 @@ mod tests {
         );
     }
 
+    #[test]
+    fn trace_is_off_and_never_read_from_the_environment() {
+        assert!(!EngineConfig::default().trace);
+        // The TUI sets `trace` from its own flag; no variable turns it on here.
+        let c = config(&[
+            ("RCHESS_DEBUG", "1"),
+            ("JEV_TRACE", "1"),
+            ("JEV_DEBUG", "1"),
+        ]);
+        assert!(!c.trace);
+        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
+        let traced = EngineConfig {
+            trace: true,
+            ..EngineConfig::default()
+        };
+        assert!(format!("{traced:?}").contains("trace: true"));
+    }
+
     #[test]
     fn debug_hides_the_key() {
         let c = config(&[("JEV_API_KEY", "secret-key-123")]);
diff --git a/src/engine/jev.rs b/src/engine/jev.rs
index 504353dcf74dd6a163c8d42629be988d04c5a767..967e4d80fdc6000a7efcfd08297d6e056af8e312 100644
--- a/src/engine/jev.rs
+++ b/src/engine/jev.rs
@@ -514,6 +514,11 @@ mod tests {
     /// Serves `responses` in order, one per connection, on 127.0.0.1, then stops
     /// listening. Returns a client pointed at its `/v1/systemone` and the requests read.
     fn serve(responses: Vec<String>) -> (JevClient, Requests) {
+        serve_with_key(responses, "test-key")
+    }
+
+    /// [`serve`] with a client that sends `api_key`.
+    fn serve_with_key(responses: Vec<String>, api_key: &str) -> (JevClient, Requests) {
         let listener = TcpListener::bind("127.0.0.1:0").unwrap();
         let port = listener.local_addr().unwrap().port();
         let requests = Requests::default();
@@ -532,16 +537,18 @@ mod tests {
                 // Dropping the stream closes the connection.
             }
         });
+        (client_for_port(port, api_key), requests)
+    }
+
+    /// A client for `http://127.0.0.1:<port>/v1/systemone` sending `api_key`.
+    fn client_for_port(port: u16, api_key: &str) -> JevClient {
         let config = EngineConfig {
-            api_key: Some("test-key".to_string()),
+            api_key: Some(api_key.to_string()),
             timeout: Duration::from_secs(2),
             ..EngineConfig::default()
         };
         let endpoint = format!("http://127.0.0.1:{port}/v1/systemone");
-        (
-            JevClient::with_endpoint(&config, endpoint).unwrap(),
-            requests,
-        )
+        JevClient::with_endpoint(&config, endpoint).unwrap()
     }
 
     fn count(requests: &Requests) -> usize {
@@ -694,6 +701,183 @@ mod tests {
         assert_eq!(count(&requests), 1);
     }
 
+    /// A successful answer for [`request`]: Jev picks O-O.
+    const ANSWER: &str = r#"{"model":"jev-1.13.0","answers":{"move":{"type":"choice","choice":"O-O","probabilities":{"Nxe5":0.25,"O-O":0.75},"confidence":0.6}},"usage":{"input_tokens":321,"output_tokens":9}}"#;
+
+    /// An API key the redaction tests look for in everything the client records.
+    const SENTINEL_KEY: &str = "sentinel-key-7Qx9";
+
+    #[test]
+    fn traced_exchange_records_the_request_and_every_attempt() {
+        let busy = response("503 Service Unavailable", "text/plain", "busy, try again");
+        let ok = response("200 OK", "application/json", ANSWER);
+        let (client, requests) = serve(vec![busy, ok]);
+        let mut trace = None;
+        let started = std::time::Instant::now();
+        let answer = client.choose_traced(&request(), &mut trace).unwrap();
+        let total = started.elapsed();
+        assert_eq!(answer.choice, "O-O");
+        assert_eq!(count(&requests), 2);
+
+        let exchange = trace.expect("the client records the exchange");
+        assert_eq!(exchange.method, "POST");
+        assert!(
+            exchange.url.starts_with("http://127.0.0.1:"),
+            "{}",
+            exchange.url
+        );
+        assert!(exchange.url.ends_with("/v1/systemone"), "{}", exchange.url);
+        assert_eq!(
+            exchange.headers,
+            vec![
+                ("Authorization".to_string(), "Bearer <redacted>".to_string()),
+                ("Content-Type".to_string(), "application/json".to_string()),
+            ]
+        );
+        assert_eq!(exchange.body, request().to_body("jev-latest"));
+        let [busy, ok] = exchange.attempts.as_slice() else {
+            panic!("expected two attempts, got {:?}", exchange.attempts);
+        };
+        assert_eq!(busy.status, Some(503));
+        assert_eq!(busy.response.as_deref(), Some("busy, try again"));
+        assert_eq!(busy.error.as_deref(), Some("HTTP 503: busy, try again"));
+        assert_eq!(ok.status, Some(200));
+        assert_eq!(ok.response.as_deref(), Some(ANSWER));
+        assert_eq!(ok.error, None);
+        // Each attempt times its own request, not the 250 ms backoff between them.
+        assert!(
+            busy.elapsed + ok.elapsed + BASE_BACKOFF <= total,
+            "{:?} + {:?} + backoff > {total:?}",
+            busy.elapsed,
+            ok.elapsed
+        );
+
+        // Only the record is redacted: the wire carries the real key.
+        for raw in requests.lock().unwrap().iter() {
+            assert!(raw.contains("Bearer test-key"), "{raw}");
+        }
+    }
+
+    #[test]
+    fn traced_exchange_records_a_failed_connection_without_a_status() {
+        // Nothing listens on the port once the listener is dropped.
+        let port = {
+            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
+            listener.local_addr().unwrap().port()
+        };
+        let client = client_for_port(port, "test-key");
+        let mut trace = None;
+        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
+        assert!(matches!(error, JevError::Transport(_)), "{error:?}");
+        let exchange = trace.expect("the client records a failed exchange too");
+        assert_eq!(exchange.attempts.len(), 3, "{:?}", exchange.attempts);
+        for attempt in &exchange.attempts {
+            assert_eq!(attempt.status, None);
+            assert_eq!(attempt.response, None);
+            let text = attempt.error.as_deref().expect("an error for each attempt");
+            assert!(text.starts_with("network error: "), "{text}");
+        }
+    }
+
+    #[test]
+    fn traced_exchange_redacts_a_key_the_server_echoes() {
+        let echo = format!(r#"{{"error":"invalid api key {SENTINEL_KEY}"}}"#);
+        // The key straddles the 200-character cut of the error snippet.
+        let long = format!("{}{SENTINEL_KEY}", "x".repeat(195));
+        let (client, requests) = serve_with_key(
+            vec![
+                response("401 Unauthorized", "application/json", &echo),
+                // serde_json quotes the offending string in its error.
+                response("200 OK", "application/json", &format!("\"{SENTINEL_KEY}\"")),
+                response("422 Unprocessable Entity", "text/plain", &long),
+            ],
+            SENTINEL_KEY,
+        );
+
+        let mut trace = None;
+        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
+        let exchange = trace.unwrap();
+        assert_eq!(
+            exchange.attempts[0].response.as_deref(),
+            Some(r#"{"error":"invalid api key <redacted>"}"#)
+        );
+        assert_eq!(
+            exchange.attempts[0].error.as_deref(),
+            Some(r#"HTTP 401: {"error":"invalid api key <redacted>"}"#)
+        );
+        assert_eq!(exchange.headers[0].1, "Bearer <redacted>");
+        let mut texts = vec![error.to_string(), format!("{exchange:?}")];
+
+        let mut trace = None;
+        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
+        assert!(matches!(error, JevError::InvalidResponse(_)), "{error:?}");
+        let exchange = trace.unwrap();
+        assert_eq!(
+            exchange.attempts[0].response.as_deref(),
+            Some("\"<redacted>\"")
+        );
+        texts.extend([error.to_string(), format!("{exchange:?}")]);
+
+        let mut trace = None;
+        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
+        let JevError::Http { message, .. } = &error else {
+            panic!("expected an HTTP error, got {error:?}");
+        };
+        assert_eq!(*message, format!("{}<reda", "x".repeat(195)));
+        texts.extend([error.to_string(), format!("{:?}", trace.unwrap())]);
+
+        assert_eq!(count(&requests), 3);
+        for text in &texts {
+            assert!(!text.contains(&SENTINEL_KEY[..8]), "{text}");
+        }
+        // The server did receive the key.
+        assert!(requests.lock().unwrap()[0].contains(SENTINEL_KEY));
+    }
+
+    #[test]
+    fn untraced_errors_redact_an_echoed_key_too() {
+        let echo = format!("no such key: {SENTINEL_KEY}");
+        let (client, _requests) = serve_with_key(
+            vec![response("401 Unauthorized", "text/plain", &echo)],
+            SENTINEL_KEY,
+        );
+        let error = client.choose(&request()).unwrap_err();
+        assert_eq!(error.to_string(), "HTTP 401: no such key: <redacted>");
+    }
+
+    #[test]
+    fn redaction_covers_every_string_in_a_json_body() {
+        let body = json!({
+            "model": format!("m-{SENTINEL_KEY}"),
+            "nested": [{ SENTINEL_KEY: [SENTINEL_KEY, 1, true, null] }],
+        });
+        assert_eq!(
+            redact_value(&body, SENTINEL_KEY),
+            json!({
+                "model": "m-<redacted>",
+                "nested": [{ "<redacted>": ["<redacted>", 1, true, null] }],
+            })
+        );
+        assert_eq!(redact("a key b key", "key"), "a <redacted> b <redacted>");
+        assert_eq!(redact("text", ""), "text", "an empty key redacts nothing");
+    }
+
+    #[test]
+    fn choose_traced_by_default_answers_without_a_trace() {
+        struct Plain;
+        impl MoveChooser for Plain {
+            fn choose(&self, _: &ChoiceRequest) -> Result<ChoiceAnswer, JevError> {
+                Err(JevError::Timeout)
+            }
+        }
+        let mut trace = None;
+        assert_eq!(
+            Plain.choose_traced(&request(), &mut trace),
+            Err(JevError::Timeout)
+        );
+        assert_eq!(trace, None);
+    }
+
     #[test]
     fn client_needs_a_key_and_hides_it() {
         assert!(JevClient::new(&EngineConfig::default()).is_none());
diff --git a/src/engine/player.rs b/src/engine/player.rs
index fc49989c76dcc014a77641d4e8e8706fbb1ffa91..0ed797b1c8ac4f19a537f4dc7ceacb4ba7de1c25 100644
--- a/src/engine/player.rs
+++ b/src/engine/player.rs
@@ -245,8 +245,10 @@ fn shortlist(
 #[cfg(test)]
 mod tests {
     use super::*;
-    use crate::engine::jev::{ChoiceAnswer, JevError, http_error};
+    use crate::engine::jev::{ChoiceAnswer, JevAttempt, JevError, JevExchange, http_error};
+    use serde_json::json;
     use std::sync::Mutex;
+    use std::sync::atomic::{AtomicUsize, Ordering};
 
     /// What the mock returns: a fixed answer or an error.
     enum Reply {
@@ -260,6 +262,8 @@ mod tests {
     struct MockChooser {
         reply: Reply,
         seen: Mutex<Vec<ChoiceRequest>>,
+        /// How many of the requests came through `choose_traced`.
+        traced: AtomicUsize,
     }
 
     impl MockChooser {
@@ -270,6 +274,7 @@ mod tests {
                     probabilities,
                 },
                 seen: Mutex::new(Vec::new()),
+                traced: AtomicUsize::new(0),
             }
         }
 
@@ -277,12 +282,33 @@ mod tests {
             MockChooser {
                 reply: Reply::Error(error),
                 seen: Mutex::new(Vec::new()),
+                traced: AtomicUsize::new(0),
             }
         }
 
         fn requests(&self) -> Vec<ChoiceRequest> {
             self.seen.lock().unwrap().clone()
         }
+
+        fn traced_calls(&self) -> usize {
+            self.traced.load(Ordering::SeqCst)
+        }
+    }
+
+    /// The exchange every traced mock call records, answer or error.
+    fn mock_exchange() -> JevExchange {
+        JevExchange {
+            method: "POST".to_string(),
+            url: "http://mock/v1/systemone".to_string(),
+            headers: vec![("Authorization".to_string(), "Bearer <redacted>".to_string())],
+            body: json!({ "model": "jev-latest" }),
+            attempts: vec![JevAttempt {
+                status: Some(200),
+                response: Some("{}".to_string()),
+                error: None,
+                elapsed: Duration::from_millis(7),
+            }],
+        }
     }
 
     impl MoveChooser for MockChooser {
@@ -305,12 +331,40 @@ mod tests {
                 Reply::Error(error) => Err(error.clone()),
             }
         }
+
+        fn choose_traced(
+            &self,
+            request: &ChoiceRequest,
+            trace: &mut Option<JevExchange>,
+        ) -> Result<ChoiceAnswer, JevError> {
+            self.traced.fetch_add(1, Ordering::SeqCst);
+            *trace = Some(mock_exchange());
+            self.choose(request)
+        }
+    }
+
+    /// A chooser that keeps the default `choose_traced`, which records nothing.
+    struct Untraced(MockChooser);
+
+    impl MoveChooser for Untraced {
+        fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceAnswer, JevError> {
+            self.0.choose(request)
+        }
     }
 
     fn player(mock: MockChooser) -> ComputerPlayer<MockChooser> {
         ComputerPlayer::new(Some(mock), EngineConfig::default())
     }
 
+    /// A player with `trace` on, as the TUI builds it in debug mode.
+    fn traced_player(mock: MockChooser) -> ComputerPlayer<MockChooser> {
+        let config = EngineConfig {
+            trace: true,
+            ..EngineConfig::default()
+        };
+        ComputerPlayer::new(Some(mock), config)
+    }
+
     fn game(fen: &str) -> Game {
         Game::from_fen(fen).unwrap()
     }
@@ -603,4 +657,99 @@ mod tests {
         assert_eq!(result.source, MoveSource::Jev);
         assert_eq!(result.top, vec![("Kd2".to_string(), 0.4)]);
     }
+
+    #[test]
+    fn trace_attaches_the_exchange_to_a_jev_move() {
+        let p = traced_player(MockChooser::answering("Kd2", vec![("Kd2", 1.0)]));
+        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
+        assert_eq!(result.source, MoveSource::Jev);
+        assert_eq!(result.exchange, Some(Box::new(mock_exchange())));
+        assert_eq!(p.chooser.as_ref().unwrap().traced_calls(), 1);
+    }
+
+    #[test]
+    fn trace_attaches_the_exchange_to_a_vetoed_move() {
+        let config = EngineConfig {
+            filter_losing: false,
+            trace: true,
+            ..EngineConfig::default()
+        };
+        let mock = MockChooser::answering("Qxd5", vec![("Qxd5", 0.7), ("Kd2", 0.3)]);
+        let p = ComputerPlayer::new(Some(mock), config);
+        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
+        assert!(matches!(result.source, MoveSource::Vetoed { .. }));
+        assert_eq!(result.exchange, Some(Box::new(mock_exchange())));
+    }
+
+    #[test]
+    fn trace_attaches_the_exchange_to_a_fallback_after_jev_was_asked() {
+        let p = traced_player(MockChooser::failing(http_error(503, "overloaded", None)));
+        let result = p.choose_move(&Game::new()).unwrap();
+        assert_eq!(result.source, MoveSource::Fallback);
+        assert!(result.note.unwrap().contains("HTTP 503"));
+        assert_eq!(result.exchange, Some(Box::new(mock_exchange())));
+
+        let p = traced_player(MockChooser::answering("Qh8", vec![("Qh8", 1.0)]));
+        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
+        assert_eq!(result.source, MoveSource::Fallback);
+        assert!(result.note.unwrap().contains("unknown option"));
+        assert_eq!(
+            result.exchange,
+            Some(Box::new(mock_exchange())),
+            "Jev was asked, so an unusable answer is recorded too"
+        );
+    }
+
+    #[test]
+    fn no_exchange_without_trace() {
+        let p = player(MockChooser::answering("Kd2", vec![("Kd2", 1.0)]));
+        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
+        assert_eq!(result.source, MoveSource::Jev);
+        assert_eq!(result.exchange, None);
+        let mock = p.chooser.as_ref().unwrap();
+        assert_eq!(mock.requests().len(), 1);
+        assert_eq!(mock.traced_calls(), 0, "the untraced path calls `choose`");
+
+        let p = player(MockChooser::failing(JevError::Timeout));
+        assert_eq!(p.choose_move(&Game::new()).unwrap().exchange, None);
+    }
+
+    #[test]
+    fn no_exchange_when_jev_is_not_asked() {
+        for (fen, source) in [
+            ("7k/8/8/8/8/8/6q1/7K w - - 0 1", MoveSource::OnlyMove),
+            (
+                "6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 0 1",
+                MoveSource::MateInOne,
+            ),
+        ] {
+            let p = traced_player(MockChooser::answering("x", vec![]));
+            let result = p.choose_move(&game(fen)).unwrap();
+            assert_eq!(result.source, source);
+            assert_eq!(result.exchange, None, "{fen}");
+            assert!(p.chooser.as_ref().unwrap().requests().is_empty());
+        }
+
+        let config = EngineConfig {
+            trace: true,
+            ..EngineConfig::default()
+        };
+        let p: ComputerPlayer<MockChooser> = ComputerPlayer::new(None, config);
+        let result = p.choose_move(&Game::new()).unwrap();
+        assert_eq!(result.source, MoveSource::Fallback);
+        assert_eq!(result.exchange, None, "no key, no request");
+    }
+
+    #[test]
+    fn no_exchange_from_a_chooser_that_does_not_trace() {
+        let config = EngineConfig {
+            trace: true,
+            ..EngineConfig::default()
+        };
+        let chooser = Untraced(MockChooser::answering("Kd2", vec![("Kd2", 1.0)]));
+        let p = ComputerPlayer::new(Some(chooser), config);
+        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
+        assert_eq!(result.source, MoveSource::Jev);
+        assert_eq!(result.exchange, None);
+    }
 }
````

- [ ] **Step: Run the tests to verify they fail**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib engine::`
Expected: FAIL. The test build does not compile: the tests use names the implementation patch adds, for example "method choose_traced is not a member of trait MoveChooser"; "cannot find function redact_value in this scope"; "cannot find function redact in this scope"; "unresolved imports crate::engine::jev::JevAttempt, crate::engine::jev::JevExchange". Any other failure (a patch that does not apply, a test that fails at run time) is not the expected RED: stop and report.

- [ ] **Step: Apply the implementation patch**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/engine/config.rs b/src/engine/config.rs
index 4d25ea64d28ff6c0086677736aebbb79d6efc0ec..89e7de47bf571ca55b637fefe38d10925193b65c 100644
--- a/src/engine/config.rs
+++ b/src/engine/config.rs
@@ -30,6 +30,12 @@ pub struct EngineConfig {
     pub veto_margin_cp: i32,
     /// Human-readable notes about ignored or adjusted settings, for the TUI.
     pub warnings: Vec<String>,
+    /// Record each Jev request and its responses in [`ComputerMove::exchange`]
+    /// (debug mode). Off by default and never read from the environment here: the
+    /// TUI sets it from `--debug` / `RCHESS_DEBUG`.
+    ///
+    /// [`ComputerMove::exchange`]: super::ComputerMove::exchange
+    pub trace: bool,
 }
 
 impl Default for EngineConfig {
@@ -42,6 +48,7 @@ impl Default for EngineConfig {
             timeout: Duration::from_secs(5),
             veto_margin_cp: 150,
             warnings: Vec::new(),
+            trace: false,
         }
     }
 }
@@ -56,6 +63,7 @@ impl fmt::Debug for EngineConfig {
             .field("timeout", &self.timeout)
             .field("veto_margin_cp", &self.veto_margin_cp)
             .field("warnings", &self.warnings)
+            .field("trace", &self.trace)
             .finish()
     }
 }
diff --git a/src/engine/jev.rs b/src/engine/jev.rs
index 967e4d80fdc6000a7efcfd08297d6e056af8e312..253de9eaae7fbac80ed73a8cd3ac228713ab9d58 100644
--- a/src/engine/jev.rs
+++ b/src/engine/jev.rs
@@ -1,6 +1,8 @@
 //! TypeSafe Jev client: request/response types, the retry policy and the HTTP
 //! transport (spec 5.1, 5.7). Everything is tested offline; the transport tests
-//! talk to a scripted HTTP server on 127.0.0.1.
+//! talk to a scripted HTTP server on 127.0.0.1. For debug mode the client can also
+//! record each request and its responses as a [`JevExchange`], with the API key
+//! redacted (spec 9.5).
 //!
 //! Never enable TRACE-level logging for `ureq` or `ureq_proto`: it prints request
 //! headers, including the `Authorization` bearer key.
@@ -8,7 +10,7 @@
 use std::collections::HashMap;
 use std::fmt;
 use std::thread;
-use std::time::Duration;
+use std::time::{Duration, Instant};
 
 use serde::{Deserialize, Serialize};
 use serde_json::{Map, Value, json};
@@ -29,6 +31,10 @@ const MAX_RETRY_AFTER: Duration = Duration::from_secs(2);
 const ERROR_SNIPPET_CHARS: usize = 200;
 /// Largest response body read, in bytes; a real answer is a few kilobytes.
 const MAX_BODY_BYTES: u64 = 1 << 20;
+/// The request's `Content-Type`, as the TypeSafe quickstart sends it.
+const CONTENT_TYPE: &str = "application/json";
+/// What a recorded exchange shows in place of the API key.
+const REDACTED: &str = "<redacted>";
 
 /// One candidate move offered to Jev.
 #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
@@ -98,7 +104,8 @@ pub struct ChoiceAnswer {
     pub input_tokens: u32,
 }
 
-/// Why a Jev request failed. Messages never contain the API key.
+/// Why a Jev request failed. Messages never contain the API key: `JevClient`
+/// replaces any occurrence of it with `<redacted>`.
 #[derive(Debug, Error, Clone, PartialEq)]
 pub enum JevError {
     /// The API answered with a status other than 200.
@@ -126,10 +133,54 @@ pub enum JevError {
     Request(String),
 }
 
+/// One HTTP request to Jev and every attempt made for it, for debug mode (spec 9.5).
+/// The API key never appears in it: the `Authorization` header reads
+/// `Bearer <redacted>`, and any occurrence of the key in the body, a response or
+/// an error is replaced by `<redacted>`.
+#[derive(Clone, Debug, PartialEq)]
+pub struct JevExchange {
+    /// The HTTP method, `POST`.
+    pub method: String,
+    /// The endpoint the request went to.
+    pub url: String,
+    /// The headers the client sets, in the order it sets them.
+    pub headers: Vec<(String, String)>,
+    /// The JSON body sent with every attempt.
+    pub body: Value,
+    /// Every attempt in order, including the ones that were retried.
+    pub attempts: Vec<JevAttempt>,
+}
+
+/// One attempt of a [`JevExchange`].
+#[derive(Clone, Debug, PartialEq, Eq)]
+pub struct JevAttempt {
+    /// The HTTP status, or `None` when no response arrived.
+    pub status: Option<u16>,
+    /// The response body as text (read under the 1 MiB cap), or `None` when no
+    /// response arrived or its body could not be read.
+    pub response: Option<String>,
+    /// Why the attempt failed, as the `JevError` message; `None` for an answer.
+    pub error: Option<String>,
+    /// Time from sending the request to reading the whole response, without the
+    /// backoff before the next attempt.
+    pub elapsed: Duration,
+}
+
 /// Something that can answer a `choice` question: `JevClient`, or a mock in tests.
 pub trait MoveChooser: Send + Sync {
     /// Answers one `choice` question. An error is final: retries happen inside.
     fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceAnswer, JevError>;
+
+    /// Like [`choose`](MoveChooser::choose), and records the HTTP exchange in
+    /// `trace`, whether it succeeded or not. The default calls `choose` and records
+    /// nothing, leaving `trace` as it was (callers pass `None`).
+    fn choose_traced(
+        &self,
+        request: &ChoiceRequest,
+        _trace: &mut Option<JevExchange>,
+    ) -> Result<ChoiceAnswer, JevError> {
+        self.choose(request)
+    }
 }
 
 #[derive(Deserialize)]
@@ -212,6 +263,59 @@ pub fn retry_delay(error: &JevError, attempt: u32) -> Option<Duration> {
     }
 }
 
+/// `text` with every occurrence of `secret` replaced by `<redacted>`. An empty
+/// secret redacts nothing.
+fn redact(text: &str, secret: &str) -> String {
+    if secret.is_empty() {
+        text.to_string()
+    } else {
+        text.replace(secret, REDACTED)
+    }
+}
+
+/// `value` with [`redact`] applied to every string and object key in it.
+fn redact_value(value: &Value, secret: &str) -> Value {
+    match value {
+        Value::String(text) => Value::String(redact(text, secret)),
+        Value::Array(items) => {
+            Value::Array(items.iter().map(|v| redact_value(v, secret)).collect())
+        }
+        Value::Object(map) => Value::Object(
+            map.iter()
+                .map(|(k, v)| (redact(k, secret), redact_value(v, secret)))
+                .collect(),
+        ),
+        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
+    }
+}
+
+/// `error` with [`redact`] applied to its text.
+fn redact_error(error: JevError, secret: &str) -> JevError {
+    match error {
+        JevError::Http {
+            status,
+            message,
+            retry_after,
+        } => JevError::Http {
+            status,
+            message: redact(&message, secret),
+            retry_after,
+        },
+        JevError::Transport(text) => JevError::Transport(redact(&text, secret)),
+        JevError::InvalidResponse(text) => JevError::InvalidResponse(redact(&text, secret)),
+        JevError::Request(text) => JevError::Request(redact(&text, secret)),
+        JevError::Timeout => JevError::Timeout,
+    }
+}
+
+/// What one attempt produced: the answer or error, plus the status and the
+/// redacted response body when they arrived.
+struct Attempt {
+    result: Result<ChoiceAnswer, JevError>,
+    status: Option<u16>,
+    response: Option<String>,
+}
+
 /// Classifies a `ureq` failure: timeouts and connection problems are worth
 /// retrying; anything else (a bad URL, a protocol error, an oversized body) is final.
 fn request_error(error: ureq::Error) -> JevError {
@@ -270,16 +374,78 @@ impl JevClient {
         })
     }
 
-    fn post_once(&self, body: &Value) -> Result<ChoiceAnswer, JevError> {
-        let mut response = self
+    /// The exchange record for `body` before any attempt: the redacted headers the
+    /// client sends and the body with the key redacted.
+    fn exchange(&self, body: &Value) -> JevExchange {
+        JevExchange {
+            method: "POST".to_string(),
+            url: self.url.clone(),
+            headers: vec![
+                ("Authorization".to_string(), format!("Bearer {REDACTED}")),
+                ("Content-Type".to_string(), CONTENT_TYPE.to_string()),
+            ],
+            body: redact_value(body, &self.api_key),
+            attempts: Vec::new(),
+        }
+    }
+
+    /// Posts `body` up to [`MAX_ATTEMPTS`] times, retrying as [`retry_delay`] says,
+    /// and pushes each attempt to `attempts` when it is given.
+    fn post(
+        &self,
+        body: &Value,
+        mut attempts: Option<&mut Vec<JevAttempt>>,
+    ) -> Result<ChoiceAnswer, JevError> {
+        let mut attempt = 1;
+        loop {
+            let started = Instant::now();
+            let Attempt {
+                result,
+                status,
+                response,
+            } = self.post_once(body);
+            if let Some(attempts) = attempts.as_deref_mut() {
+                attempts.push(JevAttempt {
+                    status,
+                    response,
+                    error: result.as_ref().err().map(JevError::to_string),
+                    elapsed: started.elapsed(),
+                });
+            }
+            match result {
+                Ok(answer) => return Ok(answer),
+                Err(error) => match retry_delay(&error, attempt) {
+                    Some(delay) => {
+                        thread::sleep(delay);
+                        attempt += 1;
+                    }
+                    None => return Err(error),
+                },
+            }
+        }
+    }
+
+    /// One HTTP attempt. The response body is redacted before it is parsed or cut
+    /// into an error snippet, so no error can carry part of the key.
+    fn post_once(&self, body: &Value) -> Attempt {
+        let sent = self
             .agent
             .post(&self.url)
             .header("Authorization", &format!("Bearer {}", self.api_key))
             // `send_json` alone would send `application/json; charset=utf-8`; the
             // TypeSafe quickstart sends plain `application/json`.
-            .content_type("application/json")
-            .send_json(body)
-            .map_err(request_error)?;
+            .content_type(CONTENT_TYPE)
+            .send_json(body);
+        let mut response = match sent {
+            Ok(response) => response,
+            Err(error) => {
+                return Attempt {
+                    result: Err(redact_error(request_error(error), &self.api_key)),
+                    status: None,
+                    response: None,
+                };
+            }
+        };
         let status = response.status().as_u16();
         let retry_after = response
             .headers()
@@ -291,38 +457,45 @@ impl JevClient {
             .body_mut()
             .with_config()
             .limit(MAX_BODY_BYTES)
-            .read_to_string();
-        if status != 200 {
+            .read_to_string()
+            .map(|text| redact(&text, &self.api_key));
+        let (result, response) = match text {
+            Ok(text) if status == 200 => (parse_answer(&text), Some(text)),
+            Ok(text) => (Err(http_error(status, &text, retry_after)), Some(text)),
             // Keep the status and Retry-After even when the body cannot be read.
-            return Err(match text {
-                Ok(text) => http_error(status, &text, retry_after),
-                Err(_) => JevError::Http {
+            Err(_) if status != 200 => (
+                Err(JevError::Http {
                     status,
                     message: "<unreadable body>".into(),
                     retry_after,
-                },
-            });
+                }),
+                None,
+            ),
+            Err(error) => (Err(request_error(error)), None),
+        };
+        Attempt {
+            result: result.map_err(|error| redact_error(error, &self.api_key)),
+            status: Some(status),
+            response,
         }
-        parse_answer(&text.map_err(request_error)?)
     }
 }
 
 impl MoveChooser for JevClient {
     fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceAnswer, JevError> {
+        self.post(&request.to_body(&self.model), None)
+    }
+
+    /// Records the method, URL, redacted headers, the body and every attempt,
+    /// including retried ones; `trace` is `Some` afterwards, answer or error.
+    fn choose_traced(
+        &self,
+        request: &ChoiceRequest,
+        trace: &mut Option<JevExchange>,
+    ) -> Result<ChoiceAnswer, JevError> {
         let body = request.to_body(&self.model);
-        let mut attempt = 1;
-        loop {
-            match self.post_once(&body) {
-                Ok(answer) => return Ok(answer),
-                Err(error) => match retry_delay(&error, attempt) {
-                    Some(delay) => {
-                        thread::sleep(delay);
-                        attempt += 1;
-                    }
-                    None => return Err(error),
-                },
-            }
-        }
+        let exchange = trace.insert(self.exchange(&body));
+        self.post(&body, Some(&mut exchange.attempts))
     }
 }
 
diff --git a/src/engine/mod.rs b/src/engine/mod.rs
index c867ecf3c782a6f51a7265fef88cd565d41d0aac..53a454d9da380df66b0caa1d87eec09c78bbda68 100644
--- a/src/engine/mod.rs
+++ b/src/engine/mod.rs
@@ -13,7 +13,8 @@ pub use annotate::{Annotation, Bucket};
 pub use config::EngineConfig;
 pub use describe::JevState;
 pub use jev::{
-    ChoiceAnswer, ChoiceOption, ChoiceRequest, JEV_ENDPOINT, JevClient, JevError, MoveChooser,
+    ChoiceAnswer, ChoiceOption, ChoiceRequest, JEV_ENDPOINT, JevAttempt, JevClient, JevError,
+    JevExchange, MoveChooser,
 };
 pub use player::{ComputerMove, ComputerPlayer, MoveSource};
 pub use search::{MATE, ScoredMove, analyse};
diff --git a/src/engine/player.rs b/src/engine/player.rs
index 0ed797b1c8ac4f19a537f4dc7ceacb4ba7de1c25..c236b3cccc7d5b6d1a5e3bffbca0165ff6d62037 100644
--- a/src/engine/player.rs
+++ b/src/engine/player.rs
@@ -10,7 +10,7 @@ use crate::core::{Game, Move};
 use super::annotate::{Annotation, Bucket, annotate};
 use super::config::{EngineConfig, MAX_CHOICE_OPTIONS};
 use super::describe::describe;
-use super::jev::{ChoiceOption, ChoiceRequest, JevClient, MoveChooser, printable};
+use super::jev::{ChoiceOption, ChoiceRequest, JevClient, JevExchange, MoveChooser, printable};
 use super::search::{MATE, analyse};
 
 /// Longest part of an unknown option key echoed back in a note.
@@ -73,6 +73,10 @@ pub struct ComputerMove {
     pub input_tokens: Option<u32>,
     /// Why a fallback or veto happened.
     pub note: Option<String>,
+    /// The HTTP exchange with Jev, when [`EngineConfig::trace`] is on and Jev was
+    /// asked (a Jev or vetoed move, or a fallback after Jev failed or answered
+    /// unusably); `None` otherwise.
+    pub exchange: Option<Box<JevExchange>>,
 }
 
 /// Chooses computer moves: local search, plus Jev through `C` when available.
@@ -129,6 +133,7 @@ impl<C: MoveChooser> ComputerPlayer<C> {
             latency: started.elapsed(),
             input_tokens: None,
             note,
+            exchange: None,
         };
 
         if scored.len() == 1 {
@@ -162,11 +167,21 @@ impl<C: MoveChooser> ComputerPlayer<C> {
                 .collect(),
         };
 
-        let answer = match chooser.choose(&request) {
+        let mut trace = None;
+        let answer = if self.config.trace {
+            chooser.choose_traced(&request, &mut trace)
+        } else {
+            chooser.choose(&request)
+        };
+        let exchange = trace.map(Box::new);
+        let answer = match answer {
             Ok(answer) => answer,
             Err(error) => {
                 let note = format!("Jev unavailable ({error}) — local search");
-                return Some(plain(best.mv, MoveSource::Fallback, Some(note)));
+                return Some(ComputerMove {
+                    exchange,
+                    ..plain(best.mv, MoveSource::Fallback, Some(note))
+                });
             }
         };
         let Some(pick) = candidates.iter().find(|a| a.san == answer.choice) else {
@@ -179,7 +194,10 @@ impl<C: MoveChooser> ComputerPlayer<C> {
                 "Jev returned an unknown option ({}{cut}) — local search",
                 printable(&answer.choice, NOTE_KEY_CHARS)
             );
-            return Some(plain(best.mv, MoveSource::Fallback, Some(note)));
+            return Some(ComputerMove {
+                exchange,
+                ..plain(best.mv, MoveSource::Fallback, Some(note))
+            });
         };
 
         let margin = self.config.veto_margin_cp;
@@ -223,6 +241,7 @@ impl<C: MoveChooser> ComputerPlayer<C> {
             latency: started.elapsed(),
             input_tokens: Some(answer.input_tokens),
             note,
+            exchange,
         })
     }
 }
diff --git a/src/tui/app.rs b/src/tui/app.rs
index 5bb3f8fc6be4504fb57617d2f2c80d1206603c64..57993adef453faa328ffcf7f27b88ac25b6ca0f7 100644
--- a/src/tui/app.rs
+++ b/src/tui/app.rs
@@ -3386,6 +3386,7 @@ mod tests {
             latency: Duration::from_millis(300),
             input_tokens: None,
             note: Some(ENGINE_ERROR_NOTE.to_string()),
+            exchange: None,
         };
         h.send(AppEvent::Engine(EngineReply {
             generation: request.generation,
diff --git a/src/tui/panels.rs b/src/tui/panels.rs
index e2eefaa1fc44f959a935b6f0a155135da3284f04..41ac600518f48f023dc19e9eb98a53d5bd1728bf 100644
--- a/src/tui/panels.rs
+++ b/src/tui/panels.rs
@@ -1937,6 +1937,7 @@ mod tests {
             latency: Duration::from_millis(870),
             input_tokens: Some(900),
             note: None,
+            exchange: None,
         };
         assert_eq!(
             texts(&jev_lines(Some(&computer), JEV_STATUS, 40)),
diff --git a/src/tui/test_support/engine.rs b/src/tui/test_support/engine.rs
index 01fa00ea8427eeeead9fbb200197d8c33547166d..56f680c3787f1c37052a06a929f9069c0d5c52b7 100644
--- a/src/tui/test_support/engine.rs
+++ b/src/tui/test_support/engine.rs
@@ -71,6 +71,7 @@ pub(crate) fn jev_move(pos: &ChessPosition, uci: &str) -> ComputerMove {
         latency: Duration::from_millis(1234),
         input_tokens: Some(512),
         note: None,
+        exchange: None,
     }
 }
 
diff --git a/src/tui/worker.rs b/src/tui/worker.rs
index c602bf42162107988d4b1c3639de3e456e290c75..46cd83be0ed33d8f770a893a9e78817757d96ad2 100644
--- a/src/tui/worker.rs
+++ b/src/tui/worker.rs
@@ -220,6 +220,7 @@ fn local_search_move(game: &Game, started: Instant) -> Option<ComputerMove> {
         latency: started.elapsed(),
         input_tokens: None,
         note: Some(ENGINE_ERROR_NOTE.to_string()),
+        exchange: None,
     })
 }
 
````

- [ ] **Step: Run the tests to verify they pass**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::` and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test --lib engine::`
Expected: tui:: 306 passed, engine:: 109 passed; `find src -name '*.snap.new'` prints nothing.

Run: `cargo fmt --check` and `cargo clippy --all-targets 2>&1 | grep -A5 -E "src/tui|src/engine|src/main.rs"`
Expected: no output from either.

- [ ] **Step: Update the handoff file**

Edit `docs/handoff/HANDOFF.md`. In `## Current`, set:

```
Sub-project: tui-polish | Plan: docs/superpowers/plans/2026-09-27-tui-polish.md
Branch: feat/tui-polish
Last completed task: tui-polish task 2 (Engine: record the Jev exchange for debug mode)
Next task: tui-polish task 3 (full-screen layout)
State: green (except 3 pre-existing old-code test failures)
```

Set `## Verify before continuing` to:

```
env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui:: && cargo test --lib engine::
```

Add at the top of `## Log (newest first)`: `- <today's date> tui-polish task 2 done: Engine: record the Jev exchange for debug mode`. Keep every other section. Record under `## Notes / decisions made during work` any decision this plan did not specify.

- [ ] **Step: Refresh the knowledge graph**

Run: `graphify update .`
Expected: completes without error. Never commit `graphify-out/`, `.claude/` or `.superpowers/`.

- [ ] **Step: Commit**

```bash
git add docs/handoff/HANDOFF.md src/engine/config.rs src/engine/jev.rs src/engine/mod.rs src/engine/player.rs src/tui/app.rs src/tui/panels.rs src/tui/test_support/engine.rs src/tui/worker.rs
git commit -m "feat(engine): record the Jev request and responses when tracing, with the key redacted

Co-Authored-By: <the model you are> <noreply@anthropic.com>"
```

Replace `<the model you are>` with your own model name.

---

### Task 3: Full-screen layout with font-shaped squares

**Files:**
- Modify: `src/tui/app.rs`, `src/tui/board.rs`, `src/tui/panels.rs`, `src/tui/snapshots/chess__tui__panels__tests__playing_start_120x40_vs_jev.snap`

**Interfaces:**
- Consumes: `board::BoardGeometry`, `square_rect`, `square_at`, panels layout.
- Produces (`src/tui/board.rs`): `CellSize` (`new(width, height)`, `width()`, `height()`, `DEFAULT` = 10×20); `MIN_SQUARE = (3, 1)`; `square_width(square_h: u16, cell: CellSize) -> u16` (odd, at least 3); `layout_board(area: Rect, flipped: bool, cell: CellSize) -> Option<BoardGeometry>` (replaces the three presets). `App::cell_size()`, `App::set_cell_size(CellSize)`.
- Panels: the Playing screen fills the terminal (no `SIDE_MAX_WIDTH`, no centring); menu and dialogs stay centred boxes. Existing snapshots change accordingly (hand-reviewed).

- [ ] **Step: Confirm the starting point**

Run: `git status --short` (only untracked `.claude/`, `graphify-out/` and user files) and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: 306 passed (engine: `cargo test --lib engine::` 109 passed).

- [ ] **Step: Apply the tests patch (write the failing tests)**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/tui/board.rs b/src/tui/board.rs
index cf4760666df6980962779242c01e40e841963cf9..f07c73e1ae01a75392899f496a1f6602f2696d24 100644
--- a/src/tui/board.rs
+++ b/src/tui/board.rs
@@ -376,9 +376,12 @@ mod tests {
     use crate::tui::glyphs::{SOLID_PAWN, palette};
     use crate::tui::test_support::sq;
 
+    /// Square sizes the default font gives for heights 1 to 4.
+    const SIZES: [(u16, u16); 4] = [(3, 1), (5, 2), (7, 3), (9, 4)];
+
     fn geometry(square_w: u16, square_h: u16, x: u16, y: u16, flipped: bool) -> BoardGeometry {
         let area = Rect::new(x, y, 8 * square_w + 1, 8 * square_h + 1);
-        let g = layout_board(area, flipped).expect("exact-fit area");
+        let g = layout_board(area, flipped, CellSize::DEFAULT).expect("exact-fit area");
         assert_eq!((g.square_w, g.square_h), (square_w, square_h));
         g
     }
@@ -413,7 +416,7 @@ mod tests {
         terminal
             .draw(|frame| {
                 let area = frame.area();
-                let geometry = layout_board(area, flipped).expect("board fits");
+                let geometry = layout_board(area, flipped, CellSize::DEFAULT).expect("board fits");
                 saved = Some(geometry);
                 frame.render_widget(
                     BoardView {
@@ -436,38 +439,121 @@ mod tests {
     }
 
     #[test]
-    fn layout_picks_the_largest_size_that_fits_and_centres_it() {
+    fn cell_size_defaults_to_ten_by_twenty() {
+        assert_eq!(CellSize::default(), CellSize::DEFAULT);
+        assert_eq!(
+            (CellSize::DEFAULT.width(), CellSize::DEFAULT.height()),
+            (10, 20)
+        );
+        let font = CellSize::new(9, 19);
+        assert_eq!((font.width(), font.height()), (9, 19));
+        // A terminal that does not know its font size reports zero.
+        for (width, height) in [(0, 0), (0, 20), (10, 0)] {
+            assert_eq!(CellSize::new(width, height), CellSize::DEFAULT);
+        }
+    }
+
+    #[test]
+    fn square_width_makes_squares_look_square() {
+        // (font, widths for square heights 1 to 8)
         let cases = [
-            ((80, 24), (5, 2), Rect::new(19, 3, 41, 17)),
-            ((120, 40), (7, 3), Rect::new(31, 7, 57, 25)),
-            ((200, 60), (7, 3), Rect::new(71, 17, 57, 25)),
-            ((60, 20), (5, 2), Rect::new(9, 1, 41, 17)),
-            ((40, 16), (3, 1), Rect::new(7, 3, 25, 9)),
-            ((25, 9), (3, 1), Rect::new(0, 0, 25, 9)),
+            // Twice as tall as wide: 2h, bumped to odd.
+            ((10, 20), [3, 5, 7, 9, 11, 13, 15, 17]),
+            ((8, 16), [3, 5, 7, 9, 11, 13, 15, 17]),
+            ((16, 32), [3, 5, 7, 9, 11, 13, 15, 17]),
+            // 2.5: 2.5 rounds to 3, 7.5 to 8 and then 9.
+            ((10, 25), [3, 5, 9, 11, 13, 15, 19, 21]),
+            // 15/7 = 2.14: 4.29 rounds to 4 then 5, 8.57 to 9.
+            ((7, 15), [3, 5, 7, 9, 11, 13, 15, 17]),
+            // 1.8: 3.6 rounds to 4 then 5, 5.4 to 5, 7.2 to 7.
+            ((10, 18), [3, 5, 5, 7, 9, 11, 13, 15]),
+            // Square cells: at least 3 columns.
+            ((12, 12), [3, 3, 3, 5, 5, 7, 7, 9]),
+            // Very tall cells: one row already needs 5 columns.
+            ((5, 20), [5, 9, 13, 17, 21, 25, 29, 33]),
         ];
-        for ((width, height), size, outer) in cases {
-            let g = layout_board(Rect::new(0, 0, width, height), false)
-                .unwrap_or_else(|| panic!("{width}x{height} should fit"));
-            assert_eq!((g.square_w, g.square_h), size, "{width}x{height}");
-            assert_eq!(g.outer, outer, "{width}x{height}");
+        for ((cell_w, cell_h), widths) in cases {
+            let cell = CellSize::new(cell_w, cell_h);
+            for (square_h, width) in (1..=8).zip(widths) {
+                assert_eq!(
+                    square_width(square_h, cell),
+                    width,
+                    "{cell_w}x{cell_h} font, {square_h} rows"
+                );
+            }
+        }
+        // Huge values saturate instead of overflowing.
+        assert_eq!(square_width(u16::MAX, CellSize::new(1, u16::MAX)), u16::MAX);
+        assert_eq!(square_width(0, CellSize::DEFAULT), 3);
+    }
+
+    #[test]
+    fn layout_picks_the_tallest_square_that_fits_and_centres_it() {
+        // (area, font, square size, outer rect)
+        let cases = [
+            ((80, 24), (10, 20), (5, 2), Rect::new(19, 3, 41, 17)),
+            ((120, 40), (10, 20), (9, 4), Rect::new(23, 3, 73, 33)),
+            ((200, 60), (10, 20), (15, 7), Rect::new(39, 1, 121, 57)),
+            ((60, 20), (10, 20), (5, 2), Rect::new(9, 1, 41, 17)),
+            ((40, 16), (10, 20), (3, 1), Rect::new(7, 3, 25, 9)),
+            ((25, 9), (10, 20), (3, 1), Rect::new(0, 0, 25, 9)),
+            // Limited by width: shorter squares, centred vertically.
+            ((73, 60), (10, 20), (9, 4), Rect::new(0, 13, 73, 33)),
+            ((74, 60), (10, 20), (9, 4), Rect::new(0, 13, 73, 33)),
+            ((80, 40), (10, 25), (9, 3), Rect::new(3, 7, 73, 25)),
+            // A taller font needs more columns per square, a wider one fewer.
+            ((120, 40), (10, 25), (11, 4), Rect::new(15, 3, 89, 33)),
+            ((80, 40), (12, 12), (5, 4), Rect::new(19, 3, 41, 33)),
+        ];
+        for ((width, height), (cell_w, cell_h), size, outer) in cases {
+            let at = format!("{width}x{height} with a {cell_w}x{cell_h} font");
+            let g = layout_board(
+                Rect::new(0, 0, width, height),
+                false,
+                CellSize::new(cell_w, cell_h),
+            )
+            .unwrap_or_else(|| panic!("{at} should fit"));
+            assert_eq!((g.square_w, g.square_h), size, "{at}");
+            assert_eq!(g.outer, outer, "{at}");
             assert_eq!(
                 g.grid,
                 Rect::new(outer.x + 1, outer.y, 8 * size.0, 8 * size.1),
-                "{width}x{height}"
+                "{at}"
             );
         }
     }
 
+    #[test]
+    fn a_very_narrow_font_still_gets_the_smallest_squares() {
+        // One-row squares would be 5 columns wide (41 with labels): too wide for 30.
+        let tall = CellSize::new(5, 20);
+        let g = layout_board(Rect::new(0, 0, 30, 20), false, tall).expect("fits");
+        assert_eq!((g.square_w, g.square_h), MIN_SQUARE);
+        assert_eq!(g.outer, Rect::new(2, 5, 25, 9));
+        // With the room, the font's own width wins.
+        let g = layout_board(Rect::new(0, 0, 41, 20), false, tall).expect("fits");
+        assert_eq!((g.square_w, g.square_h), (5, 1));
+    }
+
     #[test]
     fn layout_is_none_when_too_small() {
-        for (width, height) in [(24, 9), (25, 8), (0, 0), (100, 8), (24, 100)] {
-            assert_eq!(layout_board(Rect::new(0, 0, width, height), false), None);
+        for cell in [
+            CellSize::DEFAULT,
+            CellSize::new(5, 20),
+            CellSize::new(20, 10),
+        ] {
+            for (width, height) in [(24, 9), (25, 8), (0, 0), (100, 8), (24, 100)] {
+                assert_eq!(
+                    layout_board(Rect::new(0, 0, width, height), false, cell),
+                    None
+                );
+            }
         }
     }
 
     #[test]
     fn layout_respects_the_area_origin_and_flip() {
-        let g = layout_board(Rect::new(10, 5, 26, 10), true).expect("fits");
+        let g = layout_board(Rect::new(10, 5, 26, 10), true, CellSize::DEFAULT).expect("fits");
         assert_eq!(g.outer, Rect::new(10, 5, 25, 9));
         assert_eq!(g.grid, Rect::new(11, 5, 24, 8));
         assert!(g.flipped);
@@ -475,7 +561,7 @@ mod tests {
 
     #[test]
     fn square_rect_and_square_at_round_trip() {
-        for (w, h) in SQUARE_SIZES {
+        for (w, h) in SIZES {
             for flipped in [false, true] {
                 for (x, y) in [(0, 0), (7, 4)] {
                     let g = geometry(w, h, x, y, flipped);
@@ -501,7 +587,7 @@ mod tests {
 
     #[test]
     fn white_is_at_the_bottom_unless_flipped() {
-        for (w, h) in SQUARE_SIZES {
+        for (w, h) in SIZES {
             let g = geometry(w, h, 0, 0, false);
             let bottom = g.grid.bottom() - 1;
             let right = g.grid.right() - 1;
@@ -519,7 +605,7 @@ mod tests {
 
     #[test]
     fn clicks_outside_the_grid_miss() {
-        for (w, h) in SQUARE_SIZES {
+        for (w, h) in SIZES {
             for flipped in [false, true] {
                 let g = geometry(w, h, 3, 2, flipped);
                 let grid = g.grid;
diff --git a/src/tui/panels.rs b/src/tui/panels.rs
index 41ac600518f48f023dc19e9eb98a53d5bd1728bf..28a952632218b4a66740d602aa1fccf21279a6cb 100644
--- a/src/tui/panels.rs
+++ b/src/tui/panels.rs
@@ -1296,7 +1296,8 @@ mod tests {
     use ratatui::widgets::Widget;
 
     use super::*;
-    use crate::core::START_FEN;
+    use crate::core::{START_FEN, Square};
+    use crate::tui::board::{square_at, square_rect};
     use crate::tui::event::AppEvent;
     use crate::tui::glyphs::initial_glyphs;
     use crate::tui::test_support::engine::{FakeEngine, JEV_STATUS};
@@ -1313,7 +1314,12 @@ mod tests {
     fn panel_rows(h: &Harness, title: &str) -> Vec<String> {
         let buffer = h.buffer();
         let area = buffer.area;
-        let layout = playing_layout(area, status_rows(h.app.mode(), area.height), None);
+        let layout = playing_layout(
+            area,
+            h.app.cell_size(),
+            status_rows(h.app.mode(), area.height),
+            None,
+        );
         let (left, right) = (layout.status.x, layout.status.right());
         let rows: Vec<String> = (0..area.height)
             .map(|y| (left..right).map(|x| buffer[(x, y)].symbol()).collect())
@@ -1394,80 +1400,238 @@ mod tests {
         h
     }
 
+    /// Terminal sizes the layout is checked at, from the minimum up.
+    const SIZES: [(u16, u16); 5] = [(60, 20), (80, 24), (120, 40), (200, 60), (300, 100)];
+    /// Font sizes (cell width × height in pixels) the layout is checked with.
+    const FONTS: [(u16, u16); 3] = [(10, 20), (8, 16), (16, 32)];
+
+    /// An app with a `cell_w`×`cell_h` pixel font in a `width`×`height` terminal, started
+    /// from the menu with `key` (against Jev for '2' to '5').
+    fn with_font((width, height): (u16, u16), (cell_w, cell_h): (u16, u16), key: char) -> Harness {
+        let mut h = Harness::build(FakeEngine::jev(), (width, height), Vec::new(), |mut app| {
+            app.set_cell_size(CellSize::new(cell_w, cell_h));
+            app
+        });
+        h.char(key);
+        h
+    }
+
     #[test]
-    fn playing_layout_fits_every_size() {
-        let sizes = [
+    fn the_board_takes_the_tallest_squares_that_fit() {
+        // (terminal, font, square width and height). The three fonts are all twice as tall
+        // as wide, so they agree; other shapes follow.
+        let cases = [
             ((60, 20), (3, 1)),
             ((80, 24), (5, 2)),
-            ((100, 30), (7, 3)),
-            ((120, 40), (7, 3)),
-            ((200, 60), (7, 3)),
-            ((70, 50), (3, 1)),
+            ((120, 40), (9, 4)),
+            ((200, 60), (13, 6)),
+            ((300, 100), (23, 11)),
         ];
-        for ((width, height), square) in sizes {
-            let area = Rect::new(0, 0, width, height);
-            for with_jev in [false, true] {
-                let mode = if with_jev {
-                    Mode::JevVsJev
-                } else {
-                    Mode::HumanVsHuman
-                };
-                let jev_rows = with_jev.then_some(40);
-                let l = playing_layout(area, status_rows(mode, height), jev_rows);
-                let at = format!("{width}x{height} jev={with_jev}");
-                let inner = Block::bordered()
-                    .padding(Padding::horizontal(1))
-                    .inner(l.board);
-                let board = layout_board(inner, false).expect("board fits");
-                assert_eq!((board.square_w, board.square_h), square, "{at}");
-                assert_eq!(board.outer.width, inner.width, "no spare columns: {at}");
-                assert_eq!(
-                    (l.command.x, l.command.y, l.command.width, l.command.height),
-                    (l.board.x, l.board.bottom(), l.board.width, COMMAND_HEIGHT),
-                    "{at}"
-                );
-                assert_eq!(l.command.bottom(), area.bottom(), "{at}");
-                let side = l.status.width;
-                assert!((SIDE_MIN_WIDTH..=SIDE_MAX_WIDTH).contains(&side), "{at}");
-                assert_eq!(l.status.x, l.board.right(), "{at}");
-                // Centred: the margins differ by at most one column.
-                let left = l.board.x - area.x;
-                let right = area.right() - l.status.right();
-                assert!(left.abs_diff(right) <= 1, "{at}");
-                // The right column: full height, one shared border row between panels.
-                let column: Vec<Rect> = [Some(l.status), l.jev, Some(l.moves), Some(l.captured)]
-                    .into_iter()
-                    .flatten()
-                    .collect();
-                assert_eq!(column.len(), if with_jev { 4 } else { 3 }, "{at}");
-                assert_eq!(column[0].y, area.y, "{at}");
-                for pair in column.windows(2) {
-                    assert_eq!(pair[1].y, pair[0].bottom() - 1, "{at}");
-                    assert_eq!((pair[1].x, pair[1].width), (l.status.x, side), "{at}");
+        for ((width, height), square) in cases {
+            for font in FONTS {
+                let h = with_font((width, height), font, '1');
+                let g = h.app.hit_map().board.expect("board drawn");
+                let at = format!("{width}x{height} with a {font:?} font");
+                assert_eq!((g.square_w, g.square_h), square, "{at}");
+            }
+        }
+        // A 10x25 font makes squares wider: at 120x40 four rows would need 11 columns,
+        // which leave the side column too narrow, so the board is limited by the width.
+        let cases = [
+            ((60, 20), (3, 1)),
+            ((80, 24), (5, 2)),
+            ((120, 40), (9, 3)),
+            ((200, 60), (15, 6)),
+            ((300, 100), (29, 11)),
+        ];
+        for ((width, height), square) in cases {
+            let h = with_font((width, height), (10, 25), '1');
+            let g = h.app.hit_map().board.expect("board drawn");
+            assert_eq!((g.square_w, g.square_h), square, "{width}x{height}");
+        }
+    }
+
+    #[test]
+    fn the_playing_screen_uses_every_cell() {
+        for (width, height) in SIZES {
+            for font in FONTS.into_iter().chain([(10, 25), (5, 20)]) {
+                for with_jev in [false, true] {
+                    let at = format!("{width}x{height} font={font:?} jev={with_jev}");
+                    let area = Rect::new(0, 0, width, height);
+                    let (mode, jev_rows) = if with_jev {
+                        (Mode::JevVsJev, Some(40))
+                    } else {
+                        (Mode::HumanVsHuman, None)
+                    };
+                    let cell = CellSize::new(font.0, font.1);
+                    let l = playing_layout(area, cell, status_rows(mode, height), jev_rows);
+
+                    // Left column: the board block over the command box, full height.
+                    assert_eq!((l.board.x, l.board.y), (area.x, area.y), "{at}");
+                    assert_eq!(
+                        l.command,
+                        Rect::new(l.board.x, l.board.bottom(), l.board.width, COMMAND_HEIGHT),
+                        "{at}"
+                    );
+                    assert_eq!(l.command.bottom(), area.bottom(), "{at}");
+                    // The board block is exactly as wide as its board: no spare columns.
+                    let inner = Block::bordered()
+                        .padding(Padding::horizontal(1))
+                        .inner(l.board);
+                    let board = layout_board(inner, false, cell).expect("board fits");
+                    assert_eq!(board.outer.width, inner.width, "{at}");
+
+                    // Right column: every column left, at least the minimum, full height,
+                    // one shared border row between panels.
+                    let column: Vec<Rect> =
+                        [Some(l.status), l.jev, Some(l.moves), Some(l.captured)]
+                            .into_iter()
+                            .flatten()
+                            .collect();
+                    assert_eq!(column.len(), if with_jev { 4 } else { 3 }, "{at}");
+                    for rect in &column {
+                        assert_eq!(rect.x, l.board.right(), "{at}");
+                        assert_eq!(rect.right(), area.right(), "{at}");
+                    }
+                    assert!(l.status.width >= SIDE_MIN_WIDTH, "{at}");
+                    assert_eq!(column[0].y, area.y, "{at}");
+                    for pair in column.windows(2) {
+                        assert_eq!(pair[1].y, pair[0].bottom() - 1, "{at}");
+                    }
+                    assert_eq!(l.captured.bottom(), area.bottom(), "{at}");
+
+                    // Status and Captured keep their heights; Moves takes what Jev leaves.
+                    assert_eq!(l.status.height, status_rows(mode, height) + 2, "{at}");
+                    assert_eq!(l.captured.height, CAPTURED_ROWS + 2, "{at}");
+                    assert!(l.moves.height >= MOVES_MIN_ROWS + 2, "{at}");
+                    if let Some(jev) = l.jev {
+                        assert!(jev.height >= JEV_MIN_ROWS + 2, "{at}");
+                    }
                 }
-                assert_eq!(l.captured.bottom(), area.bottom(), "{at}");
-                assert!(
-                    l.moves.height >= MOVES_MIN_ROWS + 2,
-                    "room for three moves: {at}"
-                );
-                assert_eq!(l.status.height, status_rows(mode, height) + 2, "{at}");
-                if let Some(jev) = l.jev {
-                    assert!(jev.height >= JEV_MIN_ROWS + 2, "{at}");
+            }
+        }
+    }
+
+    #[test]
+    fn the_drawn_screen_reaches_every_edge() {
+        for size in SIZES {
+            for font in FONTS {
+                for key in ['1', '2'] {
+                    let h = with_font(size, font, key);
+                    let at = format!("{size:?} font={font:?} key={key}");
+                    let buffer = h.buffer();
+                    let (right, bottom) = (size.0 - 1, size.1 - 1);
+                    assert_eq!(buffer[(0, 0)].symbol(), "┌", "{at}");
+                    assert_eq!(buffer[(right, 0)].symbol(), "┐", "{at}");
+                    assert_eq!(buffer[(0, bottom)].symbol(), "└", "{at}");
+                    assert_eq!(buffer[(right, bottom)].symbol(), "┘", "{at}");
+                    for y in 1..bottom {
+                        assert_ne!(buffer[(0, y)].symbol(), " ", "{at} row {y}");
+                        assert_ne!(buffer[(right, y)].symbol(), " ", "{at} row {y}");
+                    }
                 }
             }
         }
     }
 
+    #[test]
+    fn the_right_column_panels_stretch() {
+        // Jev's text needs a few rows; Moves takes the rest of a tall terminal.
+        let h = with_font((300, 100), (10, 20), '2');
+        let area = h.buffer().area;
+        let rows = status_rows(h.app.mode(), area.height);
+        let jev = JevText::new(
+            None,
+            h.app.engine_status(),
+            side_text_width(area, h.app.cell_size()),
+        );
+        let l = playing_layout(area, h.app.cell_size(), rows, Some(jev.rows()));
+        let jev_rect = l.jev.expect("jev panel");
+        assert_eq!(jev_rect.height, JEV_MIN_ROWS + 2);
+        // Four panels share three border rows.
+        assert_eq!(
+            l.moves.height,
+            area.height - (rows + 2) - (JEV_MIN_ROWS + 2) - (CAPTURED_ROWS + 2) + 3
+        );
+        assert_eq!(
+            panel_rows(&h, "Moves").len(),
+            usize::from(l.moves.height - 2)
+        );
+    }
+
+    #[test]
+    fn a_board_limited_by_width_is_centred_in_a_full_height_block() {
+        // Tall and narrow: the side column's minimum width limits the board.
+        for (size, font) in [
+            ((70, 50), (10, 20)),
+            ((120, 40), (10, 25)),
+            ((60, 40), (8, 16)),
+        ] {
+            let h = with_font(size, font, '1');
+            let at = format!("{size:?} font={font:?}");
+            let g = h.app.hit_map().board.expect("board drawn");
+            let block = playing_layout(
+                h.buffer().area,
+                h.app.cell_size(),
+                status_rows(h.app.mode(), size.1),
+                None,
+            )
+            .board;
+            assert_eq!(block.height, size.1 - COMMAND_HEIGHT, "{at}");
+            let inner = Rect::new(block.x + 2, block.y + 1, block.width - 4, block.height - 2);
+            assert_eq!(g.outer.width, inner.width, "{at}");
+            let (above, below) = (g.outer.y - inner.y, inner.bottom() - g.outer.bottom());
+            assert!(above + below >= 8, "width-limited: {at}");
+            assert!(above.abs_diff(below) <= 1, "centred: {at}");
+        }
+    }
+
+    #[test]
+    fn every_square_is_hit_at_every_size() {
+        for size in SIZES {
+            for font in FONTS.into_iter().chain([(10, 25)]) {
+                // Human vs Human (White at the bottom), then flipped with `f`.
+                let mut h = with_font(size, font, '1');
+                for flipped in [false, true] {
+                    if flipped {
+                        h.char('f');
+                    }
+                    let at = format!("{size:?} font={font:?} flipped={flipped}");
+                    let g = h.app.hit_map().board.expect("board drawn");
+                    assert_eq!(g.flipped, flipped, "{at}");
+                    let mut covered = 0;
+                    for sq in Square::all() {
+                        let rect = square_rect(&g, sq);
+                        assert_eq!(rect.intersection(g.grid), rect, "{at}: {sq}");
+                        for pos in rect.positions() {
+                            assert_eq!(square_at(&g, pos.x, pos.y), Some(sq), "{at}: {pos:?}");
+                            covered += 1;
+                        }
+                    }
+                    assert_eq!(covered, g.grid.area(), "{at}");
+                }
+            }
+        }
+        // And clicks land: a move by mouse on the largest board, flipped.
+        let mut h = with_font((300, 100), (10, 20), '1');
+        h.char('f');
+        h.click_square(sq("e2"));
+        h.click_square(sq("e4"));
+        assert_eq!(h.uci(), ["e2e4"]);
+        h.drag_square(sq("g8"), sq("f6"));
+        assert_eq!(h.uci(), ["e2e4", "g8f6"]);
+    }
+
     #[test]
     fn the_jev_panel_grows_with_its_text_and_moves_gives_way() {
         let area = Rect::new(0, 0, 80, 24);
         let rows = status_rows(Mode::HumanVsJev { human: Side::White }, 24);
-        let short = playing_layout(area, rows, Some(1));
-        let long = playing_layout(area, rows, Some(6));
+        let short = playing_layout(area, CellSize::DEFAULT, rows, Some(1));
+        let long = playing_layout(area, CellSize::DEFAULT, rows, Some(6));
         assert_eq!(short.jev.expect("jev").height, JEV_MIN_ROWS + 2);
         assert_eq!(long.jev.expect("jev").height, 6 + 2);
         assert_eq!(short.moves.height - long.moves.height, 6 - JEV_MIN_ROWS);
-        let huge = playing_layout(area, rows, Some(100));
+        let huge = playing_layout(area, CellSize::DEFAULT, rows, Some(100));
         assert_eq!(
             huge.moves.height,
             MOVES_MIN_ROWS + 2,
@@ -1556,7 +1720,13 @@ mod tests {
     fn status_title(h: &Harness) -> String {
         let buffer = h.buffer();
         let area = buffer.area;
-        let status = playing_layout(area, status_rows(h.app.mode(), area.height), None).status;
+        let status = playing_layout(
+            area,
+            h.app.cell_size(),
+            status_rows(h.app.mode(), area.height),
+            None,
+        )
+        .status;
         (status.x..status.right())
             .map(|x| buffer[(x, status.y)].symbol())
             .collect()
diff --git a/src/tui/snapshots/chess__tui__panels__tests__playing_start_120x40_vs_jev.snap b/src/tui/snapshots/chess__tui__panels__tests__playing_start_120x40_vs_jev.snap
index 08201ed713602f32d5c88c8c71d4b42a798535c7..78c6185f9e04c7ea676d531ad769bac495008d85 100644
--- a/src/tui/snapshots/chess__tui__panels__tests__playing_start_120x40_vs_jev.snap
+++ b/src/tui/snapshots/chess__tui__panels__tests__playing_start_120x40_vs_jev.snap
@@ -2,43 +2,43 @@
 source: src/tui/panels.rs
 expression: h.terminal.backend()
 ---
-"     ┌ Board ────────────────────────────────────────────────────┐┌ Status ────────────────── You (Black) vs Jev ┐      "
-"     │                                                           ││ White to move (Jev)                          │      "
-"     │                                                           ││ | Jev thinking... 0.0s                       │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │                                                           │├ Jev ─────────────────────────────────────────┤      "
-"     │                                                           ││ Jev ready (jev-test)                         │      "
-"     │ 1   ♜      ♞      ♝      ♚      ♛      ♝      ♞      ♜    ││ no move yet                                  │      "
-"     │                                                           ││                                              │      "
-"     │                                                           │├ Moves ───────────────────────────────────────┤      "
-"     │ 2   ♟︎      ♟︎      ♟︎      ♟︎      ♟︎      ♟︎      ♟︎      ♟︎    ││ no moves yet                                 │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │ 3                                                         ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │ 4                                                         ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │ 5                                                         ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │ 6                                                         ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │ 7   ♟︎      ♟︎      ♟︎      ♟︎      ♟︎      ♟︎      ♟︎      ♟︎    ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │ 8   ♜      ♞      ♝      ♚      ♛      ♝      ♞      ♜    ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │     h      g      f      e      d      c      b      a    ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     │                                                           ││                                              │      "
-"     └───────────────────────────────────────────────────────────┘├ Captured ────────────────────────────────────┤      "
-"     ┌ Command ──────────────────────────────────────────────────┐│ White                                        │      "
-"     │ / move  : command  ? help                                 ││ Black                                        │      "
-"     └───────────────────────────────────────────────────────────┘└──────────────────────────────────────────────┘      "
+"┌ Board ────────────────────────────────────────────────────────────────────┐┌ Status ───────────── You (Black) vs Jev ┐"
+"│                                                                           ││ White to move (Jev)                     │"
+"│                                                                           ││ | Jev thinking... 0.0s                  │"
+"│ 1    ♜        ♞        ♝        ♚        ♛        ♝        ♞        ♜     ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           │├ Jev ────────────────────────────────────┤"
+"│                                                                           ││ Jev ready (jev-test)                    │"
+"│ 2    ♟︎        ♟︎        ♟︎        ♟︎        ♟︎        ♟︎        ♟︎        ♟︎     ││ no move yet                             │"
+"│                                                                           ││                                         │"
+"│                                                                           │├ Moves ──────────────────────────────────┤"
+"│                                                                           ││ no moves yet                            │"
+"│ 3                                                                         ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│ 4                                                                         ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│ 5                                                                         ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│ 6                                                                         ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│ 7    ♟︎        ♟︎        ♟︎        ♟︎        ♟︎        ♟︎        ♟︎        ♟︎     ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│ 8    ♜        ♞        ♝        ♚        ♛        ♝        ♞        ♜     ││                                         │"
+"│                                                                           ││                                         │"
+"│                                                                           ││                                         │"
+"│      h        g        f        e        d        c        b        a     ││                                         │"
+"│                                                                           ││                                         │"
+"└───────────────────────────────────────────────────────────────────────────┘├ Captured ───────────────────────────────┤"
+"┌ Command ──────────────────────────────────────────────────────────────────┐│ White                                   │"
+"│ / move  : command  ? help                                                 ││ Black                                   │"
+"└───────────────────────────────────────────────────────────────────────────┘└─────────────────────────────────────────┘"
````

- [ ] **Step: Run the tests to verify they fail**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: FAIL. The test build does not compile: the tests use names the implementation patch adds, for example "this function takes 1 argument but 2 arguments were supplied"; "this function takes 2 arguments but 3 arguments were supplied"; "this function takes 3 arguments but 4 arguments were supplied"; "cannot find function square_width in this scope". Any other failure (a patch that does not apply, a test that fails at run time) is not the expected RED: stop and report.

- [ ] **Step: Apply the implementation patch**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/tui/app.rs b/src/tui/app.rs
index 57993adef453faa328ffcf7f27b88ac25b6ca0f7..399aed4a790b343b4495b0b49d13752d234622e7 100644
--- a/src/tui/app.rs
+++ b/src/tui/app.rs
@@ -51,7 +51,7 @@ use ratatui::crossterm::event::{
 };
 use ratatui::layout::{Position as CellPosition, Rect};
 
-use super::board::{BoardGeometry, Highlights, square_at};
+use super::board::{BoardGeometry, CellSize, Highlights, square_at};
 use super::event::AppEvent;
 use super::files::{SaveError, pgn_export, resolve_path, tilde_path, today, write_file};
 use super::glyphs::{self, GlyphSet, Palette};
@@ -600,6 +600,8 @@ pub struct App {
     /// The terminal shows no colour (`NO_COLOR`): the board marks highlights with text too.
     no_color: bool,
     glyphs: GlyphSet,
+    /// The terminal's font size, which shapes the board's squares.
+    cell_size: CellSize,
     pick_side: fn() -> Side,
     today: fn() -> String,
     home: Option<PathBuf>,
@@ -673,6 +675,7 @@ impl App {
             palette: glyphs::palette(truecolor),
             no_color: false,
             glyphs,
+            cell_size: CellSize::DEFAULT,
             pick_side: random_side,
             today,
             home: std::env::var_os("HOME").map(PathBuf::from),
@@ -713,6 +716,12 @@ impl App {
         self
     }
 
+    /// Sets the terminal's font size, which is known only once the terminal has been asked
+    /// (default [`CellSize::DEFAULT`]). The next draw shapes the board's squares for it.
+    pub fn set_cell_size(&mut self, cell_size: CellSize) {
+        self.cell_size = cell_size;
+    }
+
     /// Replaces the coin flip used by "Human vs Jev: random side" (tests pass a fixed side).
     #[must_use]
     pub fn with_side_picker(mut self, pick: fn() -> Side) -> App {
@@ -872,6 +881,11 @@ impl App {
         self.no_color
     }
 
+    /// The terminal's font size (see [`App::set_cell_size`]).
+    pub fn cell_size(&self) -> CellSize {
+        self.cell_size
+    }
+
     /// The command box text.
     pub fn command_text(&self) -> &str {
         self.command.text()
diff --git a/src/tui/board.rs b/src/tui/board.rs
index f07c73e1ae01a75392899f496a1f6602f2696d24..986c5ea107626435a030c563b4251d2ae222d30d 100644
--- a/src/tui/board.rs
+++ b/src/tui/board.rs
@@ -1,10 +1,14 @@
-//! Board widget and flip-aware hit-testing (spec section 6.3).
+//! Board widget and flip-aware hit-testing (spec sections 6.3 and 9.2).
 //!
 //! [`layout_board`] picks a square size and places the board inside an area;
 //! the resulting [`BoardGeometry`] is used both to draw ([`BoardView`]) and to
 //! map mouse cells back to squares ([`square_at`]), so the App must keep the
 //! geometry from its most recent draw for hit-testing.
 //!
+//! Squares are as tall as the area allows, and as wide as the font makes them
+//! look square: [`square_width`] turns a height in rows into a width in columns
+//! from the terminal's [`CellSize`].
+//!
 //! The widget draws only the 8×8 grid plus a rank-label column on the left and
 //! a file-label row underneath; any enclosing block is the caller's.
 
@@ -18,10 +22,63 @@ use ratatui::{
 use super::glyphs::{self, GlyphSet, Palette};
 use crate::core::{Color as Side, PieceKind, Position as ChessPosition, Square};
 
-/// Square sizes `(width, height)` in cells, largest first. Terminal cells are
-/// about twice as tall as wide, so these look roughly square, and the odd
-/// widths let the one-cell glyph sit exactly in the middle.
-pub const SQUARE_SIZES: [(u16, u16); 3] = [(7, 3), (5, 2), (3, 1)];
+/// The smallest square `(width, height)` in cells: one row, with room for the
+/// glyph and the cursor's `[` `]` on either side of it. A font so narrow that
+/// even one-row squares come out too wide for the area still gets this size.
+pub const MIN_SQUARE: (u16, u16) = (3, 1);
+
+/// A terminal cell's size in pixels, which is the font size. It makes squares
+/// look square: see [`square_width`].
+#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
+pub struct CellSize {
+    width: u16,
+    height: u16,
+}
+
+impl CellSize {
+    /// 10×20 pixels, assumed when the terminal does not report its font size.
+    pub const DEFAULT: CellSize = CellSize {
+        width: 10,
+        height: 20,
+    };
+
+    /// A cell `width` × `height` pixels, or [`CellSize::DEFAULT`] when either is
+    /// zero (a terminal that does not know its size reports zero).
+    pub const fn new(width: u16, height: u16) -> CellSize {
+        if width == 0 || height == 0 {
+            CellSize::DEFAULT
+        } else {
+            CellSize { width, height }
+        }
+    }
+
+    /// Width in pixels (never zero).
+    pub const fn width(self) -> u16 {
+        self.width
+    }
+
+    /// Height in pixels (never zero).
+    pub const fn height(self) -> u16 {
+        self.height
+    }
+}
+
+impl Default for CellSize {
+    fn default() -> CellSize {
+        CellSize::DEFAULT
+    }
+}
+
+/// The width in cells of a square `square_h` rows high, so that it looks square
+/// in pixels: `square_h × cell height / cell width`, rounded, at least 3, and
+/// bumped up to the next odd number so a one-cell glyph sits in the middle
+/// column. With the default 10×20 font that is `2 × square_h + 1`.
+pub fn square_width(square_h: u16, cell: CellSize) -> u16 {
+    let (cell_w, cell_h) = (u64::from(cell.width), u64::from(cell.height));
+    // Rounded to the nearest whole column, halves up; u64 cannot overflow here.
+    let columns = (2 * u64::from(square_h) * cell_h + cell_w) / (2 * cell_w);
+    u16::try_from(columns.max(3) | 1).unwrap_or(u16::MAX)
+}
 
 /// Width of the rank-label column left of the grid.
 const LABEL_COLUMNS: u16 = 1;
@@ -38,33 +95,45 @@ pub struct BoardGeometry {
     pub outer: Rect,
     /// The 8×8 squares only: `8 * square_w` by `8 * square_h` cells.
     pub grid: Rect,
-    /// Width of one square in cells (7, 5 or 3).
+    /// Width of one square in cells: odd, at least 3 (see [`square_width`]).
     pub square_w: u16,
-    /// Height of one square in cells (3, 2 or 1).
+    /// Height of one square in cells: at least 1.
     pub square_h: u16,
     /// Black at the bottom when true; White at the bottom otherwise.
     pub flipped: bool,
 }
 
-/// Lays the board out in `area` with the largest square size from
-/// [`SQUARE_SIZES`] that fits (labels included), centred in both directions.
-/// Returns `None` when even 3×1 squares do not fit (25×9 cells).
-pub fn layout_board(area: Rect, flipped: bool) -> Option<BoardGeometry> {
-    SQUARE_SIZES.into_iter().find_map(|(square_w, square_h)| {
-        let outer_w = 8 * square_w + LABEL_COLUMNS;
-        let outer_h = 8 * square_h + LABEL_ROWS;
-        if area.width < outer_w || area.height < outer_h {
-            return None;
-        }
-        let x = area.x + (area.width - outer_w) / 2;
-        let y = area.y + (area.height - outer_h) / 2;
-        Some(BoardGeometry {
-            outer: Rect::new(x, y, outer_w, outer_h),
-            grid: Rect::new(x + LABEL_COLUMNS, y, 8 * square_w, 8 * square_h),
-            square_w,
-            square_h,
-            flipped,
-        })
+/// Lays the board out in `area` with the tallest squares that fit (labels
+/// included), each [`square_width`] wide for the font `cell`, centred in both
+/// directions: a board limited by the area's width is centred vertically.
+///
+/// When even one-row squares are too wide for the area (a very narrow font),
+/// the board uses [`MIN_SQUARE`]. Returns `None` when that does not fit either
+/// (25×9 cells).
+pub fn layout_board(area: Rect, flipped: bool, cell: CellSize) -> Option<BoardGeometry> {
+    // Grid plus labels, in u32 so wide squares cannot overflow.
+    let fits = |square_w: u16, square_h: u16| {
+        8 * u32::from(square_w) + u32::from(LABEL_COLUMNS) <= u32::from(area.width)
+            && 8 * u32::from(square_h) + u32::from(LABEL_ROWS) <= u32::from(area.height)
+    };
+    // The width grows with the height, so the first height that fits is the tallest.
+    let tallest = area.height.saturating_sub(LABEL_ROWS) / 8;
+    let (square_w, square_h) = (1..=tallest)
+        .rev()
+        .map(|square_h| (square_width(square_h, cell), square_h))
+        .chain([MIN_SQUARE])
+        .find(|&(square_w, square_h)| fits(square_w, square_h))?;
+    // `fits` bounds both below the area's size, so these cannot overflow.
+    let (grid_w, grid_h) = (8 * square_w, 8 * square_h);
+    let (outer_w, outer_h) = (grid_w + LABEL_COLUMNS, grid_h + LABEL_ROWS);
+    let x = area.x + (area.width - outer_w) / 2;
+    let y = area.y + (area.height - outer_h) / 2;
+    Some(BoardGeometry {
+        outer: Rect::new(x, y, outer_w, outer_h),
+        grid: Rect::new(x + LABEL_COLUMNS, y, grid_w, grid_h),
+        square_w,
+        square_h,
+        flipped,
     })
 }
 
diff --git a/src/tui/panels.rs b/src/tui/panels.rs
index 28a952632218b4a66740d602aa1fccf21279a6cb..b968b4c6b86c1401d67097caaab6baa9d7397c53 100644
--- a/src/tui/panels.rs
+++ b/src/tui/panels.rs
@@ -1,20 +1,23 @@
-//! Drawing every screen (spec sections 6.2 and 6.3).
+//! Drawing every screen (spec sections 6.2, 6.3 and 9.2).
 //!
 //! [`draw`] renders an [`App`] through its public accessors only: the menu, the playing
 //! screen's panels, the game-over overlay, the top dialog and the too-small notice. It
 //! returns the [`HitMap`] of everything clickable, which the app keeps for the next mouse
 //! event, and the move-list scroll clamped to what the list can show.
 //!
-//! Playing layout: the left column holds the Board block, sized for the largest board that
-//! fits beside the narrowest side column, with the Command box under it. The right column
-//! stacks Status, the computer's panel (titled "Jev" or "Local search", only in games
-//! against the computer), Moves and Captured with shared borders. On wide terminals the
-//! right column stops growing at [`SIDE_MAX_WIDTH`] and the whole layout is centred.
+//! Playing layout: the screen fills the terminal. The left column holds the Board block,
+//! sized for the largest board that fits beside the narrowest side column
+//! ([`SIDE_MIN_WIDTH`]) with squares shaped for the font ([`App::cell_size`]), and the
+//! Command box under it. The right column takes every column left over and stacks Status,
+//! the computer's panel (titled "Jev" or "Local search", only in games against the
+//! computer), Moves and Captured with shared borders. A board limited by the width is
+//! centred vertically in its block, which still fills the column.
 //!
-//! Status has a fixed height per mode and terminal height, so it never jumps. The Jev panel
-//! grows with its text (the source, Jev's top three, confidence, latency, model and the
-//! note) and takes the rows from Moves, which keeps at least [`MOVES_MIN_ROWS`]; when even
-//! that is not enough, only the note is cut short, ending in `…`.
+//! Status and Captured have fixed heights (Status per mode and terminal height), so they
+//! never jump. The Jev panel grows with its text (the source, Jev's top three, confidence,
+//! latency, model and the note) and takes the rows from Moves, which gets the rest and
+//! keeps at least [`MOVES_MIN_ROWS`]; when even that is not enough, only the note is cut
+//! short, ending in `…`. Menu, dialogs, help and the game-over overlay stay centred boxes.
 //!
 //! The snapshot tests at the bottom of this file render every screen; their files in
 //! `src/tui/snapshots/` (`chess__tui__panels__tests__*.snap`) show the exact output at
@@ -40,7 +43,7 @@ use super::app::{
     Message, Mode, PROMOTION_CHOICES, Question, Screen, SidePick, TOO_SMALL, WAITING_FOR_ENGINE,
     is_too_small, move_rows, outcome_text,
 };
-use super::board::{BoardGeometry, BoardView, layout_board};
+use super::board::{BoardGeometry, BoardView, CellSize, layout_board};
 use super::glyphs::{self, ELLIPSIS, GlyphSet, Palette, char_width};
 use super::input::LineEditor;
 use crate::core::{Color as Side, Game, Piece, PieceKind, Position as ChessPosition};
@@ -67,10 +70,9 @@ pub const HELP_LINES: [&str; 13] = [
 /// Width of the key column in [`HELP_LINES`].
 pub const HELP_KEY_WIDTH: usize = 10;
 
-/// Narrowest right column; the board shrinks before the column does.
+/// Narrowest right column; the board shrinks before the column does. The column takes
+/// every column the board leaves.
 pub const SIDE_MIN_WIDTH: u16 = 30;
-/// Widest right column; wider terminals centre the layout instead.
-pub const SIDE_MAX_WIDTH: u16 = 48;
 /// Height of the Command box (one text row between borders).
 const COMMAND_HEIGHT: u16 = 3;
 /// Side panel chrome across: two borders and the blank column on the left.
@@ -303,11 +305,11 @@ struct PlayingLayout {
     captured: Rect,
 }
 
-/// The playing screen's two columns in `area`: the left edge of the layout, the board
-/// column's width and the right column's width.
-fn columns(area: Rect) -> (u16, u16, u16) {
+/// The playing screen's two columns in `area`, which together take its full width: the
+/// board column's width and the right column's width.
+fn columns(area: Rect, cell: CellSize) -> (u16, u16) {
     // The largest board whose block fits beside the narrowest right column and above the
-    // command box.
+    // command box; the block is exactly as wide as that board.
     let room = Rect::new(
         0,
         0,
@@ -316,21 +318,15 @@ fn columns(area: Rect) -> (u16, u16, u16) {
         area.height
             .saturating_sub(COMMAND_HEIGHT.saturating_add(BOARD_CHROME_HEIGHT)),
     );
-    let board_width = layout_board(room, false).map_or(area.width / 2, |g| {
+    let board_width = layout_board(room, false, cell).map_or(area.width / 2, |g| {
         g.outer.width.saturating_add(BOARD_CHROME_WIDTH)
     });
-    let side_width = area.width.saturating_sub(board_width).min(SIDE_MAX_WIDTH);
-    let x = area.x
-        + area
-            .width
-            .saturating_sub(board_width.saturating_add(side_width))
-            / 2;
-    (x, board_width, side_width)
+    (board_width, area.width.saturating_sub(board_width))
 }
 
 /// Text width inside the right column's panels.
-fn side_text_width(area: Rect) -> u16 {
-    let (_, _, side_width) = columns(area);
+fn side_text_width(area: Rect, cell: CellSize) -> u16 {
+    let (_, side_width) = columns(area, cell);
     side_width.saturating_sub(SIDE_CHROME_WIDTH)
 }
 
@@ -346,14 +342,20 @@ fn status_rows(mode: Mode, height: u16) -> u16 {
     }
 }
 
-/// Lays out the playing screen in `area` (not [`is_too_small`]). The Status panel gets
-/// `status_rows` text rows; the Jev panel, when `jev_rows` is given, gets that many (at
-/// least [`JEV_MIN_ROWS`]) as long as Moves keeps [`MOVES_MIN_ROWS`]; Moves gets the rest.
-fn playing_layout(area: Rect, status_rows: u16, jev_rows: Option<u16>) -> PlayingLayout {
-    let (x, board_width, side_width) = columns(area);
+/// Lays out the playing screen in `area` (not [`is_too_small`]) for the font `cell`, using
+/// every cell of it. The Status panel gets `status_rows` text rows and Captured
+/// [`CAPTURED_ROWS`]; the Jev panel, when `jev_rows` is given, gets that many (at least
+/// [`JEV_MIN_ROWS`]) as long as Moves keeps [`MOVES_MIN_ROWS`]; Moves gets the rest.
+fn playing_layout(
+    area: Rect,
+    cell: CellSize,
+    status_rows: u16,
+    jev_rows: Option<u16>,
+) -> PlayingLayout {
+    let (board_width, side_width) = columns(area, cell);
     let command_height = COMMAND_HEIGHT.min(area.height);
-    let board = Rect::new(x, area.y, board_width, area.height - command_height);
-    let command = Rect::new(x, board.bottom(), board_width, command_height);
+    let board = Rect::new(area.x, area.y, board_width, area.height - command_height);
+    let command = Rect::new(area.x, board.bottom(), board_width, command_height);
 
     let right = Rect::new(board.right(), area.y, side_width, area.height);
     let panels: u16 = if jev_rows.is_some() { 4 } else { 3 };
@@ -389,11 +391,12 @@ fn playing_layout(area: Rect, status_rows: u16, jev_rows: Option<u16>) -> Playin
 
 /// The board, the command box and the side panels.
 fn playing(frame: &mut Frame, area: Rect, app: &App, now: Instant, drawn: &mut Drawn) {
-    let text_width = side_text_width(area);
+    let text_width = side_text_width(area, app.cell_size());
     let jev = (app.mode() != Mode::HumanVsHuman)
         .then(|| JevText::new(app.last_computer(), app.engine_status(), text_width));
     let layout = playing_layout(
         area,
+        app.cell_size(),
         status_rows(app.mode(), area.height),
         jev.as_ref().map(JevText::rows),
     );
@@ -433,7 +436,7 @@ fn board_panel(frame: &mut Frame, area: Rect, app: &App) -> Option<BoardGeometry
         .padding(Padding::horizontal(1));
     let inner = block.inner(area);
     frame.render_widget(block, area);
-    let geometry = layout_board(inner, app.flipped())?;
+    let geometry = layout_board(inner, app.flipped(), app.cell_size())?;
     let highlights = app.highlights();
     frame.render_widget(
         BoardView {
````

- [ ] **Step: Run the tests to verify they pass**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::` and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test --lib engine::`
Expected: tui:: 314 passed, engine:: 109 passed; `find src -name '*.snap.new'` prints nothing.

Run: `cargo fmt --check` and `cargo clippy --all-targets 2>&1 | grep -A5 -E "src/tui|src/engine|src/main.rs"`
Expected: no output from either.

- [ ] **Step: Update the handoff file**

Edit `docs/handoff/HANDOFF.md`. In `## Current`, set:

```
Sub-project: tui-polish | Plan: docs/superpowers/plans/2026-09-27-tui-polish.md
Branch: feat/tui-polish
Last completed task: tui-polish task 3 (Full-screen layout with font-shaped squares)
Next task: tui-polish task 4 (graphics detection and the image style)
State: green (except 3 pre-existing old-code test failures)
```

Set `## Verify before continuing` to:

```
env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui:: && cargo test --lib engine::
```

Add at the top of `## Log (newest first)`: `- <today's date> tui-polish task 3 done: Full-screen layout with font-shaped squares`. Keep every other section. Record under `## Notes / decisions made during work` any decision this plan did not specify.

- [ ] **Step: Refresh the knowledge graph**

Run: `graphify update .`
Expected: completes without error. Never commit `graphify-out/`, `.claude/` or `.superpowers/`.

- [ ] **Step: Commit**

```bash
git add docs/handoff/HANDOFF.md src/tui/app.rs src/tui/board.rs src/tui/panels.rs src/tui/snapshots/chess__tui__panels__tests__playing_start_120x40_vs_jev.snap
git commit -m "feat(tui): fill the terminal and shape the board's squares for the font

Co-Authored-By: <the model you are> <noreply@anthropic.com>"
```

Replace `<the model you are>` with your own model name.

---

### Task 4: Graphics detection and the Image style

**Files:**
- Create: `src/tui/graphics.rs`
- Modify: `Cargo.lock`, `Cargo.toml`, `src/tui/app.rs`, `src/tui/glyphs.rs`, `src/tui/mod.rs`, `src/tui/panels.rs`, `src/tui/snapshots/chess__tui__panels__tests__menu_80x24.snap`, `src/tui/terminal.rs`, `src/tui/test_support/mod.rs`

**Interfaces:**
- Consumes: `board::CellSize`, `glyphs::{GlyphSet, initial_glyphs}`, `terminal::enter`, ratatui-image `cap_parser::{Parser, QueryStdioOptions, Response}` and `picker::{Picker, ProtocolType}`, `rustix::event::poll`.
- Produces (`src/tui/graphics.rs`): `QUERY_TIMEOUT` (1 s); `Graphics { picker: Option<Picker>, cell_size: CellSize, warning: Option<String>, answers_pending: bool }` with `off`, `without_images`, `images_available`, `support`, `picker`; `detect(stop: impl Fn() -> bool) -> Graphics` (poll-based, no thread); `picker_for(ProtocolType, CellSize) -> Picker`; `LateAnswers` (`after`, `keep`) that drops a late kitty answer's key presses.
- `glyphs`: `GlyphSet::Image` (first in `ALL`), `GlyphSet::next(self, images: bool)`, `IMAGES_ENV` (`RCHESS_IMAGES`), `images_off`, `images_wanted`, `ImageSupport`.
- `terminal::enter<T>(before_input: impl FnOnce() -> T) -> io::Result<(DefaultTerminal, T)>`: the closure runs after raw mode and the alternate screen, before mouse capture and bracketed paste.
- `App::with_picker(Option<Picker>)`, `App::picker()`; `--glyphs image`; menu warning on query failure.
- Cargo: `rustix` 1 (`event`).

- [ ] **Step: Confirm the starting point**

Run: `git status --short` (only untracked `.claude/`, `graphify-out/` and user files) and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: 314 passed (engine: `cargo test --lib engine::` 109 passed).

- [ ] **Step: Apply the tests patch (write the failing tests)**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/tui/app.rs b/src/tui/app.rs
index 399aed4a790b343b4495b0b49d13752d234622e7..a8c522e68b2494799d8711cc64825441f8c3a9a1 100644
--- a/src/tui/app.rs
+++ b/src/tui/app.rs
@@ -2309,10 +2309,12 @@ mod tests {
     use std::sync::mpsc;
 
     use ratatui::style::Modifier;
+    use ratatui_image::picker::ProtocolType;
 
     use super::*;
     use crate::core::{Position as ChessPosition, START_FEN};
     use crate::engine::{MoveSource, analyse};
+    use crate::tui::graphics;
     use crate::tui::panels::HELP_LINES;
     use crate::tui::test_support::engine::{FakeEngine, REPLY_TIMEOUT, chord, key, mouse, paste};
     use crate::tui::test_support::harness::{Harness, request};
@@ -3116,6 +3118,60 @@ mod tests {
         assert_eq!(h.app.screen_name(), "menu", "no moves: nothing to lose");
     }
 
+    #[test]
+    fn without_images_the_glyph_cycle_has_only_text_styles() {
+        let mut h = hvh();
+        assert!(!h.app.images_available());
+        assert!(h.app.picker().is_none());
+        let mut seen = Vec::new();
+        for _ in 0..4 {
+            h.char('g');
+            seen.push(h.app.glyphs());
+        }
+        assert_eq!(
+            seen,
+            [
+                GlyphSet::Outline,
+                GlyphSet::Ascii,
+                GlyphSet::Solid,
+                GlyphSet::Outline
+            ]
+        );
+    }
+
+    #[test]
+    fn with_a_picker_the_image_style_joins_the_glyph_cycle() {
+        let picker = graphics::picker_for(ProtocolType::Halfblocks, CellSize::DEFAULT);
+        let mut h = Harness::build(FakeEngine::local(), (80, 24), Vec::new(), |app| {
+            app.with_picker(Some(picker))
+        });
+        h.char('1');
+        assert!(h.app.images_available());
+        assert_eq!(
+            h.app.picker().map(|picker| picker.protocol_type()),
+            Some(ProtocolType::Halfblocks)
+        );
+        let mut seen = Vec::new();
+        for _ in 0..4 {
+            h.char('g');
+            seen.push(h.app.glyphs());
+        }
+        assert_eq!(
+            seen,
+            [
+                GlyphSet::Outline,
+                GlyphSet::Ascii,
+                GlyphSet::Image,
+                GlyphSet::Solid
+            ]
+        );
+        h.command(":glyphs");
+        h.command(":glyphs");
+        h.command(":glyphs");
+        assert_eq!(h.app.glyphs(), GlyphSet::Image);
+        assert_eq!(h.app.status_line(), "glyphs: image");
+    }
+
     #[test]
     fn new_game_and_menu_ask_while_a_game_is_in_progress() {
         let mut h = hvh();
diff --git a/src/tui/glyphs.rs b/src/tui/glyphs.rs
index 2db533fffb2a38d6573807e93fad39f75fafa2b3..9d6169c1858f6e6440cfc81c5fc4e655799f7569 100644
--- a/src/tui/glyphs.rs
+++ b/src/tui/glyphs.rs
@@ -350,6 +350,9 @@ mod tests {
             .flat_map(|color| PieceKind::ALL.map(|kind| Piece::new(color, kind)))
     }
 
+    /// No graphics query ran: the text styles only.
+    const OFF: ImageSupport = ImageSupport::Off;
+
     fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
         let map: HashMap<String, String> = pairs
             .iter()
@@ -372,11 +375,29 @@ mod tests {
     }
 
     #[test]
-    fn next_cycles_through_all_sets() {
-        assert_eq!(GlyphSet::Solid.next(), GlyphSet::Outline);
-        assert_eq!(GlyphSet::Outline.next(), GlyphSet::Ascii);
-        assert_eq!(GlyphSet::Ascii.next(), GlyphSet::Solid);
+    fn next_cycles_through_all_sets_with_images() {
+        assert_eq!(GlyphSet::Image.next(true), GlyphSet::Solid);
+        assert_eq!(GlyphSet::Solid.next(true), GlyphSet::Outline);
+        assert_eq!(GlyphSet::Outline.next(true), GlyphSet::Ascii);
+        assert_eq!(GlyphSet::Ascii.next(true), GlyphSet::Image);
         assert_eq!(GlyphSet::default(), GlyphSet::Solid);
+        assert_eq!(GlyphSet::ALL[0], GlyphSet::Image, "Image comes first");
+    }
+
+    #[test]
+    fn next_skips_the_image_style_without_images() {
+        assert_eq!(GlyphSet::Solid.next(false), GlyphSet::Outline);
+        assert_eq!(GlyphSet::Outline.next(false), GlyphSet::Ascii);
+        assert_eq!(GlyphSet::Ascii.next(false), GlyphSet::Solid);
+        assert_eq!(GlyphSet::Image.next(false), GlyphSet::Solid);
+        // Starting anywhere, a full turn visits each text set once and never Image.
+        let mut set = GlyphSet::Solid;
+        let mut seen = Vec::new();
+        for _ in 0..3 {
+            set = set.next(false);
+            seen.push(set);
+        }
+        assert_eq!(seen, [GlyphSet::Outline, GlyphSet::Ascii, GlyphSet::Solid]);
     }
 
     #[test]
@@ -387,6 +408,8 @@ mod tests {
             assert_eq!(set.to_string(), set.name());
         }
         assert_eq!(GlyphSet::from_name("Outline"), Some(GlyphSet::Outline));
+        assert_eq!(GlyphSet::from_name("image"), Some(GlyphSet::Image));
+        assert_eq!(GlyphSet::Image.to_string(), "image");
         assert_eq!(GlyphSet::from_name(" ascii\n"), Some(GlyphSet::Ascii));
         assert_eq!(GlyphSet::from_name(""), None);
         assert_eq!(GlyphSet::from_name("fancy"), None);
@@ -433,6 +456,14 @@ mod tests {
         }
     }
 
+    #[test]
+    fn the_image_style_writes_solid_glyphs_where_it_draws_text() {
+        // The Captured panel, and squares too small for a picture.
+        for piece in all_pieces() {
+            assert_eq!(glyph(GlyphSet::Image, piece), glyph(GlyphSet::Solid, piece));
+        }
+    }
+
     #[test]
     fn ascii_matches_fen_letters() {
         for piece in all_pieces() {
@@ -449,7 +480,11 @@ mod tests {
             let mut seen: Vec<&str> = all_pieces().map(|p| glyph(set, p)).collect();
             seen.sort_unstable();
             seen.dedup();
-            let expected = if set == GlyphSet::Solid { 6 } else { 12 };
+            let expected = if matches!(set, GlyphSet::Solid | GlyphSet::Image) {
+                6
+            } else {
+                12
+            };
             assert_eq!(seen.len(), expected, "{set}");
         }
     }
@@ -592,9 +627,12 @@ mod tests {
 
     #[test]
     fn initial_glyphs_defaults_to_solid() {
-        assert_eq!(initial_glyphs(None, env(&[])), (GlyphSet::Solid, vec![]));
         assert_eq!(
-            initial_glyphs(None, env(&[(GLYPHS_ENV, "  ")])),
+            initial_glyphs(None, env(&[]), OFF),
+            (GlyphSet::Solid, vec![])
+        );
+        assert_eq!(
+            initial_glyphs(None, env(&[(GLYPHS_ENV, "  ")]), OFF),
             (GlyphSet::Solid, vec![])
         );
     }
@@ -603,7 +641,7 @@ mod tests {
     fn cli_wins_over_environment() {
         let get = env(&[(GLYPHS_ENV, "outline"), ("NO_COLOR", "1")]);
         assert_eq!(
-            initial_glyphs(Some("ASCII"), get),
+            initial_glyphs(Some("ASCII"), get, OFF),
             (GlyphSet::Ascii, vec![])
         );
     }
@@ -611,25 +649,25 @@ mod tests {
     #[test]
     fn environment_used_without_cli() {
         let get = env(&[(GLYPHS_ENV, "Ascii")]);
-        assert_eq!(initial_glyphs(None, get), (GlyphSet::Ascii, vec![]));
+        assert_eq!(initial_glyphs(None, get, OFF), (GlyphSet::Ascii, vec![]));
     }
 
     #[test]
     fn no_color_switches_to_outline_unless_chosen() {
         assert_eq!(
-            initial_glyphs(None, env(&[("NO_COLOR", "1")])),
+            initial_glyphs(None, env(&[("NO_COLOR", "1")]), OFF),
             (GlyphSet::Outline, vec![])
         );
         assert_eq!(
-            initial_glyphs(None, env(&[("NO_COLOR", "")])),
+            initial_glyphs(None, env(&[("NO_COLOR", "")]), OFF),
             (GlyphSet::Solid, vec![])
         );
         assert_eq!(
-            initial_glyphs(Some("solid"), env(&[("NO_COLOR", "1")])),
+            initial_glyphs(Some("solid"), env(&[("NO_COLOR", "1")]), OFF),
             (GlyphSet::Solid, vec![])
         );
         assert_eq!(
-            initial_glyphs(None, env(&[("NO_COLOR", "1"), (GLYPHS_ENV, "solid")])),
+            initial_glyphs(None, env(&[("NO_COLOR", "1"), (GLYPHS_ENV, "solid")]), OFF),
             (GlyphSet::Solid, vec![])
         );
     }
@@ -644,12 +682,12 @@ mod tests {
 
     #[test]
     fn invalid_cli_value_warns_and_falls_back_to_environment() {
-        let (set, warnings) = initial_glyphs(Some("fancy"), env(&[(GLYPHS_ENV, "ascii")]));
+        let (set, warnings) = initial_glyphs(Some("fancy"), env(&[(GLYPHS_ENV, "ascii")]), OFF);
         assert_eq!(set, GlyphSet::Ascii);
         assert_eq!(
             warnings,
             vec![
-                "--glyphs: unknown glyph set \"fancy\" (expected solid, outline or ascii); \
+                "--glyphs: unknown glyph set \"fancy\" (expected image, solid, outline or ascii); \
                  using ascii"
             ]
         );
@@ -658,27 +696,140 @@ mod tests {
     #[test]
     fn invalid_values_everywhere_fall_back_to_default() {
         let get = env(&[(GLYPHS_ENV, "bold"), ("NO_COLOR", "yes")]);
-        let (set, warnings) = initial_glyphs(Some(""), get);
+        let (set, warnings) = initial_glyphs(Some(""), get, OFF);
         assert_eq!(set, GlyphSet::Outline);
         assert_eq!(
             warnings,
             vec![
-                "--glyphs: unknown glyph set \"\" (expected solid, outline or ascii); \
+                "--glyphs: unknown glyph set \"\" (expected image, solid, outline or ascii); \
                  using outline",
-                "RCHESS_GLYPHS: unknown glyph set \"bold\" (expected solid, outline or ascii); \
+                "RCHESS_GLYPHS: unknown glyph set \"bold\" (expected image, solid, outline or ascii); \
                  using outline",
             ]
         );
     }
 
+    #[test]
+    fn the_image_style_is_the_default_only_with_a_graphics_protocol() {
+        assert_eq!(
+            initial_glyphs(None, env(&[]), ImageSupport::Protocol),
+            (GlyphSet::Image, vec![])
+        );
+        assert_eq!(
+            initial_glyphs(None, env(&[]), ImageSupport::Halfblocks),
+            (GlyphSet::Solid, vec![])
+        );
+        assert_eq!(
+            initial_glyphs(None, env(&[]), OFF),
+            (GlyphSet::Solid, vec![])
+        );
+    }
+
+    #[test]
+    fn a_named_style_wins_over_the_image_default() {
+        let get = env(&[(GLYPHS_ENV, "outline")]);
+        assert_eq!(
+            initial_glyphs(Some("ascii"), &get, ImageSupport::Protocol),
+            (GlyphSet::Ascii, vec![])
+        );
+        assert_eq!(
+            initial_glyphs(None, &get, ImageSupport::Protocol),
+            (GlyphSet::Outline, vec![])
+        );
+    }
+
+    #[test]
+    fn the_image_style_can_be_named_whenever_the_query_ran() {
+        for images in [ImageSupport::Halfblocks, ImageSupport::Protocol] {
+            assert_eq!(
+                initial_glyphs(Some("image"), env(&[]), images),
+                (GlyphSet::Image, vec![])
+            );
+            assert_eq!(
+                initial_glyphs(None, env(&[(GLYPHS_ENV, "IMAGE")]), images),
+                (GlyphSet::Image, vec![])
+            );
+        }
+    }
+
+    #[test]
+    fn a_named_image_style_without_images_warns_and_falls_through() {
+        assert_eq!(
+            initial_glyphs(Some("image"), env(&[("NO_COLOR", "1")]), OFF),
+            (
+                GlyphSet::Outline,
+                vec!["--glyphs: images are off (NO_COLOR is set); using outline".to_string()]
+            )
+        );
+        assert_eq!(
+            initial_glyphs(
+                None,
+                env(&[(GLYPHS_ENV, "image"), (IMAGES_ENV, "off")]),
+                OFF
+            ),
+            (
+                GlyphSet::Solid,
+                vec!["RCHESS_GLYPHS: images are off (RCHESS_IMAGES=off); using solid".to_string()]
+            )
+        );
+        // The environment still gets its turn after a refused --glyphs.
+        let get = env(&[(GLYPHS_ENV, "ascii"), (IMAGES_ENV, "off")]);
+        let (set, warnings) = initial_glyphs(Some("image"), get, OFF);
+        assert_eq!(set, GlyphSet::Ascii);
+        assert_eq!(
+            warnings,
+            ["--glyphs: images are off (RCHESS_IMAGES=off); using ascii"]
+        );
+    }
+
+    #[test]
+    fn images_are_wanted_unless_a_text_style_is_named() {
+        assert!(images_wanted(None, env(&[])));
+        assert!(images_wanted(Some("image"), env(&[])));
+        assert!(images_wanted(None, env(&[(GLYPHS_ENV, "Image")])));
+        assert!(
+            images_wanted(None, env(&[(GLYPHS_ENV, " ")])),
+            "empty is unset"
+        );
+        for set in ["solid", "outline", "ascii"] {
+            assert!(!images_wanted(Some(set), env(&[])), "{set}");
+            assert!(!images_wanted(None, env(&[(GLYPHS_ENV, set)])), "{set}");
+        }
+        // The first valid name counts, as in `initial_glyphs`.
+        assert!(images_wanted(Some("image"), env(&[(GLYPHS_ENV, "ascii")])));
+        assert!(!images_wanted(Some("ascii"), env(&[(GLYPHS_ENV, "image")])));
+        assert!(!images_wanted(Some("fancy"), env(&[(GLYPHS_ENV, "ascii")])));
+        assert!(images_wanted(Some("fancy"), env(&[(GLYPHS_ENV, "bold")])));
+    }
+
+    #[test]
+    fn no_color_and_rchess_images_off_turn_images_off() {
+        assert!(!images_wanted(None, env(&[("NO_COLOR", "1")])));
+        assert!(!images_wanted(Some("image"), env(&[("NO_COLOR", "1")])));
+        assert!(images_wanted(None, env(&[("NO_COLOR", "")])));
+        for off in ["off", "OFF", " Off\n"] {
+            let pairs = [(IMAGES_ENV, off)];
+            let get = env(&pairs);
+            assert!(images_off(&get), "{off:?}");
+            assert!(!images_wanted(Some("image"), get), "{off:?}");
+        }
+        for on in ["", "on", "1", "offline"] {
+            let pairs = [(IMAGES_ENV, on)];
+            let get = env(&pairs);
+            assert!(!images_off(&get), "{on:?}");
+            assert!(images_wanted(None, get), "{on:?}");
+        }
+        assert!(!images_off(env(&[])));
+    }
+
     #[test]
     fn warnings_escape_and_shorten_echoed_input() {
-        let (_, warnings) = initial_glyphs(Some("\u{1b}[2Jx"), env(&[]));
+        let (_, warnings) = initial_glyphs(Some("\u{1b}[2Jx"), env(&[]), OFF);
         assert!(warnings[0].contains(r#""\u{1b}[2Jx""#), "{}", warnings[0]);
         assert!(!warnings[0].contains('\u{1b}'));
 
         let long = "x".repeat(100);
-        let (_, warnings) = initial_glyphs(Some(&long), env(&[]));
+        let (_, warnings) = initial_glyphs(Some(&long), env(&[]), OFF);
         assert!(warnings[0].contains(&format!("\"{}…\"", "x".repeat(24))));
     }
 }
diff --git a/src/tui/graphics.rs b/src/tui/graphics.rs
new file mode 100644
index 0000000000000000000000000000000000000000..6dc2c4189f978c3c7d3737f7eed1037e1d2864a9
--- /dev/null
+++ b/src/tui/graphics.rs
@@ -0,0 +1,447 @@
+//! Graphics detection (spec 9.3): which image protocol the terminal speaks and how
+//! large its font is, found out once at start-up.
+//!
+//! [`detect`] writes ratatui-image's capability query ([`Parser::query`]) and reads the
+//! answers itself, on the UI thread: it polls stdin with a deadline of
+//! [`QUERY_TIMEOUT`] and feeds each byte to [`Parser::push`] until the status report
+//! that ends the answers. It never starts a thread, so nothing is left reading stdin
+//! when a terminal does not answer (`Picker::from_query_stdio` would leave one behind,
+//! swallowing keystrokes), and it stops reading right after the status report, so keys
+//! typed after it reach the event loop.
+//!
+//! The answers map to a protocol the way ratatui-image maps them: Kitty when the
+//! terminal accepted the kitty graphics probe, else Sixel when its device attributes
+//! list sixel, else iTerm2 when the environment says so (WezTerm, iTerm2 and a few
+//! others), else half-blocks. WezTerm and Konsole are never sent the Kitty and Sixel
+//! probes (neither draws those correctly). A protocol needs a real font size; it comes
+//! from the cell-size answer, else from the window's pixel size, and without either the
+//! picker draws half-blocks at 10×20. A query that fails or times out never stops the
+//! program: it falls back to half-blocks with a warning for the menu.
+
+#[cfg(test)]
+mod tests {
+    use std::cell::Cell;
+    use std::collections::{HashMap, VecDeque};
+    use std::io;
+    use std::time::{Duration, Instant};
+
+    use ratatui::crossterm::terminal::WindowSize;
+    use ratatui_image::picker::ProtocolType;
+    use ratatui_image::picker::cap_parser::Response;
+
+    use super::*;
+    use crate::tui::board::CellSize;
+    use crate::tui::glyphs::ImageSupport;
+
+    /// Kitty 0.39: the graphics probe is accepted, no sixel, 9×18 cells.
+    const KITTY: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c\x1b[6;18;9t\x1b[0n";
+    /// Ghostty-like: kitty graphics, more device attributes, a 17×38 font on a HiDPI screen.
+    const GHOSTTY: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22;52c\x1b[6;38;17t\x1b[0n";
+    /// WezTerm-like: asked only for the cell size (Kitty and Sixel are not probed).
+    const WEZTERM: &[u8] = b"\x1b[6;16;8t\x1b[0n";
+    /// A sixel terminal (xterm -ti vt340, foot): no kitty answer, `4` in the attributes.
+    const SIXEL: &[u8] = b"\x1b[?63;1;2;4;6;9;15;22c\x1b[6;20;10t\x1b[0n";
+    /// A terminal that knows none of it (Alacritty-like): attributes and status only.
+    const PLAIN: &[u8] = b"\x1b[?6c\x1b[0n";
+    /// Both kitty graphics and sixel: Kitty wins, as in ratatui-image.
+    const KITTY_AND_SIXEL: &[u8] = b"\x1b[?62;4c\x1b_Gi=31;OK\x1b\\\x1b[6;20;10t\x1b[0n";
+
+    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
+        let map: HashMap<String, String> = pairs
+            .iter()
+            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
+            .collect();
+        move |key| map.get(key).cloned()
+    }
+
+    /// The answers in `bytes` as the query reader collects them; bytes that end before
+    /// the status report are a terminal that never finished answering.
+    fn answers(bytes: &[u8]) -> Result<Vec<Response>, QueryError> {
+        let mut source = VecDeque::from(bytes.to_vec());
+        read_answers(
+            |_| Ok(source.pop_front()),
+            Duration::from_millis(50),
+            || false,
+        )
+    }
+
+    fn detection(protocol: ProtocolType, (width, height): (u16, u16)) -> Detection {
+        Detection {
+            protocol,
+            cell_size: CellSize::new(width, height),
+            warning: None,
+        }
+    }
+
+    // ----- the query -----
+
+    #[test]
+    fn the_query_asks_for_kitty_sixel_the_cell_size_and_status() {
+        let query = query_text(false, env(&[]));
+        assert!(
+            query.starts_with("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"),
+            "{query:?}"
+        );
+        assert!(
+            query.contains("\x1b[c"),
+            "device attributes (sixel): {query:?}"
+        );
+        assert!(query.contains("\x1b[16t"), "cell size: {query:?}");
+        assert!(query.ends_with("\x1b[5n"), "status report last: {query:?}");
+        assert!(!query.contains("\x1b]11;?"), "no background colour query");
+        assert!(!query.contains("\x1b[6n"), "no text sizing probe");
+    }
+
+    #[test]
+    fn wezterm_and_konsole_are_not_probed_for_kitty_or_sixel() {
+        for pairs in [
+            &[("WEZTERM_EXECUTABLE", "/usr/bin/wezterm-gui")][..],
+            &[("KONSOLE_VERSION", "240802")],
+        ] {
+            let query = query_text(false, env(pairs));
+            assert!(!query.contains("_Gi="), "{pairs:?}: {query:?}");
+            assert!(!query.contains("\x1b[c"), "{pairs:?}: {query:?}");
+            assert!(query.contains("\x1b[16t"), "{pairs:?}");
+            assert!(query.ends_with("\x1b[5n"), "{pairs:?}");
+        }
+        // Empty values do not count, as in ratatui-image.
+        assert!(query_text(false, env(&[("WEZTERM_EXECUTABLE", "")])).contains("_Gi=31"));
+    }
+
+    #[test]
+    fn inside_tmux_the_query_is_wrapped_for_passthrough() {
+        let query = query_text(true, env(&[]));
+        assert!(query.starts_with("\x1bPtmux;\x1b\x1b_Gi=31"), "{query:?}");
+        assert!(query.ends_with("\x1b\x1b[5n\x1b\\"), "{query:?}");
+    }
+
+    // ----- reading the answers -----
+
+    #[test]
+    fn the_reader_collects_the_answers_up_to_the_status_report() {
+        assert_eq!(
+            answers(KITTY).expect("complete"),
+            [Response::Kitty, Response::CellSize(Some((9, 18)))]
+        );
+        assert_eq!(
+            answers(SIXEL).expect("complete"),
+            [Response::Sixel, Response::CellSize(Some((10, 20)))]
+        );
+        assert_eq!(answers(PLAIN).expect("complete"), []);
+    }
+
+    #[test]
+    fn keys_typed_after_the_status_report_are_left_for_the_event_loop() {
+        let mut source = VecDeque::from(b"\x1b[6;20;10t\x1b[0nq\x1b[A".to_vec());
+        let responses = read_answers(|_| Ok(source.pop_front()), Duration::from_secs(5), || false)
+            .expect("complete");
+        assert_eq!(responses, [Response::CellSize(Some((10, 20)))]);
+        assert_eq!(
+            source, b"q\x1b[A",
+            "nothing after the status report is read"
+        );
+    }
+
+    #[test]
+    fn a_terminal_that_never_answers_times_out() {
+        let started = Instant::now();
+        let mut waits = Vec::new();
+        let result = read_answers(
+            |wait| {
+                waits.push(wait);
+                std::thread::sleep(wait.min(Duration::from_millis(5)));
+                Ok(None)
+            },
+            Duration::from_millis(40),
+            || false,
+        );
+        assert!(matches!(result, Err(QueryError::Timeout)), "{result:?}");
+        let took = started.elapsed();
+        assert!(took >= Duration::from_millis(40), "{took:?}");
+        assert!(took < Duration::from_secs(2), "{took:?}");
+        assert!(
+            waits.iter().all(|&wait| wait <= Duration::from_millis(40)),
+            "never waits past the deadline: {waits:?}"
+        );
+    }
+
+    #[test]
+    fn answers_cut_off_before_the_status_report_time_out() {
+        assert!(matches!(
+            answers(b"\x1b_Gi=31;OK\x1b\\\x1b[6;20;"),
+            Err(QueryError::Timeout)
+        ));
+        assert!(matches!(answers(b""), Err(QueryError::Timeout)));
+    }
+
+    #[test]
+    fn a_long_wait_is_cut_into_short_polls() {
+        // A quit signal must end the query within one poll, long before the deadline.
+        let mut waits = Vec::new();
+        let calls = Cell::new(0);
+        let result = read_answers(
+            |wait| {
+                waits.push(wait);
+                calls.set(calls.get() + 1);
+                Ok(None)
+            },
+            Duration::from_secs(10),
+            || calls.get() >= 3,
+        );
+        assert!(matches!(result, Err(QueryError::Interrupted)), "{result:?}");
+        assert_eq!(waits.len(), 3);
+        assert!(
+            waits.iter().all(|&wait| wait <= Duration::from_millis(50)),
+            "{waits:?}"
+        );
+    }
+
+    #[test]
+    fn read_errors_end_the_query() {
+        let result = read_answers(
+            |_| Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
+            Duration::from_secs(5),
+            || false,
+        );
+        let Err(QueryError::Io(error)) = result else {
+            panic!("expected an I/O error, got {result:?}");
+        };
+        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
+    }
+
+    // ----- what the answers mean -----
+
+    #[test]
+    fn kitty_and_ghostty_get_the_kitty_protocol_and_their_font_size() {
+        assert_eq!(
+            interpret(answers(KITTY), false, env(&[]), None),
+            detection(ProtocolType::Kitty, (9, 18))
+        );
+        assert_eq!(
+            interpret(
+                answers(GHOSTTY),
+                false,
+                env(&[("TERM_PROGRAM", "ghostty")]),
+                Some(CellSize::new(8, 16))
+            ),
+            detection(ProtocolType::Kitty, (17, 38)),
+            "the answer wins over the window's pixel size"
+        );
+        assert_eq!(
+            interpret(answers(KITTY_AND_SIXEL), false, env(&[]), None),
+            detection(ProtocolType::Kitty, (10, 20))
+        );
+    }
+
+    #[test]
+    fn wezterm_gets_iterm2_from_the_environment() {
+        let get = env(&[
+            ("TERM_PROGRAM", "WezTerm"),
+            ("WEZTERM_EXECUTABLE", "/usr/bin/wezterm-gui"),
+        ]);
+        assert_eq!(
+            interpret(answers(WEZTERM), false, get, None),
+            detection(ProtocolType::Iterm2, (8, 16))
+        );
+    }
+
+    #[test]
+    fn a_sixel_terminal_gets_sixel() {
+        assert_eq!(
+            interpret(answers(SIXEL), false, env(&[]), None),
+            detection(ProtocolType::Sixel, (10, 20))
+        );
+        // The answer wins over an environment hint.
+        let get = env(&[("TERM_PROGRAM", "iTerm.app")]);
+        assert_eq!(
+            interpret(answers(SIXEL), false, get, None).protocol,
+            ProtocolType::Sixel
+        );
+    }
+
+    #[test]
+    fn the_environment_names_iterm2_like_ratatui_image_does() {
+        let window = Some(CellSize::new(8, 16));
+        for pairs in [
+            &[("TERM_PROGRAM", "iTerm.app")][..],
+            &[("TERM_PROGRAM", "WezTerm")],
+            &[("TERM_PROGRAM", "vscode")],
+            &[("TERM_PROGRAM", "mintty")],
+            &[("TERM_PROGRAM", "WarpTerminal")],
+            &[("LC_TERMINAL", "iTerm2")],
+        ] {
+            assert_eq!(
+                interpret(answers(PLAIN), false, env(pairs), window),
+                detection(ProtocolType::Iterm2, (8, 16)),
+                "{pairs:?}"
+            );
+        }
+        for pairs in [
+            &[][..],
+            &[("TERM_PROGRAM", "Apple_Terminal")],
+            &[("TERM_PROGRAM", "tmux")],
+            &[("ITERM_SESSION_ID", "w0t0p0")],
+        ] {
+            assert_eq!(
+                interpret(answers(PLAIN), false, env(pairs), window),
+                detection(ProtocolType::Halfblocks, (8, 16)),
+                "{pairs:?}"
+            );
+        }
+    }
+
+    #[test]
+    fn inside_tmux_the_outer_terminal_is_guessed_from_its_variables() {
+        let window = Some(CellSize::new(8, 16));
+        for pairs in [
+            &[("ITERM_SESSION_ID", "w0t0p0")][..],
+            &[("WEZTERM_EXECUTABLE", "/usr/bin/wezterm-gui")],
+        ] {
+            assert_eq!(
+                interpret(answers(PLAIN), true, env(pairs), window).protocol,
+                ProtocolType::Iterm2,
+                "{pairs:?}"
+            );
+        }
+        assert_eq!(
+            interpret(answers(PLAIN), true, env(&[]), window).protocol,
+            ProtocolType::Halfblocks
+        );
+    }
+
+    #[test]
+    fn the_font_size_falls_back_to_the_window_then_to_ten_by_twenty() {
+        // No cell-size answer: the window's pixels per cell.
+        let kitty_without_size: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[0n";
+        assert_eq!(
+            interpret(
+                answers(kitty_without_size),
+                false,
+                env(&[]),
+                Some(CellSize::new(11, 23))
+            ),
+            detection(ProtocolType::Kitty, (11, 23))
+        );
+        // Neither: a protocol cannot be drawn at a guessed size, so half-blocks at 10×20.
+        assert_eq!(
+            interpret(answers(kitty_without_size), false, env(&[]), None),
+            detection(ProtocolType::Halfblocks, (10, 20))
+        );
+        let get = env(&[("TERM_PROGRAM", "iTerm.app")]);
+        assert_eq!(
+            interpret(answers(PLAIN), false, get, None),
+            detection(ProtocolType::Halfblocks, (10, 20))
+        );
+    }
+
+    #[test]
+    fn no_answer_falls_back_to_half_blocks_with_a_warning() {
+        let silent = interpret(
+            Err(QueryError::Timeout),
+            false,
+            env(&[("TERM_PROGRAM", "iTerm.app")]),
+            Some(CellSize::new(8, 16)),
+        );
+        assert_eq!(silent.protocol, ProtocolType::Halfblocks);
+        assert_eq!(silent.cell_size, CellSize::new(8, 16));
+        assert_eq!(
+            silent.warning.as_deref(),
+            Some("graphics query: no answer within 1 s; images use half-blocks")
+        );
+        let failed = interpret(
+            Err(QueryError::Io(io::Error::other("input/output error"))),
+            false,
+            env(&[]),
+            None,
+        );
+        assert_eq!(failed.protocol, ProtocolType::Halfblocks);
+        assert_eq!(failed.cell_size, CellSize::DEFAULT);
+        assert_eq!(
+            failed.warning.as_deref(),
+            Some("graphics query: input/output error; images use half-blocks")
+        );
+        assert_eq!(
+            interpret(answers(b""), false, env(&[]), None)
+                .warning
+                .as_deref(),
+            Some("graphics query: no answer within 1 s; images use half-blocks")
+        );
+    }
+
+    #[test]
+    fn the_window_gives_the_cell_size_when_it_knows_its_pixels() {
+        let window = |columns, rows, width, height| WindowSize {
+            rows,
+            columns,
+            width,
+            height,
+        };
+        assert_eq!(
+            cell_size_from_window(window(80, 24, 800, 480)),
+            Some(CellSize::new(10, 20))
+        );
+        assert_eq!(
+            cell_size_from_window(window(100, 30, 1712, 1140)),
+            Some(CellSize::new(17, 38)),
+            "rounded down, as ratatui-image does"
+        );
+        assert_eq!(cell_size_from_window(window(80, 24, 0, 0)), None);
+        assert_eq!(cell_size_from_window(window(0, 0, 800, 480)), None);
+        assert_eq!(
+            cell_size_from_window(window(80, 24, 40, 480)),
+            None,
+            "under a pixel"
+        );
+    }
+
+    // ----- the picker -----
+
+    #[test]
+    fn the_picker_uses_the_detected_protocol_and_font_size() {
+        for protocol in [
+            ProtocolType::Kitty,
+            ProtocolType::Iterm2,
+            ProtocolType::Sixel,
+            ProtocolType::Halfblocks,
+        ] {
+            let picker = picker_for(protocol, CellSize::new(9, 18));
+            assert_eq!(picker.protocol_type(), protocol);
+            let font = picker.font_size();
+            assert_eq!((font.width, font.height), (9, 18));
+        }
+    }
+
+    #[test]
+    fn graphics_report_what_the_glyph_choice_needs() {
+        let graphics = Graphics::from(detection(ProtocolType::Kitty, (9, 18)));
+        assert_eq!(graphics.support(), ImageSupport::Protocol);
+        assert_eq!(graphics.cell_size, CellSize::new(9, 18));
+        assert!(graphics.warning.is_none());
+        for protocol in [ProtocolType::Iterm2, ProtocolType::Sixel] {
+            assert_eq!(
+                Graphics::from(detection(protocol, (9, 18))).support(),
+                ImageSupport::Protocol
+            );
+        }
+
+        let fallback = Graphics::from(Detection {
+            warning: Some("graphics query: no answer within 1 s; images use half-blocks".into()),
+            ..detection(ProtocolType::Halfblocks, (10, 20))
+        });
+        assert_eq!(fallback.support(), ImageSupport::Halfblocks);
+        assert_eq!(
+            fallback
+                .picker
+                .as_ref()
+                .map(|picker| picker.protocol_type()),
+            Some(ProtocolType::Halfblocks)
+        );
+        assert!(fallback.warning.is_some());
+
+        let off = Graphics::off(CellSize::new(8, 16));
+        assert_eq!(off.support(), ImageSupport::Off);
+        assert!(off.picker.is_none());
+        assert_eq!(off.cell_size, CellSize::new(8, 16));
+        assert!(off.warning.is_none());
+    }
+}
diff --git a/src/tui/mod.rs b/src/tui/mod.rs
index 44902a976aa02cc8b758e78188de5c5eabef0039..285951a9fb80b32f26f390d465ea1749707d5ae0 100644
--- a/src/tui/mod.rs
+++ b/src/tui/mod.rs
@@ -11,6 +11,7 @@ pub mod board;
 pub mod event;
 pub mod files;
 pub mod glyphs;
+pub mod graphics;
 pub mod input;
 pub mod movetext;
 pub mod panels;
@@ -370,8 +371,12 @@ mod tests {
     use ratatui::backend::TestBackend;
     use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};
 
+    use ratatui_image::picker::ProtocolType;
+
     use super::app::{Hit, Mode, Screen};
-    use super::glyphs::GlyphSet;
+    use super::board::CellSize;
+    use super::glyphs::{GlyphSet, ImageSupport};
+    use super::graphics::{Graphics, picker_for};
     use super::test_support::engine::{REPLY_TIMEOUT, chars, key, mouse};
     use super::test_support::uci_moves;
     use super::*;
@@ -437,14 +442,23 @@ mod tests {
         let Cli::Play(options) = parse_args(args(&["--glyphs", "fancy"])) else {
             panic!("expected Play");
         };
-        let (set, warnings) = glyphs::initial_glyphs(options.glyphs.as_deref(), |_| None);
+        let (set, warnings) =
+            glyphs::initial_glyphs(options.glyphs.as_deref(), |_| None, ImageSupport::Off);
         assert_eq!(set, GlyphSet::Solid);
         assert_eq!(warnings.len(), 1, "{warnings:?}");
     }
 
+    #[test]
+    fn the_image_style_can_be_asked_for() {
+        assert_eq!(
+            parse_args(args(&["--glyphs", "image"])),
+            play(Some("image"), &[])
+        );
+    }
+
     #[test]
     fn a_missing_glyphs_value_is_a_warning() {
-        let missing = "--glyphs needs a value: solid, outline or ascii";
+        let missing = "--glyphs needs a value: image, solid, outline or ascii";
         assert_eq!(parse_args(args(&["--glyphs"])), play(None, &[missing]));
         assert_eq!(
             parse_args(args(&["--glyphs", "--glyphs=ascii"])),
@@ -496,12 +510,14 @@ mod tests {
             "JEV_MAX_OPTIONS",
             "JEV_FILTER_LOSING",
             "RCHESS_GLYPHS",
+            "RCHESS_IMAGES",
             "NO_COLOR",
             "COLORTERM",
         ] {
             assert!(USAGE.contains(name), "{name}");
         }
         assert!(USAGE.contains("Usage: chess "));
+        assert!(USAGE.contains("[--glyphs image|solid|outline|ascii]"));
         assert!(USAGE.lines().all(|line| line.chars().count() <= 80));
     }
 
@@ -519,6 +535,78 @@ mod tests {
         assert!(terminal_problem(false, false).is_some());
     }
 
+    // ----- start-up -----
+
+    fn options(glyphs: Option<&str>, warnings: &[&str]) -> Options {
+        let Cli::Play(options) = play(glyphs, warnings) else {
+            unreachable!("play() builds Cli::Play");
+        };
+        options
+    }
+
+    #[test]
+    fn a_graphics_protocol_starts_the_app_with_piece_images() {
+        let cell = CellSize::new(9, 18);
+        let graphics = Graphics {
+            picker: Some(picker_for(ProtocolType::Kitty, cell)),
+            cell_size: cell,
+            warning: None,
+        };
+        let app = build_app(options(None, &[]), local_engine(), graphics, |_| None);
+        assert_eq!(app.glyphs(), GlyphSet::Image);
+        assert!(app.images_available());
+        assert_eq!(
+            app.picker().map(|picker| picker.protocol_type()),
+            Some(ProtocolType::Kitty)
+        );
+        assert_eq!(app.cell_size(), cell);
+        assert!(app.warnings().is_empty(), "{:?}", app.warnings());
+    }
+
+    #[test]
+    fn a_failed_query_starts_solid_and_warns_after_the_other_notes() {
+        let warning = "graphics query: no answer within 1 s; images use half-blocks";
+        let graphics = Graphics {
+            picker: Some(picker_for(ProtocolType::Halfblocks, CellSize::DEFAULT)),
+            cell_size: CellSize::DEFAULT,
+            warning: Some(warning.to_string()),
+        };
+        let unknown = r#"ignored unknown argument "--frob" (see --help)"#;
+        let app = build_app(
+            options(Some("fancy"), &[unknown]),
+            local_engine(),
+            graphics,
+            |_| None,
+        );
+        assert_eq!(app.glyphs(), GlyphSet::Solid);
+        assert!(app.images_available(), "Image stays in the cycle");
+        assert_eq!(
+            app.warnings(),
+            [
+                unknown,
+                "--glyphs: unknown glyph set \"fancy\" (expected image, solid, outline or \
+                 ascii); using solid",
+                warning,
+            ]
+        );
+    }
+
+    #[test]
+    fn without_images_the_app_has_no_picker() {
+        let get = |name: &str| (name == "NO_COLOR").then(|| "1".to_string());
+        let cell = CellSize::new(8, 16);
+        let app = build_app(options(None, &[]), local_engine(), Graphics::off(cell), get);
+        assert_eq!(app.glyphs(), GlyphSet::Outline);
+        assert!(!app.images_available());
+        assert!(app.picker().is_none());
+        assert!(app.no_color());
+        assert_eq!(
+            app.cell_size(),
+            cell,
+            "the font size still shapes the squares"
+        );
+    }
+
     // ----- main loop -----
 
     /// One scripted `next_batch` result.
diff --git a/src/tui/panels.rs b/src/tui/panels.rs
index b968b4c6b86c1401d67097caaab6baa9d7397c53..aeaf6813345f97be8eb8283b5294e8d6b903c47b 100644
--- a/src/tui/panels.rs
+++ b/src/tui/panels.rs
@@ -1302,7 +1302,7 @@ mod tests {
     use crate::core::{START_FEN, Square};
     use crate::tui::board::{square_at, square_rect};
     use crate::tui::event::AppEvent;
-    use crate::tui::glyphs::initial_glyphs;
+    use crate::tui::glyphs::{ImageSupport, initial_glyphs};
     use crate::tui::test_support::engine::{FakeEngine, JEV_STATUS};
     use crate::tui::test_support::harness::Harness;
     use crate::tui::test_support::{PROMOTION_FEN, game_from, sq};
@@ -2276,7 +2276,7 @@ mod tests {
     fn snapshot_menu() {
         let engine =
             FakeEngine::local().with_warnings(&["JEV_TIMEOUT_MS is not a number; using 20000"]);
-        let (_, glyph_warnings) = initial_glyphs(Some("fancy"), |_| None);
+        let (_, glyph_warnings) = initial_glyphs(Some("fancy"), |_| None, ImageSupport::Off);
         let h = Harness::build(engine, (80, 24), glyph_warnings, |app| app);
         insta::assert_snapshot!("menu_80x24", h.terminal.backend());
     }
diff --git a/src/tui/snapshots/chess__tui__panels__tests__menu_80x24.snap b/src/tui/snapshots/chess__tui__panels__tests__menu_80x24.snap
index bfa7763c3c10ff79a774be1d1abd14dcd0aac84a..24650feca4758f6699c960a556e73d2cc3331373 100644
--- a/src/tui/snapshots/chess__tui__panels__tests__menu_80x24.snap
+++ b/src/tui/snapshots/chess__tui__panels__tests__menu_80x24.snap
@@ -19,8 +19,8 @@ expression: h.terminal.backend()
 "          │                                                          │          "
 "          │  No JEV_API_KEY — local search                           │          "
 "          │  ! JEV_TIMEOUT_MS is not a number; using 20000           │          "
-"          │  ! --glyphs: unknown glyph set "fancy" (expected solid,  │          "
-"          │    outline or ascii); using solid                        │          "
+"          │  ! --glyphs: unknown glyph set "fancy" (expected image,  │          "
+"          │    solid, outline or ascii); using solid                 │          "
 "          │                                                          │          "
 "          │  arrows/jk choose · Enter/1-7 start · q quit             │          "
 "          └──────────────────────────────────────────────────────────┘          "
````

- [ ] **Step: Run the tests to verify they fail**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: FAIL. The test build does not compile: the tests use names the implementation patch adds, for example "this function takes 2 arguments but 3 arguments were supplied"; "this method takes 0 arguments but 1 argument was supplied"; "cannot find struct, variant or union type Detection in this scope"; "cannot find function build_app in this scope". Any other failure (a patch that does not apply, a test that fails at run time) is not the expected RED: stop and report.

- [ ] **Step: Apply the implementation patch**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/Cargo.lock b/Cargo.lock
index 1a29c9b3580db6b49cd1f7a9764be350262854f3..153dbe0a1bd60cd01564ae99e3383d9923f78fe0 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -245,6 +245,7 @@ dependencies = [
  "proptest",
  "ratatui",
  "ratatui-image",
+ "rustix 1.1.5",
  "serde",
  "serde_json",
  "signal-hook 0.4.4",
diff --git a/Cargo.toml b/Cargo.toml
index 817157f00112a10acaef9ab910c24644ca1f284d..96b5bb7d8e712a77bd1fb0d964e3c3a3fa21e7c2 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -18,6 +18,7 @@ ratatui = "0.30"
 signal-hook = "0.4"
 ratatui-image = { version = "11.1", default-features = false, features = ["crossterm"] }
 image = { version = "0.25", default-features = false, features = ["png"] }
+rustix = { version = "1", features = ["event"] }
 
 [dev-dependencies]
 env_logger = "0.10.1"
diff --git a/src/tui/app.rs b/src/tui/app.rs
index a8c522e68b2494799d8711cc64825441f8c3a9a1..5c4c28a333aa1a5265e6cdd6c2baea166e6cab38 100644
--- a/src/tui/app.rs
+++ b/src/tui/app.rs
@@ -50,6 +50,7 @@ use ratatui::crossterm::event::{
     Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
 };
 use ratatui::layout::{Position as CellPosition, Rect};
+use ratatui_image::picker::Picker;
 
 use super::board::{BoardGeometry, CellSize, Highlights, square_at};
 use super::event::AppEvent;
@@ -600,6 +601,9 @@ pub struct App {
     /// The terminal shows no colour (`NO_COLOR`): the board marks highlights with text too.
     no_color: bool,
     glyphs: GlyphSet,
+    /// Draws piece images; `None` when images are off, which also keeps
+    /// [`GlyphSet::Image`] out of the `g` cycle.
+    picker: Option<Picker>,
     /// The terminal's font size, which shapes the board's squares.
     cell_size: CellSize,
     pick_side: fn() -> Side,
@@ -675,6 +679,7 @@ impl App {
             palette: glyphs::palette(truecolor),
             no_color: false,
             glyphs,
+            picker: None,
             cell_size: CellSize::DEFAULT,
             pick_side: random_side,
             today,
@@ -716,6 +721,15 @@ impl App {
         self
     }
 
+    /// Sets the ratatui-image picker from the graphics query (spec 9.3). With one,
+    /// `g` offers [`GlyphSet::Image`]; without one (the default: images are off) it
+    /// cycles the text sets only.
+    #[must_use]
+    pub fn with_picker(mut self, picker: Option<Picker>) -> App {
+        self.picker = picker;
+        self
+    }
+
     /// Sets the terminal's font size, which is known only once the terminal has been asked
     /// (default [`CellSize::DEFAULT`]). The next draw shapes the board's squares for it.
     pub fn set_cell_size(&mut self, cell_size: CellSize) {
@@ -871,6 +885,16 @@ impl App {
         self.glyphs
     }
 
+    /// The picker that draws piece images (see [`with_picker`](Self::with_picker)).
+    pub fn picker(&self) -> Option<&Picker> {
+        self.picker.as_ref()
+    }
+
+    /// True when piece images can be drawn, so [`GlyphSet::Image`] is in the `g` cycle.
+    pub fn images_available(&self) -> bool {
+        self.picker.is_some()
+    }
+
     /// The colours in use.
     pub fn palette(&self) -> &Palette {
         &self.palette
@@ -1735,7 +1759,7 @@ impl App {
     }
 
     fn cycle_glyphs(&mut self) {
-        self.glyphs = self.glyphs.next();
+        self.glyphs = self.glyphs.next(self.images_available());
         self.show(Message::info(format!("glyphs: {}", self.glyphs)));
     }
 
diff --git a/src/tui/glyphs.rs b/src/tui/glyphs.rs
index 9d6169c1858f6e6440cfc81c5fc4e655799f7569..0078aacc752bfc7e8bdbfbb6e69614526d059d91 100644
--- a/src/tui/glyphs.rs
+++ b/src/tui/glyphs.rs
@@ -1,5 +1,6 @@
-//! Piece glyph sets, board fillers and colour palettes (spec section 6.3), and
-//! the width rules the rest of the UI measures text with.
+//! Piece glyph sets, board fillers and colour palettes (spec sections 6.3 and
+//! 9.3), the start-up choice of a set, and the width rules the rest of the UI
+//! measures text with.
 //!
 //! Every string handed out here is exactly one terminal cell wide under
 //! ratatui's width rules, so the board grid can never shift. The tests at the
@@ -67,10 +68,19 @@ pub const ECHO_MAX_CHARS: usize = 24;
 /// Environment variable that selects the starting glyph set.
 pub const GLYPHS_ENV: &str = "RCHESS_GLYPHS";
 
+/// Environment variable that turns piece images off: `off` skips the graphics
+/// query and leaves [`GlyphSet::Image`] out of the cycle.
+pub const IMAGES_ENV: &str = "RCHESS_IMAGES";
+
 /// How pieces are drawn. `g` cycles through the sets in [`GlyphSet::next`]
 /// order.
 #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
 pub enum GlyphSet {
+    /// Pictures of the pieces, drawn with the terminal's graphics protocol (or
+    /// half-blocks without one). Offered only when the graphics query ran; where
+    /// text is drawn instead (the Captured panel, squares too small for a
+    /// picture) it uses the [`GlyphSet::Solid`] glyphs.
+    Image,
     /// Filled glyphs `♚♛♜♝♞♟` for both sides; the foreground colour carries
     /// the side. Filled shapes cover far more of the cell than outlines, so the
     /// colour stays readable on coloured squares.
@@ -88,14 +98,23 @@ pub enum GlyphSet {
 
 impl GlyphSet {
     /// All sets in cycling order.
-    pub const ALL: [GlyphSet; 3] = [GlyphSet::Solid, GlyphSet::Outline, GlyphSet::Ascii];
-
-    /// The next set in the cycle Solid → Outline → Ascii → Solid.
+    pub const ALL: [GlyphSet; 4] = [
+        GlyphSet::Image,
+        GlyphSet::Solid,
+        GlyphSet::Outline,
+        GlyphSet::Ascii,
+    ];
+
+    /// The next set in the cycle Image → Solid → Outline → Ascii → Image, or in
+    /// Solid → Outline → Ascii → Solid when `images` is false (the graphics query
+    /// was skipped, so there is nothing to draw pictures with).
     #[must_use = "next() returns the new set; it does not change `self`"]
-    pub const fn next(self) -> GlyphSet {
+    pub const fn next(self, images: bool) -> GlyphSet {
         match self {
+            GlyphSet::Image => GlyphSet::Solid,
             GlyphSet::Solid => GlyphSet::Outline,
             GlyphSet::Outline => GlyphSet::Ascii,
+            GlyphSet::Ascii if images => GlyphSet::Image,
             GlyphSet::Ascii => GlyphSet::Solid,
         }
     }
@@ -103,6 +122,7 @@ impl GlyphSet {
     /// Lower-case name as used by `--glyphs` and `RCHESS_GLYPHS`.
     pub const fn name(self) -> &'static str {
         match self {
+            GlyphSet::Image => "image",
             GlyphSet::Solid => "solid",
             GlyphSet::Outline => "outline",
             GlyphSet::Ascii => "ascii",
@@ -124,10 +144,12 @@ impl fmt::Display for GlyphSet {
     }
 }
 
-/// The one-cell string that draws `piece` in `set`. Never contains U+FE0F.
+/// The one-cell string that draws `piece` in `set` (the Solid glyph for
+/// [`GlyphSet::Image`], which draws text only where a picture does not fit).
+/// Never contains U+FE0F.
 pub const fn glyph(set: GlyphSet, piece: Piece) -> &'static str {
     match set {
-        GlyphSet::Solid => solid(piece.kind),
+        GlyphSet::Image | GlyphSet::Solid => solid(piece.kind),
         GlyphSet::Outline => match piece.color {
             Side::White => outline(piece.kind),
             Side::Black => solid(piece.kind),
@@ -262,53 +284,116 @@ pub fn detect_truecolor(get: impl Fn(&str) -> Option<String>) -> bool {
     })
 }
 
+/// What the terminal can do with pictures, as far as choosing a glyph set cares
+/// (see [`initial_glyphs`]).
+#[derive(Clone, Copy, Debug, PartialEq, Eq)]
+pub enum ImageSupport {
+    /// Images are off (`NO_COLOR` or `RCHESS_IMAGES=off`, or a text set was
+    /// named, see [`images_wanted`]): the graphics query was skipped and
+    /// [`GlyphSet::Image`] is not offered.
+    Off,
+    /// The query found no graphics protocol: [`GlyphSet::Image`] draws with
+    /// half-blocks and is offered, but not the default.
+    Halfblocks,
+    /// The query found Kitty, iTerm2 or Sixel: [`GlyphSet::Image`] is the default.
+    Protocol,
+}
+
 /// Chooses the starting glyph set and returns it with any warnings to show.
 ///
-/// Order: the `--glyphs` value (`cli`), then `RCHESS_GLYPHS`, then Outline
-/// when `NO_COLOR` is set and non-empty (the Solid set tells the sides apart
-/// only by colour), else Solid. An unknown value adds a warning and falls
-/// through to the next source; an empty `RCHESS_GLYPHS` counts as unset.
+/// Order: the `--glyphs` value (`cli`), then `RCHESS_GLYPHS`, then the default:
+/// Outline when `NO_COLOR` is set and non-empty (the Solid set tells the sides
+/// apart only by colour), else Image when `images` found a graphics protocol,
+/// else Solid. An unknown value adds a warning and falls through to the next
+/// source; so does `image` when `images` is [`ImageSupport::Off`]. An empty
+/// `RCHESS_GLYPHS` counts as unset.
 pub fn initial_glyphs(
     cli: Option<&str>,
     get: impl Fn(&str) -> Option<String>,
+    images: ImageSupport,
 ) -> (GlyphSet, Vec<String>) {
-    let mut rejected: Vec<(&str, String)> = Vec::new();
+    /// Why a named set was not used.
+    enum Refused {
+        Unknown(String),
+        ImagesOff,
+    }
+    let mut refused: Vec<(&str, Refused)> = Vec::new();
     let mut chosen = None;
 
-    if let Some(value) = cli {
-        match GlyphSet::from_name(value) {
-            Some(set) => chosen = Some(set),
-            None => rejected.push(("--glyphs", value.to_owned())),
-        }
-    }
-    if chosen.is_none()
-        && let Some(value) = get(GLYPHS_ENV).filter(|v| !v.trim().is_empty())
-    {
+    let named = [
+        ("--glyphs", cli.map(str::to_owned)),
+        (GLYPHS_ENV, get(GLYPHS_ENV).filter(|v| !v.trim().is_empty())),
+    ];
+    for (source, value) in named {
+        let Some(value) = value else { continue };
         match GlyphSet::from_name(&value) {
-            Some(set) => chosen = Some(set),
-            None => rejected.push((GLYPHS_ENV, value)),
+            Some(GlyphSet::Image) if images == ImageSupport::Off => {
+                refused.push((source, Refused::ImagesOff));
+            }
+            Some(set) => {
+                chosen = Some(set);
+                break;
+            }
+            None => refused.push((source, Refused::Unknown(value))),
         }
     }
     let set = chosen.unwrap_or_else(|| {
         if no_color(&get) {
             GlyphSet::Outline
+        } else if images == ImageSupport::Protocol {
+            GlyphSet::Image
         } else {
             GlyphSet::Solid
         }
     });
 
-    let warnings = rejected
+    // `Off` with `image` named first means one of the two switches is on.
+    let off_because = if no_color(&get) {
+        "NO_COLOR is set"
+    } else {
+        "RCHESS_IMAGES=off"
+    };
+    let warnings = refused
         .into_iter()
-        .map(|(source, value)| {
-            format!(
-                "{source}: unknown glyph set {:?} (expected solid, outline or ascii); using {set}",
+        .map(|(source, why)| match why {
+            Refused::Unknown(value) => format!(
+                "{source}: unknown glyph set {:?} (expected image, solid, outline or ascii); \
+                 using {set}",
                 shorten(&value)
-            )
+            ),
+            Refused::ImagesOff => {
+                format!("{source}: images are off ({off_because}); using {set}")
+            }
         })
         .collect();
     (set, warnings)
 }
 
+/// Whether start-up asks the terminal about graphics (spec 9.3), which also puts
+/// [`GlyphSet::Image`] in the cycle. Not when `NO_COLOR` is set, not with
+/// `RCHESS_IMAGES=off` ([`images_off`]), and not when the first valid set named by
+/// `cli` (`--glyphs`) or `RCHESS_GLYPHS` is a text set, as [`initial_glyphs`] would
+/// choose it.
+///
+/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
+pub fn images_wanted(cli: Option<&str>, get: impl Fn(&str) -> Option<String>) -> bool {
+    if no_color(&get) || images_off(&get) {
+        return false;
+    }
+    let named = cli
+        .and_then(GlyphSet::from_name)
+        .or_else(|| get(GLYPHS_ENV).and_then(|value| GlyphSet::from_name(&value)));
+    named.is_none_or(|set| set == GlyphSet::Image)
+}
+
+/// True when `RCHESS_IMAGES` is `off` (any case, surrounding whitespace ignored).
+/// Any other value leaves images on.
+///
+/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
+pub fn images_off(get: impl Fn(&str) -> Option<String>) -> bool {
+    get(IMAGES_ENV).is_some_and(|value| value.trim().eq_ignore_ascii_case("off"))
+}
+
 /// True when `NO_COLOR` is set and non-empty (<https://no-color.org/>): the terminal
 /// then shows no colour at all, because crossterm drops every colour it would write under
 /// that same test. Attributes such as reversed and underlined still show.
diff --git a/src/tui/graphics.rs b/src/tui/graphics.rs
index 6dc2c4189f978c3c7d3737f7eed1037e1d2864a9..3882b496f978471f8260f9aaaeed2a2b95b9f3ad 100644
--- a/src/tui/graphics.rs
+++ b/src/tui/graphics.rs
@@ -16,7 +16,434 @@
 //! probes (neither draws those correctly). A protocol needs a real font size; it comes
 //! from the cell-size answer, else from the window's pixel size, and without either the
 //! picker draws half-blocks at 10×20. A query that fails or times out never stops the
-//! program: it falls back to half-blocks with a warning for the menu.
+//! program: it falls back to half-blocks with a warning for the menu, and an answer
+//! that arrives after the deadline is kept out of the input ([`LateAnswers`]).
+
+use std::fmt;
+use std::io::{self, Write};
+use std::time::{Duration, Instant};
+
+use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
+use ratatui::crossterm::terminal::{WindowSize, window_size};
+use ratatui_image::FontSize;
+use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions, Response};
+use ratatui_image::picker::{Picker, ProtocolType};
+
+use super::board::CellSize;
+use super::glyphs::ImageSupport;
+
+/// How long start-up waits for the terminal's answers to the graphics query.
+pub const QUERY_TIMEOUT: Duration = Duration::from_secs(1);
+
+/// The longest single wait for input while reading the answers, so that a quit
+/// signal ends the query within this time rather than at the deadline.
+const POLL_SLICE: Duration = Duration::from_millis(50);
+
+/// How long after a query that gave up waiting its answer may still arrive and is
+/// kept out of the input ([`LateAnswers`]).
+const LATE_ANSWER_WINDOW: Duration = Duration::from_secs(10);
+
+/// The most key presses a late kitty answer is taken to have after its Alt+`_`;
+/// Kitty's answer has 8 (`Gi=31;OK`), an error answer a few dozen.
+const MAX_ANSWER_KEYS: usize = 128;
+
+/// What start-up learned about drawing pictures: the input for
+/// [`glyphs::initial_glyphs`](super::glyphs::initial_glyphs) and the App.
+#[derive(Clone, Debug)]
+pub struct Graphics {
+    /// Builds the ratatui-image protocol objects for the detected protocol and font
+    /// size; `None` when images are off (the query was skipped).
+    pub picker: Option<Picker>,
+    /// The font size in pixels, which shapes the board's squares. Known also when
+    /// images are off, if the window reports its pixel size.
+    pub cell_size: CellSize,
+    /// Why the query failed, for the menu.
+    pub warning: Option<String>,
+    /// True when the query timed out or was interrupted, so the terminal may still
+    /// answer; crossterm would read that answer as key presses ([`LateAnswers`]).
+    pub answers_pending: bool,
+}
+
+impl Graphics {
+    /// Images are off: no picker, only the font size.
+    pub const fn off(cell_size: CellSize) -> Graphics {
+        Graphics {
+            picker: None,
+            cell_size,
+            warning: None,
+            answers_pending: false,
+        }
+    }
+
+    /// What the glyph choice needs to know: whether there is a picker, and whether
+    /// it speaks a graphics protocol or draws half-blocks.
+    pub fn support(&self) -> ImageSupport {
+        match self.picker.as_ref().map(Picker::protocol_type) {
+            None => ImageSupport::Off,
+            Some(ProtocolType::Halfblocks) => ImageSupport::Halfblocks,
+            Some(_) => ImageSupport::Protocol,
+        }
+    }
+}
+
+impl From<Detection> for Graphics {
+    fn from(detection: Detection) -> Graphics {
+        Graphics {
+            picker: Some(picker_for(detection.protocol, detection.cell_size)),
+            cell_size: detection.cell_size,
+            warning: detection.warning,
+            answers_pending: detection.answers_pending,
+        }
+    }
+}
+
+/// What the answers to the query mean (see [`interpret`]).
+#[derive(Clone, Debug, PartialEq, Eq)]
+struct Detection {
+    protocol: ProtocolType,
+    cell_size: CellSize,
+    /// Set when the query failed or timed out; the protocol is then half-blocks.
+    warning: Option<String>,
+    /// See [`Graphics::answers_pending`].
+    answers_pending: bool,
+}
+
+/// Why the answers to the query could not be read.
+#[derive(Debug)]
+enum QueryError {
+    /// No status report within the deadline: a terminal that does not answer.
+    Timeout,
+    /// A quit signal arrived while waiting.
+    Interrupted,
+    /// Writing the query or reading stdin failed, or stdin ended.
+    Io(io::Error),
+}
+
+impl fmt::Display for QueryError {
+    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
+        match self {
+            QueryError::Timeout => write!(f, "no answer within {} s", QUERY_TIMEOUT.as_secs()),
+            QueryError::Interrupted => f.write_str("interrupted"),
+            QueryError::Io(error) => error.fmt(f),
+        }
+    }
+}
+
+impl From<io::Error> for QueryError {
+    fn from(error: io::Error) -> QueryError {
+        QueryError::Io(error)
+    }
+}
+
+/// Asks the terminal about graphics and builds the picker (spec 9.3). Call it
+/// once, on the UI thread, after raw mode and the alternate screen are on (so the
+/// answers are neither echoed nor shown) and before mouse capture and bracketed
+/// paste (so no mouse or paste report mixes into them). It takes at most
+/// [`QUERY_TIMEOUT`], and less once `stop` returns true (checked every 50 ms: a
+/// quit signal arrived).
+///
+/// Never fails: when the query cannot be written or answered, the picker draws
+/// half-blocks and [`Graphics::warning`] says why.
+pub fn detect(stop: impl Fn() -> bool) -> Graphics {
+    let get = |name: &str| std::env::var(name).ok();
+    // ratatui-image's own tmux check. Inside tmux it also turns passthrough on, which
+    // the kitty probe needs to reach the outer terminal (`from_query_stdio` does the
+    // same before its query).
+    let is_tmux = Picker::halfblocks().tmux_detected();
+    let answers = ask(&query_text(is_tmux, get), stop);
+    Graphics::from(interpret(answers, is_tmux, get, window_cell_size()))
+}
+
+/// [`Graphics::off`] with the font size from the window's pixel size, for when
+/// images are off and the query is skipped.
+pub fn without_images() -> Graphics {
+    Graphics::off(window_cell_size().unwrap_or_default())
+}
+
+/// A picker that draws with `protocol` for a font of `cell_size` pixels.
+pub fn picker_for(protocol: ProtocolType, cell_size: CellSize) -> Picker {
+    // The only public constructor that takes a font size. Its replacements query the
+    // terminal (`from_query_stdio`, see the module documentation) or fix the size at
+    // 10×20 (`halfblocks`).
+    #[allow(deprecated)]
+    let mut picker = Picker::from_fontsize(FontSize::new(cell_size.width(), cell_size.height()));
+    picker.set_protocol_type(protocol);
+    picker
+}
+
+/// The capability query: the kitty graphics probe, device attributes (sixel),
+/// the cell size in pixels and a status report, which every terminal answers and
+/// which therefore ends the answers. WezTerm and Konsole are not sent the kitty
+/// and sixel probes, as ratatui-image does: neither draws those correctly, and
+/// WezTerm gets iTerm2 from the environment instead.
+fn query_text(is_tmux: bool, get: impl Fn(&str) -> Option<String>) -> String {
+    let set = |name: &str| get(name).is_some_and(|value| !value.is_empty());
+    let mut options = QueryStdioOptions::default();
+    if set("WEZTERM_EXECUTABLE") || set("KONSOLE_VERSION") {
+        options.blacklist_protocols = vec![ProtocolType::Kitty, ProtocolType::Sixel];
+    }
+    Parser::query(is_tmux, options)
+}
+
+/// Writes `query` to stdout and reads the answers from stdin.
+fn ask(query: &str, stop: impl Fn() -> bool) -> Result<Vec<Response>, QueryError> {
+    let mut stdout = io::stdout().lock();
+    stdout.write_all(query.as_bytes())?;
+    stdout.flush()?;
+    drop(stdout);
+    read_answers(read_stdin_byte, QUERY_TIMEOUT, stop)
+}
+
+/// Feeds the bytes from `read_byte` to the answer parser until the status report
+/// arrives, and returns the answers before it. Nothing after the status report is
+/// read, so keys typed later stay for the event loop.
+///
+/// `read_byte(wait)` returns the next byte, or `None` when none arrived within
+/// `wait`; it is never asked to wait past the deadline `timeout` from now, nor
+/// longer than 50 ms at a time. `stop` is checked before every read.
+fn read_answers(
+    mut read_byte: impl FnMut(Duration) -> io::Result<Option<u8>>,
+    timeout: Duration,
+    stop: impl Fn() -> bool,
+) -> Result<Vec<Response>, QueryError> {
+    let deadline = Instant::now() + timeout;
+    let mut parser = Parser::new();
+    let mut responses = Vec::new();
+    loop {
+        if stop() {
+            return Err(QueryError::Interrupted);
+        }
+        let left = deadline.saturating_duration_since(Instant::now());
+        if left.is_zero() {
+            return Err(QueryError::Timeout);
+        }
+        let Some(byte) = read_byte(left.min(POLL_SLICE))? else {
+            continue;
+        };
+        // The answers are ASCII; ratatui-image feeds its parser the same way.
+        for response in parser.push(char::from(byte)) {
+            match response {
+                Response::Status => return Ok(responses),
+                other => responses.push(other),
+            }
+        }
+    }
+}
+
+/// Maps the answers to a protocol and font size as ratatui-image does
+/// (`Picker::from_query_stdio`): the protocol the answers name (Kitty over Sixel),
+/// else one the environment names ([`protocol_from_env`]), else half-blocks; the font
+/// size from the cell-size answer, else `window` (the window's pixel size per cell).
+/// Without any font size the protocol is half-blocks at 10×20, since the other
+/// protocols draw at the pixel size they are given. A failed query is half-blocks
+/// with a warning.
+fn interpret(
+    answers: Result<Vec<Response>, QueryError>,
+    is_tmux: bool,
+    get: impl Fn(&str) -> Option<String>,
+    window: Option<CellSize>,
+) -> Detection {
+    let responses = match answers {
+        Ok(responses) => responses,
+        Err(error) => {
+            return Detection {
+                protocol: ProtocolType::Halfblocks,
+                cell_size: window.unwrap_or_default(),
+                warning: Some(format!("graphics query: {error}; images use half-blocks")),
+                answers_pending: matches!(error, QueryError::Timeout | QueryError::Interrupted),
+            };
+        }
+    };
+    let mut answered = None;
+    let mut cell_size = None;
+    for response in responses {
+        match response {
+            Response::Kitty => answered = Some(ProtocolType::Kitty),
+            Response::Sixel => {
+                answered.get_or_insert(ProtocolType::Sixel);
+            }
+            Response::CellSize(Some((width, height))) => {
+                cell_size = Some(CellSize::new(width, height));
+            }
+            _ => {}
+        }
+    }
+    let (protocol, cell_size) = match cell_size.or(window) {
+        Some(cell_size) => {
+            let protocol = answered
+                .or_else(|| protocol_from_env(is_tmux, &get))
+                .unwrap_or(ProtocolType::Halfblocks);
+            (protocol, cell_size)
+        }
+        None => (ProtocolType::Halfblocks, CellSize::DEFAULT),
+    };
+    Detection {
+        protocol,
+        cell_size,
+        warning: None,
+        answers_pending: false,
+    }
+}
+
+/// Keeps a late answer to the query out of the input. A terminal that answers after
+/// [`QUERY_TIMEOUT`] (a slow SSH link) sends its answers to crossterm's reader, which
+/// turns the kitty answer `ESC _ G i=31;OK ESC \` into the key presses Alt+`_`, `G`,
+/// `i`, `=`, `3`, `1`, `;`, `O`, `K`, Alt+`\`; on the menu the `3` would start a
+/// game. The other answers never become key presses: crossterm keeps the device
+/// attributes to itself and drops the cell size and status reports.
+///
+/// After a query that gave up waiting ([`Graphics::answers_pending`]) and for the
+/// next 10 s, it drops one such run of key presses: an Alt+`_`, then printable
+/// characters starting with `G`, up to an Alt+`\`. A key that does not fit ends the
+/// run and is kept, so an Alt+`_` typed by hand costs only itself.
+#[derive(Clone, Debug)]
+pub struct LateAnswers {
+    /// Until when an answer may start; `None` once one was dropped or none is expected.
+    until: Option<Instant>,
+    /// The key presses dropped since the answer's Alt+`_`; `None` outside an answer.
+    inside: Option<usize>,
+}
+
+impl LateAnswers {
+    /// Expects a late answer when `graphics` says the query gave up waiting at `now`.
+    pub fn after(graphics: &Graphics, now: Instant) -> LateAnswers {
+        LateAnswers {
+            until: graphics.answers_pending.then(|| now + LATE_ANSWER_WINDOW),
+            inside: None,
+        }
+    }
+
+    /// Whether `event`, read at `now`, is input to keep: false for the key presses
+    /// of a late kitty answer. Other events never end or break an answer.
+    pub fn keep(&mut self, event: &Event, now: Instant) -> bool {
+        let Some(until) = self.until else {
+            return true;
+        };
+        let Event::Key(key) = event else {
+            return true;
+        };
+        match self.inside {
+            None if now > until => {
+                self.until = None;
+                true
+            }
+            None => {
+                let starts = is_alt(key, '_');
+                if starts {
+                    self.inside = Some(0);
+                }
+                !starts
+            }
+            Some(_) if is_alt(key, '\\') => {
+                // One answer only: later keys are the person's.
+                self.until = None;
+                self.inside = None;
+                false
+            }
+            Some(count) if count < MAX_ANSWER_KEYS && is_answer_char(key, count == 0) => {
+                self.inside = Some(count + 1);
+                false
+            }
+            Some(_) => {
+                self.inside = None;
+                true
+            }
+        }
+    }
+}
+
+/// Alt+`c`, as crossterm reads `ESC c`.
+fn is_alt(key: &KeyEvent, c: char) -> bool {
+    key.kind == KeyEventKind::Press
+        && key.code == KeyCode::Char(c)
+        && key.modifiers.difference(KeyModifiers::SHIFT) == KeyModifiers::ALT
+}
+
+/// A printable character as crossterm reads it from an answer; the `first` one is
+/// the `G` of a kitty graphics answer.
+fn is_answer_char(key: &KeyEvent, first: bool) -> bool {
+    let KeyCode::Char(c) = key.code else {
+        return false;
+    };
+    key.kind == KeyEventKind::Press
+        && key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
+        && (c.is_ascii_graphic() || c == ' ')
+        && (!first || c == 'G')
+}
+
+/// iTerm2 when the environment says the terminal speaks it, with ratatui-image's
+/// rules: inside tmux, when the outer terminal left `ITERM_SESSION_ID` or
+/// `WEZTERM_EXECUTABLE`; otherwise when `TERM_PROGRAM` names iTerm2, WezTerm,
+/// mintty, VS Code, Tabby, Hyper, Rio, Bobcat or Warp, or `LC_TERMINAL` names
+/// iTerm2.
+fn protocol_from_env(is_tmux: bool, get: impl Fn(&str) -> Option<String>) -> Option<ProtocolType> {
+    const TERM_PROGRAMS: [&str; 9] = [
+        "iTerm",
+        "WezTerm",
+        "mintty",
+        "vscode",
+        "Tabby",
+        "Hyper",
+        "rio",
+        "Bobcat",
+        "WarpTerminal",
+    ];
+    let set = |name: &str| get(name).is_some_and(|value| !value.is_empty());
+    let contains = |name: &str, part: &str| get(name).is_some_and(|value| value.contains(part));
+    let iterm2 = (is_tmux && (set("ITERM_SESSION_ID") || set("WEZTERM_EXECUTABLE")))
+        || TERM_PROGRAMS
+            .iter()
+            .any(|program| contains("TERM_PROGRAM", program))
+        || contains("LC_TERMINAL", "iTerm");
+    iterm2.then_some(ProtocolType::Iterm2)
+}
+
+/// The font size from the terminal's pixel and cell counts, as ratatui-image
+/// computes it (rounded down); `None` when the terminal reports no pixel size.
+fn cell_size_from_window(window: WindowSize) -> Option<CellSize> {
+    let width = window.width.checked_div(window.columns)?;
+    let height = window.height.checked_div(window.rows)?;
+    (width > 0 && height > 0).then(|| CellSize::new(width, height))
+}
+
+/// [`cell_size_from_window`] for this terminal.
+fn window_cell_size() -> Option<CellSize> {
+    window_size().ok().and_then(cell_size_from_window)
+}
+
+/// Waits up to `wait` for stdin to be readable and reads one byte, so nothing
+/// after the answers is taken from the event loop. `None` when nothing arrived
+/// (or a signal cut the wait short).
+#[cfg(unix)]
+fn read_stdin_byte(wait: Duration) -> io::Result<Option<u8>> {
+    use std::os::fd::AsFd;
+
+    use rustix::event::{PollFd, PollFlags, Timespec, poll};
+    use rustix::io::Errno;
+
+    let stdin = io::stdin();
+    let fd = stdin.as_fd();
+    let timeout = Timespec::try_from(wait).map_err(io::Error::other)?;
+    match poll(&mut [PollFd::new(&fd, PollFlags::IN)], Some(&timeout)) {
+        Ok(0) | Err(Errno::INTR) => return Ok(None),
+        Ok(_) => {}
+        Err(error) => return Err(error.into()),
+    }
+    let mut byte = [0u8];
+    match rustix::io::read(fd, &mut byte) {
+        Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
+        Ok(_) => Ok(Some(byte[0])),
+        Err(Errno::INTR | Errno::AGAIN) => Ok(None),
+        Err(error) => Err(error.into()),
+    }
+}
+
+/// Reading the answers needs `poll`, so elsewhere the query fails at once and the
+/// picker draws half-blocks.
+#[cfg(not(unix))]
+fn read_stdin_byte(_wait: Duration) -> io::Result<Option<u8>> {
+    Err(io::ErrorKind::Unsupported.into())
+}
 
 #[cfg(test)]
 mod tests {
@@ -25,6 +452,7 @@ mod tests {
     use std::io;
     use std::time::{Duration, Instant};
 
+    use ratatui::crossterm::event::{Event, KeyCode, KeyModifiers};
     use ratatui::crossterm::terminal::WindowSize;
     use ratatui_image::picker::ProtocolType;
     use ratatui_image::picker::cap_parser::Response;
@@ -32,6 +460,7 @@ mod tests {
     use super::*;
     use crate::tui::board::CellSize;
     use crate::tui::glyphs::ImageSupport;
+    use crate::tui::test_support::{chord_event, key_event, late_kitty_answer};
 
     /// Kitty 0.39: the graphics probe is accepted, no sixel, 9×18 cells.
     const KITTY: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c\x1b[6;18;9t\x1b[0n";
@@ -70,6 +499,7 @@ mod tests {
             protocol,
             cell_size: CellSize::new(width, height),
             warning: None,
+            answers_pending: false,
         }
     }
 
@@ -444,4 +874,129 @@ mod tests {
         assert_eq!(off.cell_size, CellSize::new(8, 16));
         assert!(off.warning.is_none());
     }
+
+    // ----- late answers -----
+
+    fn timed_out() -> Graphics {
+        Graphics::from(interpret(Err(QueryError::Timeout), false, env(&[]), None))
+    }
+
+    fn char_key(c: char) -> Event {
+        key_event(KeyCode::Char(c))
+    }
+
+    /// The events of `events` that `late` keeps, all seen at `now`.
+    fn kept(late: &mut LateAnswers, events: &[Event], now: Instant) -> Vec<Event> {
+        events
+            .iter()
+            .filter(|event| late.keep(event, now))
+            .cloned()
+            .collect()
+    }
+
+    #[test]
+    fn only_a_query_that_gave_up_waiting_expects_late_answers() {
+        assert!(timed_out().answers_pending);
+        let interrupted = interpret(Err(QueryError::Interrupted), false, env(&[]), None);
+        assert!(interrupted.answers_pending);
+        assert!(Graphics::from(interrupted).answers_pending);
+        let failed = interpret(
+            Err(QueryError::Io(io::Error::other("input/output error"))),
+            false,
+            env(&[]),
+            None,
+        );
+        assert!(!failed.answers_pending, "no answer comes after an error");
+        let answered = interpret(answers(KITTY), false, env(&[]), None);
+        assert!(!answered.answers_pending);
+        assert!(!Graphics::from(answered).answers_pending);
+        assert!(!Graphics::off(CellSize::DEFAULT).answers_pending);
+    }
+
+    #[test]
+    fn a_late_kitty_answer_is_not_typed_and_later_keys_are() {
+        let start = Instant::now();
+        let mut late = LateAnswers::after(&timed_out(), start);
+        let mut events = late_kitty_answer();
+        events.insert(4, Event::Resize(100, 30));
+        events.push(char_key('1'));
+        let at = start + Duration::from_millis(330);
+        assert_eq!(
+            kept(&mut late, &events, at),
+            [Event::Resize(100, 30), char_key('1')],
+            "the answer goes, a resize in the middle and the key after it stay"
+        );
+        assert_eq!(
+            kept(&mut late, &late_kitty_answer(), at),
+            late_kitty_answer(),
+            "one answer only: the same keys again are typed"
+        );
+    }
+
+    #[test]
+    fn late_answers_are_expected_only_after_a_failed_query_and_for_a_while() {
+        let start = Instant::now();
+        for graphics in [
+            Graphics::from(detection(ProtocolType::Kitty, (9, 18))),
+            Graphics::off(CellSize::DEFAULT),
+        ] {
+            let mut late = LateAnswers::after(&graphics, start);
+            assert_eq!(
+                kept(&mut late, &late_kitty_answer(), start),
+                late_kitty_answer()
+            );
+        }
+        let mut late = LateAnswers::after(&timed_out(), start);
+        let after_window = start + LATE_ANSWER_WINDOW + Duration::from_millis(1);
+        assert_eq!(
+            kept(&mut late, &late_kitty_answer(), after_window),
+            late_kitty_answer()
+        );
+    }
+
+    #[test]
+    fn keys_that_are_not_a_kitty_answer_are_kept() {
+        let start = Instant::now();
+        let alt_underscore = chord_event(KeyCode::Char('_'), KeyModifiers::ALT);
+        let mut late = LateAnswers::after(&timed_out(), start);
+        assert_eq!(
+            kept(
+                &mut late,
+                &[alt_underscore.clone(), char_key('e'), char_key('4')],
+                start
+            ),
+            [char_key('e'), char_key('4')],
+            "an answer starts with G"
+        );
+        let mut late = LateAnswers::after(&timed_out(), start);
+        let shift_g = chord_event(KeyCode::Char('G'), KeyModifiers::SHIFT);
+        assert_eq!(
+            kept(
+                &mut late,
+                &[
+                    alt_underscore,
+                    shift_g,
+                    key_event(KeyCode::Enter),
+                    char_key('x')
+                ],
+                start
+            ),
+            [key_event(KeyCode::Enter), char_key('x')],
+            "an answer holds only printable characters"
+        );
+    }
+
+    #[test]
+    fn an_answer_that_never_ends_is_cut_off() {
+        let start = Instant::now();
+        let mut late = LateAnswers::after(&timed_out(), start);
+        let mut events = vec![
+            chord_event(KeyCode::Char('_'), KeyModifiers::ALT),
+            chord_event(KeyCode::Char('G'), KeyModifiers::SHIFT),
+        ];
+        events.extend(std::iter::repeat_n(char_key('a'), 200));
+        let kept = kept(&mut late, &events, start);
+        assert_eq!(kept.len(), events.len() - 1 - MAX_ANSWER_KEYS);
+        assert!(kept.iter().all(|event| *event == char_key('a')));
+    }
 }
diff --git a/src/tui/mod.rs b/src/tui/mod.rs
index 285951a9fb80b32f26f390d465ea1749707d5ae0..8b353b22fc18abc36ddd6b7fd0050086b85c150e 100644
--- a/src/tui/mod.rs
+++ b/src/tui/mod.rs
@@ -3,8 +3,9 @@
 //! `crate::core` and `crate::engine`.
 //!
 //! [`run`] is the whole program: it reads the command line and the environment,
-//! builds the computer player, sets up the terminal and runs the main loop
-//! until the user quits or a signal asks it to stop.
+//! builds the computer player, sets up the terminal, asks it about graphics
+//! ([`graphics::detect`]) and runs the main loop until the user quits or a signal
+//! asks it to stop.
 
 pub mod app;
 pub mod board;
@@ -29,6 +30,7 @@ use std::sync::mpsc::{self, Receiver, Sender};
 use std::thread;
 use std::time::{Duration, Instant};
 
+use ratatui::backend::Backend;
 use ratatui::crossterm::event::Event;
 
 use crate::core::Game;
@@ -36,6 +38,7 @@ use crate::engine::{ComputerMove, ComputerPlayer, EngineConfig};
 
 use self::app::{Action, App};
 use self::event::AppEvent;
+use self::graphics::{Graphics, LateAnswers};
 use self::worker::{Engine, EngineOutcome, EngineReply};
 
 /// How long one batch waits for terminal input before its `Tick` (spec 6.5).
@@ -52,11 +55,12 @@ const USAGE: &str = concat!(
     "\n",
     "Usage: ",
     env!("CARGO_PKG_NAME"),
-    " [--glyphs solid|outline|ascii]\n",
+    " [--glyphs image|solid|outline|ascii]\n",
     "\n",
     "Options:\n",
-    "  --glyphs <set>     piece glyphs: solid (default), outline or ascii;\n",
-    "                     `g` cycles them during a game\n",
+    "  --glyphs <set>     how pieces look: image (pictures; the default when the\n",
+    "                     terminal can show them), solid (otherwise the default),\n",
+    "                     outline or ascii; `g` cycles them during a game\n",
     "  -h, --help         show this help and exit\n",
     "\n",
     "Environment:\n",
@@ -66,8 +70,9 @@ const USAGE: &str = concat!(
     "  JEV_MAX_OPTIONS    moves offered to Jev per turn, 1-255 (default 40)\n",
     "  JEV_FILTER_LOSING  keep losing moves off Jev's shortlist (default true)\n",
     "  RCHESS_GLYPHS      glyph set when --glyphs is not given\n",
+    "  RCHESS_IMAGES      off: no piece pictures, and no graphics query at start\n",
     "  NO_COLOR           no colours: start with outline glyphs unless a set is\n",
-    "                     chosen, and mark board highlights with text\n",
+    "                     chosen, mark board highlights with text, no pictures\n",
     "  COLORTERM          truecolor or 24bit selects 24-bit colours\n",
     "\n",
     "In the game press ? for help. Moves can be typed after / (e4, Nf3, e2e4).\n",
@@ -79,8 +84,13 @@ const USAGE: &str = concat!(
 /// `--help` prints the usage and returns without touching the terminal.
 /// Unknown arguments and a missing or invalid `--glyphs` value do not stop the
 /// program: they are listed as warnings on the menu, like invalid engine
-/// settings. The computer player comes from `EngineConfig::from_env()`; with
-/// no `JEV_API_KEY` it plays by local search and never uses the network.
+/// settings, and so is a graphics query that failed. The computer player comes
+/// from `EngineConfig::from_env()`; with no `JEV_API_KEY` it plays by local
+/// search and never uses the network.
+///
+/// Unless images are off ([`glyphs::images_wanted`]), the terminal is asked about
+/// graphics right after it is set up, which takes up to [`graphics::QUERY_TIMEOUT`]
+/// when it does not answer.
 ///
 /// Call it from the main thread (the panic hook restores the terminal only for
 /// a panic on the thread named "main"). The terminal is restored on every exit:
@@ -105,22 +115,20 @@ pub fn run(args: impl IntoIterator<Item = String>) -> io::Result<()> {
     }
 
     let env = |name: &str| std::env::var(name).ok();
-    let (glyph_set, glyph_warnings) = glyphs::initial_glyphs(options.glyphs.as_deref(), env);
-    let mut warnings = options.warnings;
-    warnings.extend(glyph_warnings);
+    let images = glyphs::images_wanted(options.glyphs.as_deref(), env);
     let fault = injected_fault(env("RCHESS_FAULT").as_deref());
     let mut engine: Arc<dyn Engine> =
         Arc::new(ComputerPlayer::from_config(EngineConfig::from_env()));
     if fault == Some(Fault::EnginePanic) {
         engine = Arc::new(PanickingEngine(engine));
     }
-    let mut app = App::new(engine, glyph_set, glyphs::detect_truecolor(env), warnings)
-        .with_no_color(glyphs::no_color(env));
 
     // Before `enter`, so a signal that arrives during setup still ends in a
     // clean restore instead of killing the process with the terminal in raw mode.
     let quit = terminal::register_signals()?;
-    let result = play(&mut app, &quit, fault);
+    let result = play(&quit, images, fault, |graphics| {
+        build_app(options, engine, graphics, env)
+    });
     let signal = if result.is_err() {
         signal_after(&quit, SIGNAL_GRACE)
     } else {
@@ -145,17 +153,58 @@ fn terminal_problem(stdin_is_terminal: bool, stdout_is_terminal: bool) -> Option
     }
 }
 
-/// Sets up the terminal, runs the main loop and restores the terminal (also on
-/// an error) before returning.
-fn play(app: &mut App, quit: &AtomicI32, fault: Option<Fault>) -> io::Result<()> {
+/// The app for the options and environment (`get`), once the terminal has been
+/// asked about graphics: the starting glyph set follows [`Graphics::support`], and
+/// the menu lists the command-line warnings, then the glyph warnings, then the
+/// graphics query's (after the engine's own, which `App::new` adds).
+fn build_app(
+    options: Options,
+    engine: Arc<dyn Engine>,
+    graphics: Graphics,
+    get: impl Fn(&str) -> Option<String>,
+) -> App {
+    let (glyph_set, glyph_warnings) =
+        glyphs::initial_glyphs(options.glyphs.as_deref(), &get, graphics.support());
+    let mut warnings = options.warnings;
+    warnings.extend(glyph_warnings);
+    warnings.extend(graphics.warning);
+    let mut app = App::new(engine, glyph_set, glyphs::detect_truecolor(&get), warnings)
+        .with_no_color(glyphs::no_color(&get))
+        .with_picker(graphics.picker);
+    app.set_cell_size(graphics.cell_size);
+    app
+}
+
+/// Sets up the terminal, asks it about graphics when `images` is true, builds the
+/// app from the result, runs the main loop and restores the terminal (also on an
+/// error) before returning.
+fn play(
+    quit: &AtomicI32,
+    images: bool,
+    fault: Option<Fault>,
+    build: impl FnOnce(Graphics) -> App,
+) -> io::Result<()> {
+    let (screen, graphics) = terminal::enter(|| {
+        if images {
+            graphics::detect(|| quit.load(Ordering::SeqCst) != 0)
+        } else {
+            graphics::without_images()
+        }
+    })?;
     // Never dropped: `Terminal`'s `Drop` shows the cursor and prints when that
     // fails, which panics on a hung-up tty. `terminal::leave` shows the cursor
     // instead, ignoring errors. The process ends soon after, so nothing leaks for
     // long.
-    let mut screen = ManuallyDrop::new(terminal::enter()?);
+    let mut screen = ManuallyDrop::new(screen);
     let _guard = terminal::Guard;
+    // A terminal that does not know a probe may have printed it, and the first draw
+    // writes only the cells that are not blank. (`Terminal::clear` would also ask
+    // for the cursor position, another answer to wait for.)
+    screen.backend_mut().clear()?;
+    let mut late = LateAnswers::after(&graphics, Instant::now());
+    let mut app = build(graphics);
     run_loop(
-        app,
+        &mut app,
         quit,
         |app| {
             screen
@@ -163,7 +212,8 @@ fn play(app: &mut App, quit: &AtomicI32, fault: Option<Fault>) -> io::Result<()>
                 .map(drop)
         },
         |replies| {
-            let batch = event::collect(replies, TICK)?;
+            let mut batch = event::collect(replies, TICK)?;
+            drop_late_answers(&mut batch, &mut late, Instant::now());
             if let Some(fault) = fault {
                 inject_ui_fault(fault, &batch);
             }
@@ -172,6 +222,15 @@ fn play(app: &mut App, quit: &AtomicI32, fault: Option<Fault>) -> io::Result<()>
     )
 }
 
+/// Removes from `batch` the key presses of a graphics answer that came after the
+/// query stopped waiting ([`LateAnswers`]), so they do not act as keys.
+fn drop_late_answers(batch: &mut Vec<AppEvent>, late: &mut LateAnswers, now: Instant) {
+    batch.retain(|event| match event {
+        AppEvent::Term(term) => late.keep(term, now),
+        _ => true,
+    });
+}
+
 /// The quit signal recorded in `quit`, waiting up to `grace` for one to arrive; 0 if
 /// none does.
 fn signal_after(quit: &AtomicI32, grace: Duration) -> i32 {
@@ -284,7 +343,7 @@ fn parse_args(args: impl IntoIterator<Item = String>) -> Cli {
                 Some(value) => options.glyphs = Some(value),
                 None => options
                     .warnings
-                    .push("--glyphs needs a value: solid, outline or ascii".to_string()),
+                    .push("--glyphs needs a value: image, solid, outline or ascii".to_string()),
             },
             _ => match arg.strip_prefix("--glyphs=") {
                 Some(value) => options.glyphs = Some(value.to_string()),
@@ -376,9 +435,9 @@ mod tests {
     use super::app::{Hit, Mode, Screen};
     use super::board::CellSize;
     use super::glyphs::{GlyphSet, ImageSupport};
-    use super::graphics::{Graphics, picker_for};
+    use super::graphics::{Graphics, LateAnswers, picker_for};
     use super::test_support::engine::{REPLY_TIMEOUT, chars, key, mouse};
-    use super::test_support::uci_moves;
+    use super::test_support::{late_kitty_answer, uci_moves};
     use super::*;
     use crate::core::Color as Side;
     use crate::engine::{JevClient, MoveSource};
@@ -551,6 +610,7 @@ mod tests {
             picker: Some(picker_for(ProtocolType::Kitty, cell)),
             cell_size: cell,
             warning: None,
+            answers_pending: false,
         };
         let app = build_app(options(None, &[]), local_engine(), graphics, |_| None);
         assert_eq!(app.glyphs(), GlyphSet::Image);
@@ -570,6 +630,7 @@ mod tests {
             picker: Some(picker_for(ProtocolType::Halfblocks, CellSize::DEFAULT)),
             cell_size: CellSize::DEFAULT,
             warning: Some(warning.to_string()),
+            answers_pending: true,
         };
         let unknown = r#"ignored unknown argument "--frob" (see --help)"#;
         let app = build_app(
@@ -684,6 +745,44 @@ mod tests {
         }
     }
 
+    #[test]
+    fn a_late_graphics_answer_does_not_start_a_game_from_the_menu() {
+        let answer: Vec<AppEvent> = late_kitty_answer()
+            .into_iter()
+            .map(AppEvent::Term)
+            .collect();
+
+        // Typed as keys, the `3` in the answer starts Human vs Jev as Black.
+        let mut app = new_app();
+        let run = drive(
+            &mut app,
+            &AtomicI32::new(0),
+            vec![Step::Events(answer.clone()), Step::Signal],
+        );
+        run.result.expect("loop ends cleanly");
+        assert_eq!(app.mode(), Mode::HumanVsJev { human: Side::Black });
+
+        let graphics = Graphics {
+            answers_pending: true,
+            ..Graphics::off(CellSize::DEFAULT)
+        };
+        let now = Instant::now();
+        let mut late = LateAnswers::after(&graphics, now);
+        let mut batch = answer;
+        drop_late_answers(&mut batch, &mut late, now);
+        let mut app = new_app();
+        let run = drive(
+            &mut app,
+            &AtomicI32::new(0),
+            vec![Step::Events(batch), Step::Events(chars("q"))],
+        );
+        run.result.expect("q on the menu quits at once");
+        assert_eq!(run.unused_steps, 0);
+        assert!(app.should_quit());
+        assert_eq!(app.screen_name(), "menu");
+        assert!(uci_moves(app.game()).is_empty());
+    }
+
     #[test]
     fn a_human_vs_human_session_plays_e4_and_quits_after_confirmation() {
         let mut app = new_app();
diff --git a/src/tui/terminal.rs b/src/tui/terminal.rs
index 95f0a4b94dadd6231cb78b90326d79166ad8d0e4..ac366f4dccd3ffc6321b472e742946c3c5eba4c6 100644
--- a/src/tui/terminal.rs
+++ b/src/tui/terminal.rs
@@ -1,7 +1,8 @@
 //! Terminal setup and teardown (spec 6.6).
 //!
-//! [`enter`] puts the terminal in raw mode on the alternate screen with
-//! click-and-drag mouse reporting and bracketed paste. [`leave`] undoes all of it
+//! [`enter`] puts the terminal in raw mode on the alternate screen, runs the
+//! caller's start-up step (the graphics query), then turns on click-and-drag
+//! mouse reporting and bracketed paste. [`leave`] undoes all of it
 //! and is reached on every exit path: a normal return or `?` error (through
 //! [`Guard`]), a panic on the UI thread (through the panic hook) and SIGINT,
 //! SIGTERM or SIGHUP (through the flag from [`register_signals`], which the main
@@ -95,9 +96,15 @@ impl Command for DisableClickMouse {
 }
 
 /// Sets up the terminal: raw mode and the alternate screen (`ratatui::try_init`),
-/// then click-and-drag mouse reporting and bracketed paste, then a thread-aware
-/// panic hook. Call it once, from the main thread, and hold a [`Guard`] for as
-/// long as the terminal is in use.
+/// then `before_input`, then click-and-drag mouse reporting and bracketed paste,
+/// then a thread-aware panic hook. Returns the terminal with what `before_input`
+/// returned. Call it once, from the main thread, and hold a [`Guard`] for as long
+/// as the terminal is in use.
+///
+/// `before_input` is for the graphics query (spec 9.3): it runs in raw mode, so
+/// the terminal's answers are not echoed, on the alternate screen, so nothing it
+/// writes stays on the main screen, and before any mouse or paste report can mix
+/// into the answers.
 ///
 /// The panic hook replaces the one `try_init` installs (ratatui's prints when its
 /// restore fails, which panics on a hung-up tty). A panic on the "main" thread
@@ -113,8 +120,9 @@ impl Command for DisableClickMouse {
 ///
 /// When there is no usable terminal (for example no controlling tty) or it
 /// rejects the setup sequences. Whatever was set up is restored first, and the
-/// panic hook is put back as it was.
-pub fn enter() -> io::Result<DefaultTerminal> {
+/// panic hook is put back as it was. `before_input` does not run when raw mode or
+/// the alternate screen could not be entered.
+pub fn enter<T>(before_input: impl FnOnce() -> T) -> io::Result<(DefaultTerminal, T)> {
     // Taken before `try_init` wraps it in ratatui's hook, which is then dropped.
     let original = panic::take_hook();
     let terminal = match ratatui::try_init() {
@@ -126,19 +134,21 @@ pub fn enter() -> io::Result<DefaultTerminal> {
         }
     };
     ACTIVE.store(true, Ordering::SeqCst);
+    let value = before_input();
     finish_enter(
         original,
         || execute!(stdout(), EnableClickMouse, EnableBracketedPaste),
         leave,
     )?;
-    Ok(terminal)
+    Ok((terminal, value))
 }
 
 /// The rest of [`enter`] once `try_init` succeeded: `enable` turns on mouse
 /// reporting and bracketed paste. If it fails, `undo` restores the terminal and
 /// `original` becomes the panic hook again (dropping ratatui's); otherwise the
 /// thread-aware hook replaces ratatui's. Until then ratatui's hook is in place,
-/// and nothing in between can panic.
+/// which restores the terminal too (only without the care for a hung-up tty);
+/// the start-up step that runs in between does not panic.
 fn finish_enter(
     original: PanicHook,
     enable: impl FnOnce() -> io::Result<()>,
diff --git a/src/tui/test_support/mod.rs b/src/tui/test_support/mod.rs
index 7f038aace54276a1063ac96f5eb01f86963a2381..b3ba580d0e279d5700715a0092e228d2a8278fac 100644
--- a/src/tui/test_support/mod.rs
+++ b/src/tui/test_support/mod.rs
@@ -68,6 +68,24 @@ pub(crate) fn chord_event(code: KeyCode, modifiers: KeyModifiers) -> Event {
     Event::Key(KeyEvent::new(code, modifiers))
 }
 
+/// The key presses crossterm makes of Kitty's answer to the graphics probe,
+/// `ESC _ G i=31;OK ESC \`, when it arrives after the query gave up waiting: Alt+`_`,
+/// the characters in between (upper case with Shift), then Alt+`\`. The other answers
+/// never become key presses (crossterm keeps them to itself or drops them).
+pub(crate) fn late_kitty_answer() -> Vec<Event> {
+    let mut keys = vec![chord_event(KeyCode::Char('_'), KeyModifiers::ALT)];
+    keys.extend("Gi=31;OK".chars().map(|c| {
+        let modifiers = if c.is_uppercase() {
+            KeyModifiers::SHIFT
+        } else {
+            KeyModifiers::NONE
+        };
+        chord_event(KeyCode::Char(c), modifiers)
+    }));
+    keys.push(chord_event(KeyCode::Char('\\'), KeyModifiers::ALT));
+    keys
+}
+
 /// A mouse event at cell (`column`, `row`), as the terminal reports it.
 pub(crate) fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> Event {
     Event::Mouse(MouseEvent {
````

- [ ] **Step: Run the tests to verify they pass**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::` and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test --lib engine::`
Expected: tui:: 353 passed, engine:: 109 passed; `find src -name '*.snap.new'` prints nothing.

Run: `cargo fmt --check` and `cargo clippy --all-targets 2>&1 | grep -A5 -E "src/tui|src/engine|src/main.rs"`
Expected: no output from either.

- [ ] **Step: Update the handoff file**

Edit `docs/handoff/HANDOFF.md`. In `## Current`, set:

```
Sub-project: tui-polish | Plan: docs/superpowers/plans/2026-09-27-tui-polish.md
Branch: feat/tui-polish
Last completed task: tui-polish task 4 (Graphics detection and the Image style)
Next task: tui-polish task 5 (board image rendering)
State: green (except 3 pre-existing old-code test failures)
```

Set `## Verify before continuing` to:

```
env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui:: && cargo test --lib engine::
```

Add at the top of `## Log (newest first)`: `- <today's date> tui-polish task 4 done: Graphics detection and the Image style`. Keep every other section. Record under `## Notes / decisions made during work` any decision this plan did not specify.

- [ ] **Step: Refresh the knowledge graph**

Run: `graphify update .`
Expected: completes without error. Never commit `graphify-out/`, `.claude/` or `.superpowers/`.

- [ ] **Step: Commit**

```bash
git add Cargo.lock Cargo.toml docs/handoff/HANDOFF.md src/tui/app.rs src/tui/glyphs.rs src/tui/graphics.rs src/tui/mod.rs src/tui/panels.rs src/tui/snapshots/chess__tui__panels__tests__menu_80x24.snap src/tui/terminal.rs src/tui/test_support/mod.rs
git commit -m "feat(tui): ask the terminal about graphics and offer the image style

Co-Authored-By: <the model you are> <noreply@anthropic.com>"
```

Replace `<the model you are>` with your own model name.

---

### Task 5: Draw pieces as pictures

**Files:**
- Create: `src/tui/snapshots/chess__tui__board__tests__image_halfblocks_knight_23x11.snap`
- Modify: `src/tui/app.rs`, `src/tui/board.rs`, `src/tui/glyphs.rs`, `src/tui/panels.rs`

**Interfaces:**
- Consumes: `pieces::{composite, ImageCache, ImageKey}`, `graphics::Graphics`, `App::picker()`, ratatui-image `Image` / `Protocol` / `Resize`.
- Produces (`src/tui/board.rs`): `MIN_IMAGE_SQUARE = (5, 2)`; `image_area(square: Rect) -> Option<Rect>` (the square minus its outer columns); `PieceImages` (the cache of encoded pictures, cleared automatically when the image area, font or protocol changes; `new`, `len`, `is_empty`); `BoardView` gains `picker: Option<&Picker>` and `overlays: &[Rect]` and implements `StatefulWidget<State = PieceImages>`. `glyphs::xterm_rgb(Color) -> Option<[u8; 3]>`.
- `panels::draw(app: &App, images: &mut PieceImages, frame: &mut Frame, now: Instant) -> Drawn`; `App::piece_images()`.

- [ ] **Step: Confirm the starting point**

Run: `git status --short` (only untracked `.claude/`, `graphify-out/` and user files) and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: 353 passed (engine: `cargo test --lib engine::` 109 passed).

- [ ] **Step: Apply the tests patch (write the failing tests)**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/tui/app.rs b/src/tui/app.rs
index 5c4c28a333aa1a5265e6cdd6c2baea166e6cab38..94a6abe08ad83df47be5c0033ecadff06544be2b 100644
--- a/src/tui/app.rs
+++ b/src/tui/app.rs
@@ -2336,8 +2336,9 @@ mod tests {
     use ratatui_image::picker::ProtocolType;
 
     use super::*;
-    use crate::core::{Position as ChessPosition, START_FEN};
+    use crate::core::{Piece, Position as ChessPosition, START_FEN};
     use crate::engine::{MoveSource, analyse};
+    use crate::tui::board::square_rect;
     use crate::tui::graphics;
     use crate::tui::panels::HELP_LINES;
     use crate::tui::test_support::engine::{FakeEngine, REPLY_TIMEOUT, chord, key, mouse, paste};
@@ -3196,6 +3197,57 @@ mod tests {
         assert_eq!(h.app.status_line(), "glyphs: image");
     }
 
+    #[test]
+    fn the_image_style_draws_pictures_and_keeps_them_between_draws() {
+        let picker = graphics::picker_for(ProtocolType::Halfblocks, CellSize::DEFAULT);
+        let mut h = Harness::build(FakeEngine::local(), (120, 40), Vec::new(), |app| {
+            app.with_picker(Some(picker))
+        });
+        h.char('1');
+        assert!(
+            h.app.piece_images().is_empty(),
+            "text styles need no pictures"
+        );
+        let e1 = |h: &Harness| -> String {
+            let g = h.app.hit_map().board.expect("the board is drawn");
+            let buf = h.terminal.backend().buffer();
+            square_rect(&g, Square::E1)
+                .positions()
+                .map(|pos| buf[pos].symbol())
+                .collect()
+        };
+        let king = glyphs::glyph(GlyphSet::Solid, Piece::new(Side::White, PieceKind::King));
+        assert!(e1(&h).contains(king));
+
+        for _ in 0..3 {
+            h.char('g');
+        }
+        assert_eq!(h.app.glyphs(), GlyphSet::Image);
+        assert!(!e1(&h).contains(king), "the king is a picture");
+        // Each side's six kinds of piece on the square colours they stand on: 10 each.
+        assert_eq!(h.app.piece_images().len(), 20);
+        // A move tints its destination, which needs a picture of the piece on that tint.
+        h.moves(&["e2e4"]);
+        assert_eq!(h.app.piece_images().len(), 21);
+        h.moves(&["e7e5"]);
+        assert_eq!(h.app.piece_images().len(), 22);
+
+        // A text style and back: nothing is built again.
+        h.char('g');
+        assert!(e1(&h).contains(king));
+        for _ in 0..3 {
+            h.char('g');
+        }
+        assert_eq!(h.app.glyphs(), GlyphSet::Image);
+        assert_eq!(h.app.piece_images().len(), 22);
+
+        // A bigger terminal gets bigger squares: only what is on the board now is built.
+        h.terminal.backend_mut().resize(200, 60);
+        h.draw();
+        assert_eq!(h.app.piece_images().len(), 21);
+        assert!(!e1(&h).contains(king));
+    }
+
     #[test]
     fn new_game_and_menu_ask_while_a_game_is_in_progress() {
         let mut h = hvh();
diff --git a/src/tui/board.rs b/src/tui/board.rs
index 986c5ea107626435a030c563b4251d2ae222d30d..381f01bc468ef94216242d1e50199c3c9456b091 100644
--- a/src/tui/board.rs
+++ b/src/tui/board.rs
@@ -439,10 +439,16 @@ fn cell_in(buf: &mut Buffer, clip: Rect, pos: CellPosition) -> Option<&mut Cell>
 
 #[cfg(test)]
 mod tests {
-    use ratatui::{Terminal, backend::TestBackend, style::Modifier, text::Span};
+    use image::DynamicImage;
+    use ratatui::{Terminal, backend::TestBackend, layout::Size, style::Modifier, text::Span};
+    use ratatui_image::picker::{Picker, ProtocolType};
+    use ratatui_image::{Image, Resize};
 
     use super::*;
-    use crate::tui::glyphs::{SOLID_PAWN, palette};
+    use crate::core::Piece;
+    use crate::tui::glyphs::{SOLID_PAWN, palette, xterm_rgb};
+    use crate::tui::graphics::picker_for;
+    use crate::tui::pieces::composite;
     use crate::tui::test_support::sq;
 
     /// Square sizes the default font gives for heights 1 to 4.
@@ -495,6 +501,7 @@ mod tests {
                         palette: &pal,
                         highlights,
                         no_color,
+                        picker: None,
                     },
                     area,
                 );
@@ -1158,6 +1165,7 @@ mod tests {
             palette: &pal,
             highlights: &highlights,
             no_color: false,
+            picker: None,
         };
         view.render(Rect::new(0, 0, 10, 5), &mut buf);
         assert_eq!(buf[(9, 1)].bg, pal.dark, "b8 is dark");
@@ -1178,4 +1186,524 @@ mod tests {
     fn geometry_at(x: u16, y: u16) -> BoardGeometry {
         geometry(3, 1, x, y, false)
     }
+
+    // ----- pictures (the Image style) -----
+
+    /// Kings on e1 (dark) and e8 (light) and White's knight on g1 (dark).
+    const KINGS_AND_KNIGHT: &str = "4k3/8/8/8/8/8/8/4K1N1 w - - 0 1";
+    /// Black's rook on a1 (dark) checks White's king on e1 (dark).
+    const ROOK_CHECK: &str = "4k3/8/8/8/8/8/8/r3K3 w - - 0 1";
+
+    const WHITE_KING: Piece = Piece::new(Side::White, PieceKind::King);
+
+    fn fen(fen: &str) -> ChessPosition {
+        ChessPosition::from_fen(fen).expect("valid FEN")
+    }
+
+    fn halfblocks(font: CellSize) -> Picker {
+        picker_for(ProtocolType::Halfblocks, font)
+    }
+
+    /// A board in the Image style, drawn by a half-blocks picker.
+    struct Scene {
+        size: (u16, u16),
+        /// The font the squares are shaped for.
+        cell: CellSize,
+        position: ChessPosition,
+        highlights: Highlights,
+        palette: Palette,
+        no_color: bool,
+        picker: Option<Picker>,
+    }
+
+    impl Scene {
+        /// `fen` on a `width`×`height` terminal with the default font and truecolor.
+        fn new(width: u16, height: u16, position: &str) -> Scene {
+            Scene {
+                size: (width, height),
+                cell: CellSize::DEFAULT,
+                position: fen(position),
+                highlights: Highlights::default(),
+                palette: palette(true),
+                no_color: false,
+                picker: Some(halfblocks(CellSize::DEFAULT)),
+            }
+        }
+
+        fn view(&self, geometry: BoardGeometry) -> BoardView<'_> {
+            BoardView {
+                position: &self.position,
+                geometry,
+                glyphs: GlyphSet::Image,
+                palette: &self.palette,
+                highlights: &self.highlights,
+                no_color: self.no_color,
+                picker: self.picker.as_ref(),
+            }
+        }
+
+        /// Draws the board into a fresh `TestBackend`, keeping the pictures in `images`.
+        fn draw(&self, images: &mut PieceImages) -> (Terminal<TestBackend>, BoardGeometry) {
+            let (width, height) = self.size;
+            let mut terminal =
+                Terminal::new(TestBackend::new(width, height)).expect("test backend");
+            let mut saved = None;
+            terminal
+                .draw(|frame| {
+                    let area = frame.area();
+                    let geometry = layout_board(area, false, self.cell).expect("board fits");
+                    saved = Some(geometry);
+                    frame.render_stateful_widget(self.view(geometry), area, images);
+                })
+                .expect("draw");
+            (terminal, saved.expect("drawn"))
+        }
+    }
+
+    /// `piece` on `background` drawn by `picker` into `area` on its own: what the board
+    /// should show there.
+    fn picture(picker: &Picker, piece: Piece, background: Color, area: Rect) -> Buffer {
+        let font = picker.font_size();
+        let image = composite(
+            piece,
+            xterm_rgb(background).expect("a palette colour"),
+            u32::from(area.width) * u32::from(font.width),
+            u32::from(area.height) * u32::from(font.height),
+        );
+        let protocol = picker
+            .new_protocol(
+                DynamicImage::ImageRgba8(image),
+                area.as_size(),
+                Resize::Fit(None),
+            )
+            .expect("half-blocks always encode");
+        assert_eq!(protocol.size(), Size::new(area.width, area.height));
+        let mut buf = Buffer::empty(area);
+        Image::new(&protocol).render(area, &mut buf);
+        buf
+    }
+
+    /// The cells of `area` in `buf`, as a buffer of their own.
+    fn cut(buf: &Buffer, area: Rect) -> Buffer {
+        let mut part = Buffer::empty(area);
+        for pos in area.positions() {
+            part[pos] = buf[pos].clone();
+        }
+        part
+    }
+
+    /// The half-block picture in `area` as text between `|`, two lines per row of
+    /// cells: ` ` for the square's colour `background`, `#` for dark pixels (Black's
+    /// pieces, White's outlines), `o` for light ones (White's fill) and `+` for the
+    /// tones between.
+    fn pixels(buf: &Buffer, area: Rect, background: Color) -> String {
+        let [r, g, b] = xterm_rgb(background).expect("a palette colour");
+        let background = Color::Rgb(r, g, b);
+        let shade = |color: Color| match color {
+            _ if color == background => ' ',
+            Color::Rgb(r, g, b) => {
+                let luma =
+                    (2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b)) / 10_000;
+                match luma {
+                    0..64 => '#',
+                    64..=192 => '+',
+                    _ => 'o',
+                }
+            }
+            other => panic!("{other:?} is not a half-block colour"),
+        };
+        let mut lines = Vec::new();
+        for row in area.rows() {
+            let (mut upper, mut lower) = (String::from("|"), String::from("|"));
+            for pos in row.positions() {
+                let cell = &buf[pos];
+                // `▄` draws the lower half in the foreground; `▀` and ` ` the upper one.
+                let (top, bottom) = if cell.symbol() == "\u{2584}" {
+                    (cell.bg, cell.fg)
+                } else {
+                    (cell.fg, cell.bg)
+                };
+                upper.push(shade(top));
+                lower.push(shade(bottom));
+            }
+            lines.push(upper + "|");
+            lines.push(lower + "|");
+        }
+        lines.join("\n")
+    }
+
+    /// True when a half-block cell shows nothing but `background`.
+    fn plain(cell: &Cell, background: Color) -> bool {
+        let [r, g, b] = xterm_rgb(background).expect("a palette colour");
+        let rgb = Color::Rgb(r, g, b);
+        cell.symbol() == " " && cell.fg == rgb && cell.bg == rgb
+    }
+
+    #[test]
+    fn the_image_area_leaves_the_outer_columns_to_text() {
+        assert_eq!(MIN_IMAGE_SQUARE, (5, 2));
+        assert_eq!(
+            image_area(Rect::new(10, 5, 5, 2)),
+            Some(Rect::new(11, 5, 3, 2))
+        );
+        assert_eq!(
+            image_area(Rect::new(0, 0, 15, 7)),
+            Some(Rect::new(1, 0, 13, 7))
+        );
+        for (width, height) in [(3, 1), (3, 2), (5, 1), (4, 2), (0, 0)] {
+            assert_eq!(
+                image_area(Rect::new(0, 0, width, height)),
+                None,
+                "{width}x{height}"
+            );
+        }
+    }
+
+    #[test]
+    fn a_half_blocks_picture_fills_the_image_area() {
+        let scene = Scene::new(185, 89, KINGS_AND_KNIGHT);
+        let picker = scene.picker.clone().expect("a picker");
+        let pal = scene.palette;
+        let knight = Piece::new(Side::White, PieceKind::Knight);
+        let (terminal, g) = scene.draw(&mut PieceImages::new());
+        assert_eq!((g.square_w, g.square_h), (23, 11));
+        let buf = terminal.backend().buffer();
+
+        // g1 is dark; its outer columns are plain text cells.
+        let g1 = square_rect(&g, sq("g1"));
+        for y in g1.top()..g1.bottom() {
+            for x in [g1.left(), g1.right() - 1] {
+                let cell = &buf[(x, y)];
+                assert_eq!((cell.symbol(), cell.bg), (" ", pal.dark), "({x}, {y})");
+            }
+        }
+        // Between them is exactly the picture the picker makes of the composite: the
+        // square's colour around the knight, the knight in the middle, no glyph.
+        let area = image_area(g1).expect("a 23x11 square has room");
+        assert_eq!(cut(buf, area), picture(&picker, knight, pal.dark, area));
+        for corner in [
+            (area.left(), area.top()),
+            (area.right() - 1, area.top()),
+            (area.left(), area.bottom() - 1),
+            (area.right() - 1, area.bottom() - 1),
+        ] {
+            assert!(
+                plain(&buf[corner], pal.dark),
+                "{corner:?}: {:?}",
+                buf[corner]
+            );
+        }
+        assert!(!plain(&buf[glyph_cell(area)], pal.dark));
+        assert!(
+            area.positions()
+                .all(|pos| ["\u{2580}", "\u{2584}", " "].contains(&buf[pos].symbol())),
+            "only half-blocks"
+        );
+        insta::assert_snapshot!("image_halfblocks_knight_23x11", pixels(buf, area, pal.dark));
+
+        // Drawn again from scratch, and as a plain widget without a cache: the same cells.
+        let (again, _) = scene.draw(&mut PieceImages::new());
+        assert_eq!(again.backend().buffer(), buf);
+        let mut uncached = Buffer::empty(*buf.area());
+        scene.view(g).render(*buf.area(), &mut uncached);
+        assert_eq!(&uncached, buf);
+    }
+
+    #[test]
+    fn pictures_are_kept_and_reused_on_squares_of_the_same_colour() {
+        let mut images = PieceImages::new();
+        // White's king on dark e1, Black's on light e8, a white pawn on light a2.
+        let mut scene = Scene::new(57, 25, "4k3/8/8/8/8/8/P7/4K3 w - - 0 1");
+        let picker = scene.picker.clone().expect("a picker");
+        scene.draw(&mut images);
+        assert_eq!(images.len(), 3);
+
+        // The king steps to d2, dark too, and the pawn is gone: nothing new is built, the
+        // king's picture is reused, and the pawn's is kept.
+        scene.position = fen("4k3/8/8/8/8/8/3K4/8 w - - 0 1");
+        let (terminal, g) = scene.draw(&mut images);
+        assert_eq!(images.len(), 3);
+        let d2 = image_area(square_rect(&g, sq("d2"))).expect("room");
+        assert_eq!(
+            cut(terminal.backend().buffer(), d2),
+            picture(&picker, WHITE_KING, scene.palette.dark, d2)
+        );
+
+        // On light e2 the king needs another picture; back home, it needs none.
+        scene.position = fen("4k3/8/8/8/8/8/4K3/8 w - - 0 1");
+        scene.draw(&mut images);
+        assert_eq!(images.len(), 4);
+        scene.position = fen("4k3/8/8/8/8/8/P7/4K3 w - - 0 1");
+        scene.draw(&mut images);
+        assert_eq!(images.len(), 4);
+    }
+
+    #[test]
+    fn each_highlight_colour_gets_its_own_picture() {
+        let mut scene = Scene::new(57, 25, ROOK_CHECK);
+        let picker = scene.picker.clone().expect("a picker");
+        let pal = scene.palette;
+        let mut images = PieceImages::new();
+        scene.draw(&mut images);
+        assert_eq!(images.len(), 3);
+
+        let steps = [
+            (
+                Highlights {
+                    check: Some(sq("e1")),
+                    ..Highlights::default()
+                },
+                "e1",
+                pal.check,
+            ),
+            (
+                Highlights {
+                    selected: Some(sq("e1")),
+                    check: Some(sq("e1")),
+                    ..Highlights::default()
+                },
+                "e1",
+                pal.selected,
+            ),
+            (
+                Highlights {
+                    targets: vec![sq("a1")],
+                    ..Highlights::default()
+                },
+                "a1",
+                pal.target,
+            ),
+            (
+                Highlights {
+                    last_move: Some((sq("a8"), sq("a1"))),
+                    ..Highlights::default()
+                },
+                "a1",
+                pal.last_move,
+            ),
+        ];
+        for (count, (highlights, name, tint)) in (4..).zip(steps) {
+            scene.highlights = highlights;
+            let (terminal, g) = scene.draw(&mut images);
+            assert_eq!(images.len(), count, "{name} on {tint:?}");
+            let area = image_area(square_rect(&g, sq(name))).expect("room");
+            let piece = scene.position.piece_at(sq(name)).expect("a piece");
+            assert_eq!(
+                cut(terminal.backend().buffer(), area),
+                picture(&picker, piece, tint, area),
+                "{name} on {tint:?}"
+            );
+        }
+
+        // Plain colours again: the first pictures are still there.
+        scene.highlights = Highlights::default();
+        scene.draw(&mut images);
+        assert_eq!(images.len(), 7);
+    }
+
+    #[test]
+    fn squares_too_small_for_a_picture_show_the_solid_glyph() {
+        let glyph = glyphs::glyph(GlyphSet::Solid, WHITE_KING);
+        // (terminal, font the squares are shaped for, square size)
+        let cases = [
+            ((31, 11), CellSize::DEFAULT, (3, 1)),
+            ((41, 9), CellSize::new(5, 20), (5, 1)),
+            ((25, 17), CellSize::new(12, 12), (3, 2)),
+        ];
+        for ((width, height), cell, size) in cases {
+            let mut scene = Scene::new(width, height, KINGS_AND_KNIGHT);
+            scene.cell = cell;
+            let mut images = PieceImages::new();
+            let (terminal, g) = scene.draw(&mut images);
+            assert_eq!((g.square_w, g.square_h), size);
+            let e1 = &terminal.backend().buffer()[glyph_cell(square_rect(&g, Square::E1))];
+            assert_eq!(
+                (e1.symbol(), e1.fg),
+                (glyph, scene.palette.white_piece),
+                "{size:?}"
+            );
+            assert!(images.is_empty(), "{size:?}: a picture was built");
+        }
+
+        // From 5×2 on, pictures.
+        let mut scene = Scene::new(41, 17, KINGS_AND_KNIGHT);
+        let mut images = PieceImages::new();
+        let (terminal, g) = scene.draw(&mut images);
+        assert_eq!((g.square_w, g.square_h), MIN_IMAGE_SQUARE);
+        assert_eq!(images.len(), 3);
+        let e1 = glyph_cell(square_rect(&g, Square::E1));
+        assert_ne!(terminal.backend().buffer()[e1].symbol(), glyph);
+
+        // Without a picker the Image style is the Solid glyphs.
+        scene.picker = None;
+        let mut images = PieceImages::new();
+        let (terminal, _) = scene.draw(&mut images);
+        assert_eq!(terminal.backend().buffer()[e1].symbol(), glyph);
+        assert!(images.is_empty());
+    }
+
+    #[test]
+    fn another_square_size_or_font_replaces_the_pictures() {
+        let mut images = PieceImages::new();
+        let mut scene = Scene::new(41, 17, KINGS_AND_KNIGHT);
+        scene.draw(&mut images);
+        assert_eq!(images.len(), 3);
+
+        // Bigger squares: the small pictures are dropped, not kept beside the new ones.
+        scene.size = (57, 25);
+        scene.draw(&mut images);
+        assert_eq!(images.len(), 3);
+
+        // Another font gives the same squares other pixel sizes.
+        let font = CellSize::new(8, 16);
+        scene.picker = Some(halfblocks(font));
+        let (terminal, g) = scene.draw(&mut images);
+        assert_eq!(images.len(), 3);
+        let area = image_area(square_rect(&g, Square::E1)).expect("room");
+        assert_eq!(
+            cut(terminal.backend().buffer(), area),
+            picture(&halfblocks(font), WHITE_KING, scene.palette.dark, area)
+        );
+    }
+
+    #[test]
+    fn the_cursor_and_the_no_colour_marks_stay_beside_a_picture() {
+        let mut scene = Scene::new(57, 25, ROOK_CHECK);
+        let picker = scene.picker.clone().expect("a picker");
+        let pal = scene.palette;
+        let rook = Piece::new(Side::Black, PieceKind::Rook);
+
+        // The cursor's brackets and tint take the outer columns; the picture is whole.
+        scene.highlights.cursor = Some(sq("e1"));
+        let (terminal, g) = scene.draw(&mut PieceImages::new());
+        let buf = terminal.backend().buffer();
+        let e1 = square_rect(&g, sq("e1"));
+        for y in e1.top()..e1.bottom() {
+            assert_eq!(buf[(e1.left(), y)].bg, pal.cursor);
+            assert_eq!(buf[(e1.right() - 1, y)].bg, pal.cursor);
+        }
+        let row = glyph_cell(e1).y;
+        assert_eq!(buf[(e1.left(), row)].symbol(), "[");
+        assert_eq!(buf[(e1.right() - 1, row)].symbol(), "]");
+        let area = image_area(e1).expect("room");
+        assert_eq!(cut(buf, area), picture(&picker, WHITE_KING, pal.dark, area));
+
+        // Without colour the marks sit in the outer columns too, and under the cursor,
+        // which holds those, the mark keeps its right side, as on a narrow square.
+        scene.no_color = true;
+        let cases = [
+            ("a1", None, "(", ")", rook, pal.target),
+            ("a1", Some("a1"), "[", ")", rook, pal.target),
+            ("e1", None, "+", "+", WHITE_KING, pal.check),
+            ("e1", Some("e1"), "[", "+", WHITE_KING, pal.check),
+        ];
+        for (name, cursor, left, right, piece, tint) in cases {
+            scene.highlights = Highlights {
+                targets: vec![sq("a1")],
+                check: Some(sq("e1")),
+                cursor: cursor.map(sq),
+                ..Highlights::default()
+            };
+            let (terminal, g) = scene.draw(&mut PieceImages::new());
+            let buf = terminal.backend().buffer();
+            let rect = square_rect(&g, sq(name));
+            let row = glyph_cell(rect).y;
+            assert_eq!(
+                (
+                    buf[(rect.left(), row)].symbol(),
+                    buf[(rect.right() - 1, row)].symbol()
+                ),
+                (left, right),
+                "{name} cursor {cursor:?}"
+            );
+            let area = image_area(rect).expect("room");
+            assert_eq!(
+                cut(buf, area),
+                picture(&picker, piece, tint, area),
+                "{name} cursor {cursor:?}"
+            );
+        }
+    }
+
+    #[test]
+    fn empty_targets_keep_their_dots_beside_pictures() {
+        let mut scene = Scene::new(57, 25, KINGS_AND_KNIGHT);
+        scene.highlights = Highlights {
+            selected: Some(sq("g1")),
+            targets: vec![sq("e2"), sq("f3"), sq("h3")],
+            ..Highlights::default()
+        };
+        let (terminal, g) = scene.draw(&mut PieceImages::new());
+        let buf = terminal.backend().buffer();
+        for name in ["e2", "f3", "h3"] {
+            let rect = square_rect(&g, sq(name));
+            let dot = glyph_cell(rect);
+            assert_eq!(
+                (buf[dot].symbol(), buf[dot].fg),
+                (".", scene.palette.black_piece),
+                "{name}"
+            );
+            assert!(
+                rect.positions()
+                    .all(|pos| pos == dot || buf[pos].symbol() == " "),
+                "{name}"
+            );
+        }
+    }
+
+    #[test]
+    fn pictures_on_the_256_colour_palette_use_the_xterm_colours() {
+        // Big squares, so the picture's corners are far enough from the king to be exact.
+        let mut scene = Scene::new(121, 57, KINGS_AND_KNIGHT);
+        scene.palette = palette(false);
+        let picker = scene.picker.clone().expect("a picker");
+        let (terminal, g) = scene.draw(&mut PieceImages::new());
+        let buf = terminal.backend().buffer();
+        let e1 = square_rect(&g, Square::E1);
+        // The text cells keep the index; the picture is composited onto xterm's #875F00.
+        assert_eq!(buf[(e1.left(), e1.top())].bg, Color::Indexed(94));
+        let area = image_area(e1).expect("room");
+        let corner = &buf[(area.left(), area.top())];
+        assert_eq!(
+            (corner.symbol(), corner.fg, corner.bg),
+            (
+                " ",
+                Color::Rgb(0x87, 0x5F, 0x00),
+                Color::Rgb(0x87, 0x5F, 0x00)
+            )
+        );
+        assert_eq!(
+            cut(buf, area),
+            picture(&picker, WHITE_KING, Color::Indexed(94), area)
+        );
+    }
+
+    #[test]
+    fn a_picture_the_area_cuts_off_is_drawn_as_the_glyph() {
+        let scene = Scene::new(57, 25, KINGS_AND_KNIGHT);
+        let full = Rect::new(0, 0, 57, 25);
+        let g = layout_board(full, false, CellSize::DEFAULT).expect("fits");
+        // Rank 1 takes rows 21 to 23; the area stops after row 22.
+        let e1 = square_rect(&g, Square::E1);
+        assert_eq!((e1.y, e1.height), (21, 3));
+        let mut buf = Buffer::empty(full);
+        let mut images = PieceImages::new();
+        ratatui::widgets::StatefulWidget::render(
+            scene.view(g),
+            Rect::new(0, 0, 57, 23),
+            &mut buf,
+            &mut images,
+        );
+        let glyph = &buf[glyph_cell(e1)];
+        assert_eq!(
+            (glyph.symbol(), glyph.fg),
+            (
+                glyphs::glyph(GlyphSet::Solid, WHITE_KING),
+                scene.palette.white_piece
+            )
+        );
+        assert_eq!(buf[(e1.x, 23)], Cell::EMPTY);
+        // Only Black's king, on rank 8, has its whole image area inside.
+        assert_eq!(images.len(), 1);
+    }
 }
diff --git a/src/tui/glyphs.rs b/src/tui/glyphs.rs
index 0078aacc752bfc7e8bdbfbb6e69614526d059d91..73748c16849d71b7c9f1cb029dc52bef7d537a98 100644
--- a/src/tui/glyphs.rs
+++ b/src/tui/glyphs.rs
@@ -700,6 +700,48 @@ mod tests {
         }
     }
 
+    #[test]
+    fn pictures_are_composited_onto_the_xterm_value_of_each_colour() {
+        // Every colour a square can have, in both palettes, has a known RGB value.
+        for p in [palette(true), palette(false)] {
+            for (name, bg) in backgrounds(&p) {
+                let (r, g, b) = rgb(bg);
+                assert_eq!(xterm_rgb(bg), Some([r, g, b]), "{name} {bg:?}");
+            }
+        }
+        // RGB passes through; indexes get xterm's defaults, as the palette's comments give them.
+        let cases = [
+            (Color::Rgb(0xB5, 0x88, 0x63), [0xB5, 0x88, 0x63]),
+            (Color::Indexed(137), [0xAF, 0x87, 0x5F]),
+            (Color::Indexed(94), [0x87, 0x5F, 0x00]),
+            (Color::Indexed(100), [0x87, 0x87, 0x00]),
+            (Color::Indexed(65), [0x5F, 0x87, 0x5F]),
+            (Color::Indexed(67), [0x5F, 0x87, 0xAF]),
+            (Color::Indexed(133), [0xAF, 0x5F, 0xAF]),
+            (Color::Indexed(160), [0xD7, 0x00, 0x00]),
+            (Color::Indexed(16), [0, 0, 0]),
+            (Color::Indexed(231), [255, 255, 255]),
+            // The grey ramp.
+            (Color::Indexed(232), [8, 8, 8]),
+            (Color::Indexed(255), [238, 238, 238]),
+            // The 16 system colours.
+            (Color::Indexed(0), [0, 0, 0]),
+            (Color::Indexed(1), [205, 0, 0]),
+            (Color::Indexed(4), [0, 0, 238]),
+            (Color::Indexed(7), [229, 229, 229]),
+            (Color::Indexed(8), [127, 127, 127]),
+            (Color::Indexed(12), [92, 92, 255]),
+            (Color::Indexed(15), [255, 255, 255]),
+        ];
+        for (color, expected) in cases {
+            assert_eq!(xterm_rgb(color), Some(expected), "{color:?}");
+        }
+        // Named colours and the default are the terminal theme's to decide.
+        for color in [Color::Reset, Color::Red, Color::White, Color::DarkGray] {
+            assert_eq!(xterm_rgb(color), None, "{color:?}");
+        }
+    }
+
     #[test]
     fn truecolor_detection() {
         assert!(detect_truecolor(env(&[("COLORTERM", "truecolor")])));
diff --git a/src/tui/snapshots/chess__tui__board__tests__image_halfblocks_knight_23x11.snap b/src/tui/snapshots/chess__tui__board__tests__image_halfblocks_knight_23x11.snap
new file mode 100644
index 0000000000000000000000000000000000000000..73dd1c2f27a27c2e6f2eb0b3fe74a9a5fc885d4b
--- /dev/null
+++ b/src/tui/snapshots/chess__tui__board__tests__image_halfblocks_knight_23x11.snap
@@ -0,0 +1,26 @@
+---
+source: src/tui/board.rs
+expression: "pixels(buf, area, pal.dark)"
+---
+|                     |
+|                     |
+|                     |
+|     ++++++          |
+|     +###+#++        |
+|     ++++o++#++      |
+|    +#+oooooo+#+     |
+|    ++++oooooo+#+    |
+|   +#++oooooooo++    |
+|   ++ooooo++oooo++   |
+|  +#oooooo++oooo++   |
+|  ++ooooo+#ooooo+#   |
+|  #++oo+++#oooooo#+  |
+|  #++++#+++oooooo++  |
+|  ++###+++ooooooo++  |
+|  +++#++#+ooooooo++  |
+|      +#+oooooooo++  |
+|      ++ooooooooo++  |
+|      #++++++++++#+  |
+|      +++++++++++++  |
+|                     |
+|                     |
````

- [ ] **Step: Run the tests to verify they fail**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: FAIL. The test build does not compile: the tests use names the implementation patch adds, for example "the trait bound tui::board::BoardView<'_>: StatefulWidget is not satisfied"; "cannot find function image_area in this scope"; "cannot find function xterm_rgb in this scope"; "cannot find type PieceImages in this scope". Any other failure (a patch that does not apply, a test that fails at run time) is not the expected RED: stop and report.

- [ ] **Step: Apply the implementation patch**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/tui/app.rs b/src/tui/app.rs
index 94a6abe08ad83df47be5c0033ecadff06544be2b..56938d3623ded36f8cbd0c9fd83a2bca3aaaf817 100644
--- a/src/tui/app.rs
+++ b/src/tui/app.rs
@@ -52,7 +52,7 @@ use ratatui::crossterm::event::{
 use ratatui::layout::{Position as CellPosition, Rect};
 use ratatui_image::picker::Picker;
 
-use super::board::{BoardGeometry, CellSize, Highlights, square_at};
+use super::board::{BoardGeometry, CellSize, Highlights, PieceImages, square_at};
 use super::event::AppEvent;
 use super::files::{SaveError, pgn_export, resolve_path, tilde_path, today, write_file};
 use super::glyphs::{self, GlyphSet, Palette};
@@ -606,6 +606,8 @@ pub struct App {
     picker: Option<Picker>,
     /// The terminal's font size, which shapes the board's squares.
     cell_size: CellSize,
+    /// The board's piece pictures, kept between draws.
+    piece_images: PieceImages,
     pick_side: fn() -> Side,
     today: fn() -> String,
     home: Option<PathBuf>,
@@ -681,6 +683,7 @@ impl App {
             glyphs,
             picker: None,
             cell_size: CellSize::DEFAULT,
+            piece_images: PieceImages::new(),
             pick_side: random_side,
             today,
             home: std::env::var_os("HOME").map(PathBuf::from),
@@ -910,6 +913,12 @@ impl App {
         self.cell_size
     }
 
+    /// The piece pictures the board has drawn in the Image style and keeps for the next
+    /// draws.
+    pub fn piece_images(&self) -> &PieceImages {
+        &self.piece_images
+    }
+
     /// The command box text.
     pub fn command_text(&self) -> &str {
         self.command.text()
@@ -2199,7 +2208,10 @@ impl App {
     /// and notes whether only [`TOO_SMALL`] fitted (input is ignored until more does).
     pub fn render(&mut self, frame: &mut Frame, now: Instant) {
         self.too_small = is_too_small(frame.area());
-        let drawn = panels::draw(self, frame, now);
+        // Lent to the draw, which reads the rest of the app and keeps new pictures in it.
+        let mut images = std::mem::take(&mut self.piece_images);
+        let drawn = panels::draw(self, &mut images, frame, now);
+        self.piece_images = images;
         self.hits = drawn.hits;
         self.move_scroll = drawn.move_scroll;
     }
diff --git a/src/tui/board.rs b/src/tui/board.rs
index 381f01bc468ef94216242d1e50199c3c9456b091..8ce50bd1d630822fa361d98bc190b2ac9d0a76a0 100644
--- a/src/tui/board.rs
+++ b/src/tui/board.rs
@@ -1,4 +1,4 @@
-//! Board widget and flip-aware hit-testing (spec sections 6.3 and 9.2).
+//! Board widget and flip-aware hit-testing (spec sections 6.3, 9.2 and 9.3).
 //!
 //! [`layout_board`] picks a square size and places the board inside an area;
 //! the resulting [`BoardGeometry`] is used both to draw ([`BoardView`]) and to
@@ -11,22 +11,41 @@
 //!
 //! The widget draws only the 8×8 grid plus a rank-label column on the left and
 //! a file-label row underneath; any enclosing block is the caller's.
+//!
+//! In the [`GlyphSet::Image`] style a piece is a picture: the Cburnett image
+//! composited onto the square's colour
+//! ([`composite`](super::pieces::composite)) and drawn by the terminal's
+//! graphics protocol through a ratatui-image [`Picker`], in the square's
+//! [`image_area`]. Squares smaller than [`MIN_IMAGE_SQUARE`] show the Solid
+//! glyph instead. [`PieceImages`] keeps the encoded pictures between frames.
+
+use std::fmt;
 
+use image::DynamicImage;
 use ratatui::{
     buffer::{Buffer, Cell},
-    layout::{Position as CellPosition, Rect},
+    layout::{Position as CellPosition, Rect, Size},
     style::{Color, Modifier},
-    widgets::Widget,
+    widgets::{self, Widget},
 };
+use ratatui_image::picker::{Picker, ProtocolType};
+use ratatui_image::protocol::Protocol;
+use ratatui_image::{Image, Resize};
 
 use super::glyphs::{self, GlyphSet, Palette};
-use crate::core::{Color as Side, PieceKind, Position as ChessPosition, Square};
+use super::pieces::{ImageCache, ImageKey};
+use crate::core::{Color as Side, Piece, PieceKind, Position as ChessPosition, Square};
 
 /// The smallest square `(width, height)` in cells: one row, with room for the
 /// glyph and the cursor's `[` `]` on either side of it. A font so narrow that
 /// even one-row squares come out too wide for the area still gets this size.
 pub const MIN_SQUARE: (u16, u16) = (3, 1);
 
+/// The smallest square `(width, height)` in cells that shows a piece as a picture
+/// in the [`GlyphSet::Image`] style; smaller squares show the Solid glyph. Its
+/// [`image_area`] is 3×2 cells.
+pub const MIN_IMAGE_SQUARE: (u16, u16) = (5, 2);
+
 /// A terminal cell's size in pixels, which is the font size. It makes squares
 /// look square: see [`square_width`].
 #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
@@ -176,6 +195,105 @@ pub const fn is_light(sq: Square) -> bool {
     (sq.file() + sq.rank()) % 2 == 1
 }
 
+/// Where a picture of the piece on `square` goes: the square without its leftmost
+/// and rightmost columns, which stay text for the cursor's `[` `]` and the marks
+/// drawn without colour. `None` for a square smaller than [`MIN_IMAGE_SQUARE`].
+pub fn image_area(square: Rect) -> Option<Rect> {
+    let (min_width, min_height) = MIN_IMAGE_SQUARE;
+    (square.width >= min_width && square.height >= min_height)
+        .then(|| Rect::new(square.x + 1, square.y, square.width - 2, square.height))
+}
+
+/// Piece pictures kept between frames for the [`GlyphSet::Image`] style: the
+/// ratatui-image [`Protocol`] a picker made of each
+/// [`composite`](super::pieces::composite), one per [`ImageKey`], so a picture is
+/// scaled and encoded once and a piece that moves to a square of the same colour
+/// reuses it. A picture the picker could not encode is kept as `None`, and its
+/// square shows the glyph.
+///
+/// Every picture on a board has the same image area size. When a frame needs
+/// another size, or the picker another font or protocol, the pictures are dropped
+/// first (the squares or the font changed), so the cache holds at most one entry
+/// per piece and square colour in use.
+#[derive(Default)]
+pub struct PieceImages {
+    cache: ImageCache<Option<Protocol>>,
+    /// What the pictures in `cache` were made for.
+    made_for: Option<PictureFormat>,
+}
+
+/// Everything a board's pictures share, besides the piece and the colour.
+#[derive(Clone, Copy, Debug, PartialEq, Eq)]
+struct PictureFormat {
+    /// The image area in cells.
+    cells: (u16, u16),
+    /// The picker's font size in pixels.
+    font: (u16, u16),
+    protocol: ProtocolType,
+}
+
+impl PieceImages {
+    /// No pictures yet.
+    pub fn new() -> PieceImages {
+        PieceImages::default()
+    }
+
+    /// Number of pictures kept.
+    pub fn len(&self) -> usize {
+        self.cache.len()
+    }
+
+    /// True when no picture is kept.
+    pub fn is_empty(&self) -> bool {
+        self.cache.is_empty()
+    }
+
+    /// The picture of `piece` on `background` (RGB) for an image area of `cells`,
+    /// drawn by `picker`: made on first use, `None` when the picker cannot encode it.
+    fn picture(
+        &mut self,
+        picker: &Picker,
+        piece: Piece,
+        background: [u8; 3],
+        cells: Size,
+    ) -> Option<&Protocol> {
+        let font = picker.font_size();
+        let format = PictureFormat {
+            cells: (cells.width, cells.height),
+            font: (font.width, font.height),
+            protocol: picker.protocol_type(),
+        };
+        if self.made_for != Some(format) {
+            self.cache.clear();
+            self.made_for = Some(format);
+        }
+        // The composite is exactly the area's size in pixels, so the picker encodes it
+        // as it is, without scaling it again.
+        let key = ImageKey::new(
+            piece,
+            background,
+            u32::from(cells.width) * u32::from(font.width),
+            u32::from(cells.height) * u32::from(font.height),
+        );
+        self.cache
+            .get_or_insert_with(key, |key| {
+                let image = DynamicImage::ImageRgba8(key.composite());
+                picker.new_protocol(image, cells, Resize::Fit(None)).ok()
+            })
+            .as_ref()
+    }
+}
+
+impl fmt::Debug for PieceImages {
+    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
+        // Protocols hold encoded image data, and are not `Debug`.
+        f.debug_struct("PieceImages")
+            .field("len", &self.len())
+            .field("made_for", &self.made_for)
+            .finish_non_exhaustive()
+    }
+}
+
 /// The cell that holds a square's glyph or target mark: the middle column
 /// and, for even heights, the upper of the two middle rows.
 fn glyph_cell(rect: Rect) -> CellPosition {
@@ -196,6 +314,9 @@ fn glyph_cell(rect: Rect) -> CellPosition {
 /// selected square is reversed, the last move's squares are underlined, a
 /// capture target gets `(` `)` and a king in check `+` `+` beside its glyph
 /// (the cursor's `[` `]` still win there).
+///
+/// A picture ([`GlyphSet::Image`]) is composited onto the square's background,
+/// highlight tints included.
 #[derive(Clone, Debug, Default, PartialEq, Eq)]
 pub struct Highlights {
     /// Origin and destination of the last move played (tinted).
@@ -214,6 +335,13 @@ pub struct Highlights {
 
 /// The board widget. Render it with any area: it draws into
 /// `geometry.outer` and clips to the area it is given.
+///
+/// In the [`GlyphSet::Image`] style with a [`picker`](Self::picker), pieces are
+/// pictures. Render it as a [`StatefulWidget`](widgets::StatefulWidget) with
+/// [`PieceImages`] to keep them between frames; as a plain [`Widget`] every
+/// picture is made afresh. A picture is drawn only when its whole image area is
+/// inside the area rendered to and clear of the [`overlays`](Self::overlays), so
+/// nothing is drawn over it.
 #[derive(Clone, Copy, Debug)]
 pub struct BoardView<'a> {
     /// Position to draw.
@@ -229,6 +357,16 @@ pub struct BoardView<'a> {
     /// The terminal shows no colour (`NO_COLOR`, see [`glyphs::no_color`]): mark
     /// the highlights with text and attributes as well as tints.
     pub no_color: bool,
+    /// Draws the pieces as pictures in the [`GlyphSet::Image`] style; without one
+    /// that style shows the Solid glyphs.
+    pub picker: Option<&'a Picker>,
+    /// Boxes drawn over the board after it (a dialog, the game-over box). A piece
+    /// whose image area meets one shows its glyph instead of its picture while the
+    /// box is up: a box would wipe the kitty picture's one-time transmission, or
+    /// leave its text on an iTerm2 or Sixel picture whose first cell it missed
+    /// (only that cell is ever sent). When the box closes, the picture is drawn
+    /// and sent whole.
+    pub overlays: &'a [Rect],
 }
 
 impl Widget for BoardView<'_> {
@@ -239,6 +377,22 @@ impl Widget for BoardView<'_> {
 
 impl Widget for &BoardView<'_> {
     fn render(self, area: Rect, buf: &mut Buffer) {
+        widgets::StatefulWidget::render(self, area, buf, &mut PieceImages::new());
+    }
+}
+
+impl widgets::StatefulWidget for BoardView<'_> {
+    type State = PieceImages;
+
+    fn render(self, area: Rect, buf: &mut Buffer, images: &mut PieceImages) {
+        widgets::StatefulWidget::render(&self, area, buf, images);
+    }
+}
+
+impl widgets::StatefulWidget for &BoardView<'_> {
+    type State = PieceImages;
+
+    fn render(self, area: Rect, buf: &mut Buffer, images: &mut PieceImages) {
         let clip = area.intersection(*buf.area());
         if clip.is_empty() {
             return;
@@ -252,14 +406,14 @@ impl Widget for &BoardView<'_> {
             }
         }
         for sq in Square::all() {
-            self.render_square(sq, clip, buf);
+            self.render_square(sq, clip, buf, images);
         }
         self.render_labels(clip, buf);
     }
 }
 
 impl BoardView<'_> {
-    fn render_square(&self, sq: Square, clip: Rect, buf: &mut Buffer) {
+    fn render_square(&self, sq: Square, clip: Rect, buf: &mut Buffer, images: &mut PieceImages) {
         let rect = square_rect(&self.geometry, sq);
         let piece = self.position.piece_at(sq);
         let is_target = self.highlights.targets.contains(&sq);
@@ -273,7 +427,11 @@ impl BoardView<'_> {
                 cell.modifier = modifier;
             }
         }
-        if let Some(cell) = cell_in(buf, clip, glyph_cell(rect)) {
+        let picture = piece.and_then(|piece| self.picture(piece, rect, bg, clip, images));
+        let pictured = picture.is_some();
+        if let Some((area, protocol)) = picture {
+            Image::new(protocol).render(area, buf);
+        } else if let Some(cell) = cell_in(buf, clip, glyph_cell(rect)) {
             if let Some(piece) = piece {
                 let fg = match piece.color {
                     Side::White => self.palette.white_piece,
@@ -291,10 +449,38 @@ impl BoardView<'_> {
             self.render_cursor(rect, clip, buf);
         }
         if let Some((left, right)) = self.side_marks(sq, is_capture) {
-            self.render_side_marks(rect, (left, right), cursor, clip, buf);
+            // A picture leaves no room between the cursor's brackets.
+            let inside = rect.width >= 5 && !pictured;
+            self.render_side_marks(rect, (left, right), cursor, inside, clip, buf);
         }
     }
 
+    /// The picture of `piece` on the square at `rect`, whose background is `bg`, and
+    /// the area it goes in. Only in the Image style with a picker, on a square of at
+    /// least [`MIN_IMAGE_SQUARE`] whose image area lies wholly inside `clip` and meets
+    /// none of the [`overlays`](Self::overlays), and on a background with a known RGB
+    /// value ([`glyphs::xterm_rgb`]); `None` otherwise, and the square shows the glyph.
+    fn picture<'i>(
+        &self,
+        piece: Piece,
+        rect: Rect,
+        bg: Color,
+        clip: Rect,
+        images: &'i mut PieceImages,
+    ) -> Option<(Rect, &'i Protocol)> {
+        if self.glyphs != GlyphSet::Image {
+            return None;
+        }
+        let picker = self.picker?;
+        let area = image_area(rect).filter(|&area| {
+            clip.intersection(area) == area
+                && !self.overlays.iter().any(|overlay| overlay.intersects(area))
+        })?;
+        let background = glyphs::xterm_rgb(bg)?;
+        let protocol = images.picture(picker, piece, background, area.as_size())?;
+        Some((area, protocol))
+    }
+
     /// True when `sq` is the en passant square and the selected piece is a
     /// pawn, so moving there captures the pawn beside it.
     fn is_en_passant_target(&self, sq: Square) -> bool {
@@ -353,14 +539,16 @@ impl BoardView<'_> {
     }
 
     /// Writes the `(left, right)` marks in the square's outer columns on the glyph row. Under
-    /// the cursor, whose `[` `]` hold the outer columns, they go just inside them (`[(p)]`);
-    /// a square too narrow for both keeps the cursor's `[` and the mark's right side
-    /// (`[p)`), so the cursor never hides the mark.
+    /// the cursor, whose `[` `]` hold the outer columns, they go just inside them (`[(p)]`)
+    /// when `inside` says there is room; a square too narrow for both, or with a picture
+    /// in its middle, keeps the cursor's `[` and the mark's right side (`[p)`), so the
+    /// cursor never hides the mark.
     fn render_side_marks(
         &self,
         rect: Rect,
         (left, right): (&'static str, &'static str),
         cursor: bool,
+        inside: bool,
         clip: Rect,
         buf: &mut Buffer,
     ) {
@@ -368,7 +556,7 @@ impl BoardView<'_> {
             return;
         }
         let (first, last) = (rect.left(), rect.right() - 1);
-        let columns = match (cursor, rect.width >= 5) {
+        let columns = match (cursor, inside) {
             (false, _) => [Some((first, left)), Some((last, right))],
             (true, true) => [Some((first + 1, left)), Some((last - 1, right))],
             (true, false) => [None, Some((last, right))],
@@ -502,6 +690,7 @@ mod tests {
                         highlights,
                         no_color,
                         picker: None,
+                        overlays: &[],
                     },
                     area,
                 );
@@ -1166,6 +1355,7 @@ mod tests {
             highlights: &highlights,
             no_color: false,
             picker: None,
+            overlays: &[],
         };
         view.render(Rect::new(0, 0, 10, 5), &mut buf);
         assert_eq!(buf[(9, 1)].bg, pal.dark, "b8 is dark");
@@ -1214,6 +1404,8 @@ mod tests {
         palette: Palette,
         no_color: bool,
         picker: Option<Picker>,
+        /// Boxes drawn over the board.
+        overlays: Vec<Rect>,
     }
 
     impl Scene {
@@ -1227,6 +1419,7 @@ mod tests {
                 palette: palette(true),
                 no_color: false,
                 picker: Some(halfblocks(CellSize::DEFAULT)),
+                overlays: Vec::new(),
             }
         }
 
@@ -1239,6 +1432,7 @@ mod tests {
                 highlights: &self.highlights,
                 no_color: self.no_color,
                 picker: self.picker.as_ref(),
+                overlays: &self.overlays,
             }
         }
 
@@ -1706,4 +1900,44 @@ mod tests {
         // Only Black's king, on rank 8, has its whole image area inside.
         assert_eq!(images.len(), 1);
     }
+
+    #[test]
+    fn a_piece_under_a_box_shows_its_glyph_until_the_box_closes() {
+        let mut scene = Scene::new(57, 25, KINGS_AND_KNIGHT);
+        let picker = scene.picker.clone().expect("a picker");
+        let full = Rect::new(0, 0, 57, 25);
+        let g = layout_board(full, false, CellSize::DEFAULT).expect("fits");
+        let (e1, g1) = (square_rect(&g, Square::E1), square_rect(&g, sq("g1")));
+        // A box over one cell of g1's picture, its last one, and nothing of e1's.
+        let g1_area = image_area(g1).expect("room");
+        scene.overlays = vec![Rect::new(g1_area.right() - 1, g1_area.bottom() - 1, 4, 2)];
+        let mut images = PieceImages::new();
+        let (terminal, _) = scene.draw(&mut images);
+        let buf = terminal.backend().buffer();
+        let knight = Piece::new(Side::White, PieceKind::Knight);
+        let glyph = &buf[glyph_cell(g1)];
+        assert_eq!(
+            (glyph.symbol(), glyph.fg),
+            (
+                glyphs::glyph(GlyphSet::Solid, knight),
+                scene.palette.white_piece
+            )
+        );
+        let e1_area = image_area(e1).expect("room");
+        assert_eq!(
+            cut(buf, e1_area),
+            picture(&picker, WHITE_KING, scene.palette.dark, e1_area)
+        );
+        // No picture is made for the covered knight.
+        assert_eq!(images.len(), 2);
+
+        // The box closes: the knight is a picture again.
+        scene.overlays.clear();
+        let (terminal, _) = scene.draw(&mut images);
+        assert_eq!(
+            cut(terminal.backend().buffer(), g1_area),
+            picture(&picker, knight, scene.palette.dark, g1_area)
+        );
+        assert_eq!(images.len(), 3);
+    }
 }
diff --git a/src/tui/glyphs.rs b/src/tui/glyphs.rs
index 73748c16849d71b7c9f1cb029dc52bef7d537a98..15c7ed80c434b8a6eeae645a6c076fea7f8f02b9 100644
--- a/src/tui/glyphs.rs
+++ b/src/tui/glyphs.rs
@@ -274,6 +274,57 @@ pub const fn palette(truecolor: bool) -> Palette {
     }
 }
 
+/// The RGB value of a palette colour, which piece pictures are composited onto (spec
+/// 9.3): an RGB colour as it is, a 256-colour index as xterm shows it by default (the 16
+/// system colours, the 6×6×6 cube, the grey ramp). `None` for the named ANSI colours
+/// and [`Color::Reset`], which the terminal's theme decides; the palettes never use
+/// them.
+pub const fn xterm_rgb(color: Color) -> Option<[u8; 3]> {
+    match color {
+        Color::Rgb(r, g, b) => Some([r, g, b]),
+        Color::Indexed(index) => Some(indexed_rgb(index)),
+        _ => None,
+    }
+}
+
+/// xterm's default RGB for colour `index` of the 256-colour palette.
+const fn indexed_rgb(index: u8) -> [u8; 3] {
+    /// Colours 0 to 15.
+    const SYSTEM: [[u8; 3]; 16] = [
+        [0, 0, 0],
+        [205, 0, 0],
+        [0, 205, 0],
+        [205, 205, 0],
+        [0, 0, 238],
+        [205, 0, 205],
+        [0, 205, 205],
+        [229, 229, 229],
+        [127, 127, 127],
+        [255, 0, 0],
+        [0, 255, 0],
+        [255, 255, 0],
+        [92, 92, 255],
+        [255, 0, 255],
+        [0, 255, 255],
+        [255, 255, 255],
+    ];
+    /// One channel of the cube: 0, then 95 to 255 in steps of 40.
+    const fn level(step: u8) -> u8 {
+        if step == 0 { 0 } else { 55 + 40 * step }
+    }
+    match index {
+        0..=15 => SYSTEM[index as usize],
+        16..=231 => {
+            let cube = index - 16;
+            [level(cube / 36), level(cube / 6 % 6), level(cube % 6)]
+        }
+        232..=255 => {
+            let grey = 8 + 10 * (index - 232);
+            [grey, grey, grey]
+        }
+    }
+}
+
 /// True when `COLORTERM` is `truecolor` or `24bit` (any case).
 ///
 /// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
diff --git a/src/tui/panels.rs b/src/tui/panels.rs
index aeaf6813345f97be8eb8283b5294e8d6b903c47b..35c9d2aaf9e998a4a78fa9bee80ebde5e43729ec 100644
--- a/src/tui/panels.rs
+++ b/src/tui/panels.rs
@@ -3,7 +3,9 @@
 //! [`draw`] renders an [`App`] through its public accessors only: the menu, the playing
 //! screen's panels, the game-over overlay, the top dialog and the too-small notice. It
 //! returns the [`HitMap`] of everything clickable, which the app keeps for the next mouse
-//! event, and the move-list scroll clamped to what the list can show.
+//! event, and the move-list scroll clamped to what the list can show. The board's piece
+//! pictures (the Image style) are kept in the [`PieceImages`] the app lends it, and are
+//! not drawn where the game-over box or a dialog will cover them.
 //!
 //! Playing layout: the screen fills the terminal. The left column holds the Board block,
 //! sized for the largest board that fits beside the narrowest side column
@@ -43,7 +45,7 @@ use super::app::{
     Message, Mode, PROMOTION_CHOICES, Question, Screen, SidePick, TOO_SMALL, WAITING_FOR_ENGINE,
     is_too_small, move_rows, outcome_text,
 };
-use super::board::{BoardGeometry, BoardView, CellSize, layout_board};
+use super::board::{BoardGeometry, BoardView, CellSize, PieceImages, layout_board};
 use super::glyphs::{self, ELLIPSIS, GlyphSet, Palette, char_width};
 use super::input::LineEditor;
 use crate::core::{Color as Side, Game, Piece, PieceKind, Position as ChessPosition};
@@ -112,8 +114,9 @@ pub struct Drawn {
 }
 
 /// Draws `app` into `frame` and returns the click targets and the clamped move-list
-/// scroll. `now` is used for the thinking spinner only.
-pub fn draw(app: &App, frame: &mut Frame, now: Instant) -> Drawn {
+/// scroll. `images` keeps the board's piece pictures between frames (the Image style).
+/// `now` is used for the thinking spinner only.
+pub fn draw(app: &App, images: &mut PieceImages, frame: &mut Frame, now: Instant) -> Drawn {
     let mut drawn = Drawn {
         hits: HitMap::default(),
         move_scroll: app.move_scroll(),
@@ -123,11 +126,12 @@ pub fn draw(app: &App, frame: &mut Frame, now: Instant) -> Drawn {
         too_small(frame, area);
         return drawn;
     }
+    let overlays = overlays(app, area);
     match app.screen() {
         Screen::Menu => menu(frame, area, app, &mut drawn.hits),
-        Screen::Playing => playing(frame, area, app, now, &mut drawn),
+        Screen::Playing => playing(frame, area, app, images, &overlays, now, &mut drawn),
         Screen::GameOver => {
-            playing(frame, area, app, now, &mut drawn);
+            playing(frame, area, app, images, &overlays, now, &mut drawn);
             game_over(frame, area, app, &mut drawn.hits);
         }
     }
@@ -389,8 +393,17 @@ fn playing_layout(
     }
 }
 
-/// The board, the command box and the side panels.
-fn playing(frame: &mut Frame, area: Rect, app: &App, now: Instant, drawn: &mut Drawn) {
+/// The board, the command box and the side panels. `overlays` are the boxes drawn over
+/// them afterwards (see [`overlays`]).
+fn playing(
+    frame: &mut Frame,
+    area: Rect,
+    app: &App,
+    images: &mut PieceImages,
+    overlays: &[Rect],
+    now: Instant,
+    drawn: &mut Drawn,
+) {
     let text_width = side_text_width(area, app.cell_size());
     let jev = (app.mode() != Mode::HumanVsHuman)
         .then(|| JevText::new(app.last_computer(), app.engine_status(), text_width));
@@ -400,7 +413,7 @@ fn playing(frame: &mut Frame, area: Rect, app: &App, now: Instant, drawn: &mut D
         status_rows(app.mode(), area.height),
         jev.as_ref().map(JevText::rows),
     );
-    drawn.hits.board = board_panel(frame, layout.board, app);
+    drawn.hits.board = board_panel(frame, layout.board, app, images, overlays);
     command_panel(
         frame,
         layout.command,
@@ -429,8 +442,15 @@ fn playing(frame: &mut Frame, area: Rect, app: &App, now: Instant, drawn: &mut D
     );
 }
 
-/// The Board block with the board centred in it; returns the geometry for hit-testing.
-fn board_panel(frame: &mut Frame, area: Rect, app: &App) -> Option<BoardGeometry> {
+/// The Board block with the board centred in it, its pictures clear of `overlays`; returns
+/// the geometry for hit-testing.
+fn board_panel(
+    frame: &mut Frame,
+    area: Rect,
+    app: &App,
+    images: &mut PieceImages,
+    overlays: &[Rect],
+) -> Option<BoardGeometry> {
     let block = Block::bordered()
         .title(" Board ")
         .padding(Padding::horizontal(1));
@@ -438,7 +458,7 @@ fn board_panel(frame: &mut Frame, area: Rect, app: &App) -> Option<BoardGeometry
     frame.render_widget(block, area);
     let geometry = layout_board(inner, app.flipped(), app.cell_size())?;
     let highlights = app.highlights();
-    frame.render_widget(
+    frame.render_stateful_widget(
         BoardView {
             position: app.game().position(),
             geometry,
@@ -446,8 +466,11 @@ fn board_panel(frame: &mut Frame, area: Rect, app: &App) -> Option<BoardGeometry
             palette: app.palette(),
             highlights: &highlights,
             no_color: app.no_color(),
+            picker: app.picker(),
+            overlays,
         },
         inner,
+        images,
     );
     Some(geometry)
 }
@@ -926,12 +949,27 @@ fn piece_span(piece: Piece, glyph_set: GlyphSet, palette: &Palette) -> Span<'sta
 
 // ----- overlays -----
 
+/// Where the boxes drawn over the playing screen go in `area`: the game-over box (on its
+/// screen, once the game has an outcome) and the top dialog. The board keeps its pictures
+/// clear of them ([`BoardView::overlays`]).
+fn overlays(app: &App, area: Rect) -> Vec<Rect> {
+    let game_over = (app.screen() == Screen::GameOver && app.game().outcome().is_some())
+        .then(|| game_over_rect(area));
+    let dialog = app.dialog().map(|top| dialog_rect(area, top));
+    game_over.into_iter().chain(dialog).collect()
+}
+
+/// Where the game-over box goes in `area`.
+fn game_over_rect(area: Rect) -> Rect {
+    centered(area, 46, 7)
+}
+
 /// The game-over overlay: the result and the New game / Save PGN / Menu buttons.
 fn game_over(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
     let Some(outcome) = app.game().outcome() else {
         return;
     };
-    let inner = dialog_frame(frame, centered(area, 46, 7), "Game over");
+    let inner = dialog_frame(frame, game_over_rect(area), "Game over");
     frame.render_widget(
         Line::from(outcome_text(outcome)).bold().centered(),
         row_of(inner, 0),
@@ -961,18 +999,40 @@ fn game_over(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
     frame.render_widget(Line::from(hint).dim().centered(), row_of(inner, 4));
 }
 
+/// Where the dialog `top` goes in `area`.
+fn dialog_rect(area: Rect, top: &Dialog) -> Rect {
+    match top {
+        Dialog::Help => {
+            let height = u16::try_from(HELP_LINES.len()).map_or(u16::MAX, |n| n.saturating_add(2));
+            centered(area, 60, height)
+        }
+        Dialog::Promotion { .. } => centered(area, 58, 5),
+        Dialog::Input { .. } => centered(area, 66, 8),
+        Dialog::Confirm { question, .. } => {
+            let text_width = CONFIRM_WIDTH.saturating_sub(4);
+            let text_rows = confirm_lines(question)
+                .iter()
+                .map(|line| wrapped_height(line, text_width))
+                .fold(0u16, u16::saturating_add);
+            // Borders, text, gap, buttons.
+            centered(area, CONFIRM_WIDTH, text_rows.saturating_add(4))
+        }
+    }
+}
+
 /// Draws the top dialog and records its targets (none for help: any click closes it).
 fn dialog(frame: &mut Frame, area: Rect, app: &App, top: &Dialog, hits: &mut HitMap) {
+    let rect = dialog_rect(area, top);
     match top {
-        Dialog::Help => help(frame, area),
-        Dialog::Promotion { choice, .. } => promotion(frame, area, *choice, app, hits),
+        Dialog::Help => help(frame, rect),
+        Dialog::Promotion { choice, .. } => promotion(frame, rect, *choice, app, hits),
         Dialog::Input {
             purpose,
             editor,
             error,
         } => input(
             frame,
-            area,
+            rect,
             top.title(),
             *purpose,
             editor,
@@ -980,15 +1040,13 @@ fn dialog(frame: &mut Frame, area: Rect, app: &App, top: &Dialog, hits: &mut Hit
             hits,
         ),
         Dialog::Confirm { question, yes } => {
-            confirm(frame, area, top.title(), question, *yes, hits)
+            confirm(frame, rect, top.title(), question, *yes, hits)
         }
     }
 }
 
-/// Keys and commands.
-fn help(frame: &mut Frame, area: Rect) {
-    let height = u16::try_from(HELP_LINES.len()).map_or(u16::MAX, |n| n.saturating_add(2));
-    let rect = centered(area, 60, height);
+/// Keys and commands, in `rect`.
+fn help(frame: &mut Frame, rect: Rect) {
     frame.render_widget(Clear, rect);
     let block =
         dialog_block("Help").title_bottom(Line::from(" Esc or click closes ").right_aligned());
@@ -1008,11 +1066,11 @@ fn help(frame: &mut Frame, area: Rect) {
     frame.render_widget(Paragraph::new(lines), inner);
 }
 
-/// The promotion picker: Queen, Rook, Bishop and Knight buttons in the colour of the side
-/// to move (the one promoting), `choice` highlighted.
-fn promotion(frame: &mut Frame, area: Rect, choice: usize, app: &App, hits: &mut HitMap) {
+/// The promotion picker in `rect`: Queen, Rook, Bishop and Knight buttons in the colour
+/// of the side to move (the one promoting), `choice` highlighted.
+fn promotion(frame: &mut Frame, rect: Rect, choice: usize, app: &App, hits: &mut HitMap) {
     let side = app.game().position().side_to_move();
-    let inner = dialog_frame(frame, centered(area, 58, 5), "Promote to");
+    let inner = dialog_frame(frame, rect, "Promote to");
     let buttons = PROMOTION_CHOICES
         .iter()
         .enumerate()
@@ -1039,17 +1097,18 @@ fn promotion(frame: &mut Frame, area: Rect, choice: usize, app: &App, hits: &mut
     );
 }
 
-/// A text field dialog: prompt, field with the terminal cursor, error and buttons.
+/// A text field dialog in `rect`: prompt, field with the terminal cursor, error and
+/// buttons.
 fn input(
     frame: &mut Frame,
-    area: Rect,
+    rect: Rect,
     title: &str,
     purpose: InputPurpose,
     editor: &LineEditor,
     error: Option<&Message>,
     hits: &mut HitMap,
 ) {
-    let inner = dialog_frame(frame, centered(area, 66, 8), title);
+    let inner = dialog_frame(frame, rect, title);
     let (prompt, action) = match purpose {
         InputPurpose::LoadFen { .. } => ("Paste or type a FEN:".to_string(), "Load"),
         InputPurpose::Save(kind) => (
@@ -1083,18 +1142,13 @@ fn input(
     render_buttons(frame, last_row(inner), buttons, hits);
 }
 
-/// A yes/no question; the dialog grows with the question's text.
-fn confirm(
-    frame: &mut Frame,
-    area: Rect,
-    title: &str,
-    question: &Question,
-    yes: bool,
-    hits: &mut HitMap,
-) {
-    const WIDTH: u16 = 56;
+/// Width of the yes/no question dialog.
+const CONFIRM_WIDTH: u16 = 56;
+
+/// The text of a yes/no question, one entry per paragraph.
+fn confirm_lines(question: &Question) -> Vec<String> {
     let text = question.text();
-    let lines = match question {
+    match question {
         // The path on its own rows, so a long one never splits the question.
         Question::Overwrite { path, .. } => {
             let path = path.display().to_string();
@@ -1103,18 +1157,20 @@ fn confirm(
             vec![path, rest]
         }
         Question::Quit | Question::Resign | Question::NewGame | Question::Menu => vec![text],
-    };
-    let text_width = WIDTH.saturating_sub(4);
-    let text_rows = lines
-        .iter()
-        .map(|line| wrapped_height(line, text_width))
-        .fold(0u16, u16::saturating_add);
-    // Borders, text, gap, buttons.
-    let inner = dialog_frame(
-        frame,
-        centered(area, WIDTH, text_rows.saturating_add(4)),
-        title,
-    );
+    }
+}
+
+/// A yes/no question in `rect`, which [`dialog_rect`] makes grow with the question's text.
+fn confirm(
+    frame: &mut Frame,
+    rect: Rect,
+    title: &str,
+    question: &Question,
+    yes: bool,
+    hits: &mut HitMap,
+) {
+    let lines = confirm_lines(question);
+    let inner = dialog_frame(frame, rect, title);
     let text_area = Rect {
         height: inner.height.saturating_sub(2),
         ..inner
@@ -1290,6 +1346,7 @@ const fn piece_name(kind: PieceKind) -> &'static str {
 
 #[cfg(test)]
 mod tests {
+    use std::collections::HashSet;
     use std::time::Duration;
 
     use ratatui::Terminal;
@@ -1297,12 +1354,14 @@ mod tests {
     use ratatui::buffer::Buffer;
     use ratatui::crossterm::event::KeyCode;
     use ratatui::widgets::Widget;
+    use ratatui_image::picker::ProtocolType;
 
     use super::*;
     use crate::core::{START_FEN, Square};
-    use crate::tui::board::{square_at, square_rect};
+    use crate::tui::board::{image_area, square_at, square_rect};
     use crate::tui::event::AppEvent;
     use crate::tui::glyphs::{ImageSupport, initial_glyphs};
+    use crate::tui::graphics::picker_for;
     use crate::tui::test_support::engine::{FakeEngine, JEV_STATUS};
     use crate::tui::test_support::harness::Harness;
     use crate::tui::test_support::{PROMOTION_FEN, game_from, sq};
@@ -2418,4 +2477,142 @@ mod tests {
         let h = jev(50, 12);
         insta::assert_snapshot!("too_small_50x12", h.terminal.backend());
     }
+
+    // ----- pictures under the boxes drawn over the board -----
+
+    /// An app against Jev in a 120×40 terminal, started from the menu with `key` and
+    /// switched to the Image style, its pictures drawn by a `protocol` picker.
+    fn pictures(protocol: ProtocolType, key: char) -> Harness {
+        let picker = picker_for(protocol, CellSize::DEFAULT);
+        let mut h = Harness::build(FakeEngine::jev(), (120, 40), Vec::new(), |app| {
+            app.with_picker(Some(picker))
+        });
+        h.char(key);
+        for _ in 0..3 {
+            h.char('g');
+        }
+        assert_eq!(h.app.glyphs(), GlyphSet::Image);
+        h
+    }
+
+    /// Adds to `ids` the kitty pictures whose image data `buffer` sends to the terminal
+    /// (the `i=` of each transmit).
+    fn transmitted(buffer: &Buffer, ids: &mut HashSet<String>) {
+        for cell in buffer.content() {
+            if let Some((_, rest)) = cell.symbol().split_once("_Gq=2,i=") {
+                let id = rest.split(',').next().unwrap_or_default();
+                ids.insert(id.to_string());
+            }
+        }
+    }
+
+    /// Draws the app again and returns the whole frame, Skip cells included (the
+    /// backend keeps only what the diff sent it).
+    fn frame(h: &mut Harness) -> Buffer {
+        let now = h.now;
+        h.terminal
+            .draw(|frame| h.app.render(frame, now))
+            .expect("draw")
+            .buffer
+            .clone()
+    }
+
+    /// The image area of `square` as last drawn.
+    fn picture_area(h: &Harness, square: Square) -> Rect {
+        let geometry = h.app.hit_map().board.expect("the board is drawn");
+        image_area(square_rect(&geometry, square)).expect("room for a picture")
+    }
+
+    #[test]
+    fn a_kitty_picture_first_drawn_under_the_help_still_reaches_the_terminal() {
+        let mut h = pictures(ProtocolType::Kitty, '2');
+        let mut ids = HashSet::new();
+        transmitted(h.buffer(), &mut ids);
+        h.command("e4");
+        transmitted(h.buffer(), &mut ids);
+        h.press(KeyCode::Esc);
+        h.char('?');
+        transmitted(h.buffer(), &mut ids);
+        // Jev answers while the help is open: its pawn on the last move's tint is a new
+        // picture, under the help box.
+        h.reply("e7e5");
+        transmitted(h.buffer(), &mut ids);
+        let [help] = overlays(&h.app, h.buffer().area)[..] else {
+            panic!("the help box alone is over the board");
+        };
+        assert!(picture_area(&h, sq("e5")).intersects(help));
+        h.char('?');
+        assert_eq!(h.app.dialog_name(), None);
+        transmitted(h.buffer(), &mut ids);
+        h.draw();
+        transmitted(h.buffer(), &mut ids);
+        assert_eq!(
+            ids.len(),
+            h.app.piece_images().len(),
+            "a picture never sent"
+        );
+    }
+
+    #[test]
+    fn a_kitty_picture_first_drawn_under_the_game_over_box_still_reaches_the_terminal() {
+        let mut h = pictures(ProtocolType::Kitty, '1');
+        let mut ids = HashSet::new();
+        transmitted(h.buffer(), &mut ids);
+        for mv in ["f3", "e5", "g4", "Qh4#"] {
+            h.command(mv);
+            transmitted(h.buffer(), &mut ids);
+        }
+        assert_eq!(h.app.screen(), Screen::GameOver);
+        let [game_over] = overlays(&h.app, h.buffer().area)[..] else {
+            panic!("the game-over box alone is over the board");
+        };
+        assert!(picture_area(&h, sq("h4")).intersects(game_over));
+        h.press(KeyCode::Esc);
+        assert_eq!(h.app.screen(), Screen::Playing);
+        transmitted(h.buffer(), &mut ids);
+        assert_eq!(
+            ids.len(),
+            h.app.piece_images().len(),
+            "a picture never sent"
+        );
+    }
+
+    #[test]
+    fn closing_the_help_sends_the_pictures_it_covered_again() {
+        for protocol in [ProtocolType::Iterm2, ProtocolType::Sixel] {
+            let mut h = pictures(protocol, '1');
+            // Black's pawn on d6 has its first row above the help box and the rest in it.
+            h.moves(&["e4", "d6"]);
+            h.char('?');
+            let open = frame(&mut h);
+            let [help] = overlays(&h.app, open.area)[..] else {
+                panic!("the help box alone is over the board");
+            };
+            h.char('?');
+            let closed = frame(&mut h);
+            let sent: Vec<(u16, u16)> =
+                open.diff(&closed).iter().map(|&(x, y, _)| (x, y)).collect();
+            // Pictures the box covered only part of, leaving their first cell alone.
+            let mut partly = 0;
+            for square in Square::all() {
+                let area = picture_area(&h, square);
+                let first = area.as_position();
+                // A picture's image data is all in its first cell.
+                if closed[first].symbol().len() < 16 || !area.intersects(help) {
+                    continue;
+                }
+                assert!(
+                    sent.contains(&(first.x, first.y)),
+                    "{protocol:?}: {square:?} is not sent again"
+                );
+                if !help.contains(first) {
+                    partly += 1;
+                }
+            }
+            assert!(
+                partly > 0,
+                "{protocol:?}: the help covers no picture partly"
+            );
+        }
+    }
 }
````

- [ ] **Step: Run the tests to verify they pass**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::` and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test --lib engine::`
Expected: tui:: 369 passed, engine:: 109 passed; `find src -name '*.snap.new'` prints nothing.

Run: `cargo fmt --check` and `cargo clippy --all-targets 2>&1 | grep -A5 -E "src/tui|src/engine|src/main.rs"`
Expected: no output from either.

- [ ] **Step: Update the handoff file**

Edit `docs/handoff/HANDOFF.md`. In `## Current`, set:

```
Sub-project: tui-polish | Plan: docs/superpowers/plans/2026-09-27-tui-polish.md
Branch: feat/tui-polish
Last completed task: tui-polish task 5 (Draw pieces as pictures)
Next task: tui-polish task 6 (debug mode)
State: green (except 3 pre-existing old-code test failures)
```

Set `## Verify before continuing` to:

```
env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui:: && cargo test --lib engine::
```

Add at the top of `## Log (newest first)`: `- <today's date> tui-polish task 5 done: Draw pieces as pictures`. Keep every other section. Record under `## Notes / decisions made during work` any decision this plan did not specify.

- [ ] **Step: Refresh the knowledge graph**

Run: `graphify update .`
Expected: completes without error. Never commit `graphify-out/`, `.claude/` or `.superpowers/`.

- [ ] **Step: Commit**

```bash
git add docs/handoff/HANDOFF.md src/tui/app.rs src/tui/board.rs src/tui/glyphs.rs src/tui/panels.rs src/tui/snapshots/chess__tui__board__tests__image_halfblocks_knight_23x11.snap
git commit -m "feat(tui): draw pieces as pictures in the image style and cache them

Co-Authored-By: <the model you are> <noreply@anthropic.com>"
```

Replace `<the model you are>` with your own model name.

---

### Task 6: Debug mode: exchange history, view and log

**Files:**
- Create: `src/tui/debug.rs`, `src/tui/snapshots/chess__tui__panels__tests__exchange_120x40_stale.snap`, `src/tui/snapshots/chess__tui__panels__tests__exchange_80x24.snap`
- Modify: `src/engine/jev.rs`, `src/engine/mod.rs`, `src/tui/app.rs`, `src/tui/event.rs`, `src/tui/files.rs`, `src/tui/mod.rs`, `src/tui/panels.rs`, `src/tui/snapshots/chess__tui__panels__tests__help_80x24.snap`, `src/tui/test_support/engine.rs`, `src/tui/test_support/harness.rs`, `src/tui/worker.rs`

**Interfaces:**
- Consumes: `engine::{JevExchange, JevAttempt}`, `EngineConfig.trace`, `ComputerMove.exchange`, `worker::EngineReply`, `files` path expansion.
- Produces (`src/tui/debug.rs`): `DEBUG_ENV` (`RCHESS_DEBUG`), `DEBUG_LOG_ENV` (`RCHESS_DEBUG_LOG`), `DEBUG_OFF`, `NO_EXCHANGES`, `HISTORY_LEN` (50); `enabled(flag: bool, get) -> bool`; `log_path(get) -> ...`; `Exchange` / `Record` (one exchange with ply, SAN, source, stale flag), `History`, `ExchangeView` (scroll and navigation), `DebugLog` (the `debug-log` thread: `close(grace)`), `describe(&io::Error) -> String`.
- App: `debug_mode()`, `d` opens the Exchange screen, `DEBUG` tag, `close_debug_log`; `--debug` flag; `HELP_LINES` gains the debug lines.
- Engine: `redact_body` also redacts a key that a JSON response echoes in escaped form.

- [ ] **Step: Confirm the starting point**

Run: `git status --short` (only untracked `.claude/`, `graphify-out/` and user files) and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: 369 passed (engine: `cargo test --lib engine::` 109 passed).

- [ ] **Step: Apply the tests patch (write the failing tests)**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/tui/app.rs b/src/tui/app.rs
index 56938d3623ded36f8cbd0c9fd83a2bca3aaaf817..f81ecb027e7110b0a5a4b03a25bc0d12e2d6b4ea 100644
--- a/src/tui/app.rs
+++ b/src/tui/app.rs
@@ -2349,13 +2349,16 @@ mod tests {
 
     use super::*;
     use crate::core::{Piece, Position as ChessPosition, START_FEN};
+    use crate::engine::EngineConfig;
     use crate::engine::{MoveSource, analyse};
     use crate::tui::board::square_rect;
     use crate::tui::graphics;
     use crate::tui::panels::HELP_LINES;
-    use crate::tui::test_support::engine::{FakeEngine, REPLY_TIMEOUT, chord, key, mouse, paste};
+    use crate::tui::test_support::engine::{
+        FakeEngine, REPLY_TIMEOUT, SENTINEL_KEY, chord, key, mouse, paste, traced_jev_move,
+    };
     use crate::tui::test_support::harness::{Harness, request};
-    use crate::tui::test_support::{PROMOTION_FEN, TempDir, game_from, sq};
+    use crate::tui::test_support::{PROMOTION_FEN, TempDir, buffer_text, game_from, key_event, sq};
     use crate::tui::worker::{LOCAL_SEARCH_STATUS, spawn_request};
 
     const FOOLS_MATE: [&str; 4] = ["f2f3", "e7e5", "g2g4", "d8h4"];
@@ -3550,6 +3553,7 @@ mod tests {
             generation: request.generation,
             hash: request.hash,
             outcome: EngineOutcome::Move(computer),
+            exchange: None,
         }));
         assert_eq!(h.app.game().moves().last(), Some(&best));
         assert_eq!(h.app.status_line(), ENGINE_ERROR_NOTE);
@@ -4435,4 +4439,441 @@ mod tests {
             "Threefold repetition — draw"
         );
     }
+
+    // ----- debug mode -----
+
+    /// A `width`×`height` app in debug mode with `engine`, logging to `log`.
+    fn debug_app(engine: FakeEngine, (width, height): (u16, u16), log: DebugLog) -> Harness {
+        Harness::build(engine, (width, height), Vec::new(), |app| {
+            app.with_debug(log)
+        })
+    }
+
+    /// An 80×24 Human vs Jev game (the person plays White) in debug mode, with a log that
+    /// has no file (it reports that at the first exchange).
+    fn debug_game() -> Harness {
+        let mut h = debug_app(FakeEngine::jev(), (80, 24), DebugLog::open(None));
+        h.char('2');
+        h
+    }
+
+    /// Ticks until the status line starts with `start`, for up to 10 s.
+    fn wait_for_status(h: &mut Harness, start: &str) {
+        let deadline = Instant::now() + Duration::from_secs(10);
+        while !h.app.status_line().starts_with(start) {
+            assert!(
+                Instant::now() < deadline,
+                "status never said {start:?}: {:?}",
+                h.app.status_line()
+            );
+            std::thread::sleep(Duration::from_millis(5));
+            h.tick();
+        }
+    }
+
+    /// The exchanges kept, as (move, stale).
+    fn kept(h: &Harness) -> Vec<(String, bool)> {
+        h.app
+            .exchanges()
+            .expect("debug mode")
+            .iter()
+            .map(|record| (record.exchange.san.clone(), record.stale))
+            .collect()
+    }
+
+    #[test]
+    fn d_says_when_debug_mode_is_off() {
+        let mut h = hvh();
+        assert!(!h.app.debug_mode());
+        assert!(h.app.exchanges().is_none());
+        h.char('d');
+        assert_eq!(h.app.exchange_view(), None);
+        assert_eq!(h.app.status_line(), DEBUG_OFF);
+        assert_eq!(DEBUG_OFF, "debug mode is off (start with --debug)");
+    }
+
+    #[test]
+    fn d_opens_the_exchange_view_and_esc_closes_it() {
+        let mut h = debug_game();
+        assert!(h.app.debug_mode());
+        h.char('d');
+        assert_eq!(
+            h.app.exchange_view().map(|view| view.shown),
+            Some(None),
+            "open, with nothing to show"
+        );
+        let screen = h.screen();
+        assert!(screen.contains("Jev exchange"), "{screen}");
+        assert!(screen.contains("no Jev requests yet"), "{screen}");
+        assert!(!screen.contains("Board"), "the view covers the screen");
+        assert!(h.app.hit_map().board.is_none());
+        h.char('e');
+        h.char('/');
+        assert!(!h.app.command_focused(), "keys go to the view");
+        h.press(KeyCode::Esc);
+        assert_eq!(h.app.exchange_view(), None);
+        assert!(h.screen().contains("Board"));
+        assert!(h.app.hit_map().board.is_some());
+    }
+
+    #[test]
+    fn a_traced_reply_is_kept_with_the_move_it_was_for() {
+        let mut h = debug_game();
+        h.moves(&["e4"]);
+        h.reply_traced("e7e5");
+        assert_eq!(h.uci(), ["e2e4", "e7e5"]);
+        assert_eq!(kept(&h), [("e5".to_string(), false)]);
+        let history = h.app.exchanges().unwrap();
+        let record = history.last().unwrap();
+        assert_eq!(record.number, 1);
+        assert_eq!((record.exchange.ply, record.exchange.fullmove), (1, 1));
+        assert_eq!(
+            h.app.last_computer().map(|c| c.exchange.is_none()),
+            Some(true),
+            "the Jev panel's move does not hold it"
+        );
+        // Without debug mode an exchange is dropped.
+        let mut h = Harness::with_engine(FakeEngine::jev());
+        h.char('2');
+        h.moves(&["e4"]);
+        h.reply_traced("e7e5");
+        assert_eq!(h.uci(), ["e2e4", "e7e5"]);
+        assert!(h.app.exchanges().is_none());
+    }
+
+    #[test]
+    fn stale_replies_are_kept_and_marked() {
+        let mut h = debug_game();
+        let first = request(&h.command("e4"));
+        h.press(KeyCode::Esc);
+        h.char('u');
+        // Nothing pending any more.
+        h.answer(
+            &first,
+            EngineOutcome::Move(traced_jev_move(first.game.position(), "e7e5")),
+        );
+        assert!(h.uci().is_empty());
+        let second = request(&h.command("e4"));
+        h.press(KeyCode::Esc);
+        h.char('u');
+        h.command("d4");
+        h.press(KeyCode::Esc);
+        // Another request is pending: this one is from an older generation.
+        h.answer(
+            &second,
+            EngineOutcome::Move(traced_jev_move(second.game.position(), "c7c5")),
+        );
+        assert_eq!(h.uci(), ["d2d4"]);
+        h.reply_traced("d7d5");
+        assert_eq!(h.uci(), ["d2d4", "d7d5"]);
+        assert_eq!(
+            kept(&h),
+            [
+                ("e5".to_string(), true),
+                ("c5".to_string(), true),
+                ("d5".to_string(), false),
+            ]
+        );
+        h.char('d');
+        h.press(KeyCode::Left);
+        assert!(h.screen().contains("stale — not played"), "{}", h.screen());
+        h.press(KeyCode::Right);
+        assert!(!h.screen().contains("stale"), "{}", h.screen());
+    }
+
+    #[test]
+    fn an_answer_held_while_paused_is_kept_when_played_or_dropped() {
+        let mut h = debug_app(FakeEngine::jev(), (80, 24), DebugLog::open(None));
+        h.char('5');
+        h.reply("e2e4");
+        h.at_ms(5_000);
+        h.tick();
+        h.char(' ');
+        h.reply_traced("e7e5");
+        assert!(h.app.has_held_move());
+        assert!(kept(&h).is_empty(), "not decided yet");
+        h.char('u');
+        assert_eq!(h.uci(), Vec::<String>::new());
+        assert_eq!(kept(&h), [("e5".to_string(), true)], "undo dropped it");
+
+        let mut h = debug_app(FakeEngine::jev(), (80, 24), DebugLog::open(None));
+        h.char('5');
+        h.reply("e2e4");
+        h.at_ms(5_000);
+        h.tick();
+        h.char(' ');
+        h.reply_traced("e7e5");
+        h.char(' ');
+        assert_eq!(h.uci(), ["e2e4", "e7e5"]);
+        assert_eq!(kept(&h), [("e5".to_string(), false)]);
+    }
+
+    #[test]
+    fn an_illegal_traced_move_is_kept_as_not_played() {
+        let mut h = debug_game();
+        let request = request(&h.command("e4"));
+        h.press(KeyCode::Esc);
+        // Legal in the start position, not after e4 (the pawn is gone from e2).
+        let mut computer = traced_jev_move(&ChessPosition::startpos(), "e2e4");
+        computer.san = "e4?".to_string();
+        h.answer(&request, EngineOutcome::Move(computer));
+        assert!(h.app.engine_failed());
+        assert_eq!(kept(&h), [("e4?".to_string(), true)]);
+    }
+
+    #[test]
+    fn replies_are_played_under_the_view_and_it_stays_on_its_exchange() {
+        let mut h = debug_app(FakeEngine::jev(), (80, 24), DebugLog::open(None));
+        h.char('5');
+        h.char('d');
+        h.reply_traced("e2e4");
+        assert_eq!(h.uci(), ["e2e4"], "played underneath");
+        assert_eq!(h.app.exchange_view().and_then(|v| v.shown), Some(1));
+        assert!(h.screen().contains("exchange 1 of 1"), "{}", h.screen());
+        h.at_ms(5_000);
+        h.tick();
+        h.reply_traced("e7e5");
+        assert_eq!(h.uci(), ["e2e4", "e7e5"]);
+        let screen = h.screen();
+        assert!(
+            screen.contains("exchange 1 of 2 · move 1 · e4 · Jev"),
+            "{screen}"
+        );
+        h.press(KeyCode::Right);
+        assert!(h.screen().contains("exchange 2 of 2 · move 1 · e5"));
+        h.press(KeyCode::Right);
+        assert!(
+            h.screen().contains("exchange 2 of 2"),
+            "stops at the newest"
+        );
+        h.press(KeyCode::Left);
+        h.press(KeyCode::Left);
+        assert!(
+            h.screen().contains("exchange 1 of 2"),
+            "stops at the oldest"
+        );
+    }
+
+    #[test]
+    fn the_view_scrolls_by_row_page_and_end() {
+        let mut h = debug_game();
+        h.moves(&["e4"]);
+        h.reply_traced("e7e5");
+        h.char('d');
+        let view = h.app.exchange_view().expect("open");
+        let (page, max) = (view.page, view.max_scroll);
+        assert!(page > 10 && max > page, "{view:?}");
+        let scroll = |h: &Harness| h.app.exchange_view().map(|v| v.scroll);
+        assert_eq!(scroll(&h), Some(0));
+        assert!(h.screen().contains("│ REQUEST"), "{}", h.screen());
+        h.press(KeyCode::Down);
+        assert_eq!(scroll(&h), Some(1));
+        assert!(!h.screen().contains("│ REQUEST"));
+        h.press(KeyCode::End);
+        assert_eq!(scroll(&h), Some(max));
+        let last_row = h.screen().lines().nth(22).unwrap_or_default().to_string();
+        assert!(last_row.starts_with("│ }"), "{last_row:?}");
+        h.press(KeyCode::Down);
+        assert_eq!(scroll(&h), Some(max), "stops at the end");
+        h.press(KeyCode::Up);
+        assert_eq!(scroll(&h), Some(max - 1));
+        h.press(KeyCode::PageUp);
+        assert_eq!(scroll(&h), Some(max - page));
+        h.press(KeyCode::Home);
+        assert_eq!(scroll(&h), Some(0));
+        h.press(KeyCode::PageDown);
+        assert_eq!(scroll(&h), Some(page - 1));
+        h.press(KeyCode::Up);
+        h.press(KeyCode::Left);
+        assert_eq!(
+            scroll(&h),
+            Some(page - 2),
+            "no older exchange: nothing moves"
+        );
+    }
+
+    #[test]
+    fn a_resize_keeps_the_view_scroll_within_the_body() {
+        let mut h = debug_game();
+        h.moves(&["e4"]);
+        h.reply_traced("e7e5");
+        h.char('d');
+        h.press(KeyCode::End);
+        let small_end = h.app.exchange_view().unwrap().max_scroll;
+        h.terminal.backend_mut().resize(200, 60);
+        h.draw();
+        let view = h.app.exchange_view().unwrap();
+        assert!(view.max_scroll < small_end);
+        assert_eq!(view.scroll, view.max_scroll, "clamped to the new end");
+    }
+
+    #[test]
+    fn the_mouse_wheel_scrolls_the_view_and_clicks_do_nothing() {
+        let mut h = debug_game();
+        h.moves(&["e4"]);
+        h.reply_traced("e7e5");
+        let e2 = h.square_cell(sq("d2"));
+        h.char('d');
+        h.click(e2.0, e2.1);
+        assert_eq!(h.app.selected(), None);
+        assert!(h.app.exchange_view().is_some());
+        h.mouse(MouseEventKind::ScrollDown, 10, 10);
+        assert_eq!(h.app.exchange_view().map(|v| v.scroll), Some(3));
+        h.mouse(MouseEventKind::ScrollUp, 10, 10);
+        assert_eq!(h.app.exchange_view().map(|v| v.scroll), Some(0));
+        h.send(paste("e4\n"));
+        assert_eq!(
+            h.app.command_text(),
+            "",
+            "a paste is not typed under the view"
+        );
+    }
+
+    #[test]
+    fn ctrl_c_asks_over_the_view_and_no_goes_back_to_it() {
+        let mut h = debug_game();
+        h.moves(&["e4"]);
+        h.reply_traced("e7e5");
+        h.char('d');
+        h.ctrl('c');
+        assert_eq!(h.app.dialog_name(), Some("quit"));
+        assert!(h.screen().contains("Quit the game in progress?"));
+        h.char('n');
+        assert!(h.app.exchange_view().is_some());
+        assert!(h.screen().contains("exchange 1 of 1"));
+    }
+
+    #[test]
+    fn the_status_border_says_debug_in_debug_mode() {
+        let top_row = |h: &Harness| h.screen().lines().next().unwrap_or_default().to_string();
+        let mut h = Harness::sized(FakeEngine::jev(), 120, 40);
+        h.char('2');
+        assert!(!top_row(&h).contains("DEBUG"));
+        let mut h = debug_app(FakeEngine::jev(), (120, 40), DebugLog::open(None));
+        h.char('2');
+        let row = top_row(&h);
+        assert!(row.contains("┌ Status ─ DEBUG ─"), "{row}");
+        assert!(row.contains(" You (White) vs Jev ┐"), "{row}");
+    }
+
+    #[test]
+    fn each_exchange_becomes_a_log_line() {
+        let dir = TempDir::new("debug-app");
+        let path = dir.join("jev.jsonl");
+        let mut h = debug_app(FakeEngine::jev(), (80, 24), DebugLog::start(path.clone()));
+        h.char('2');
+        h.moves(&["e4"]);
+        h.reply_traced("e7e5");
+        h.moves(&["Nf3"]);
+        h.reply_traced("b8c6");
+        h.app.close_debug_log(Duration::from_secs(10));
+        let text = fs::read_to_string(&path).expect("log written");
+        let lines: Vec<serde_json::Value> = text
+            .lines()
+            .map(|line| serde_json::from_str(line).expect("JSON line"))
+            .collect();
+        assert_eq!(lines.len(), 2);
+        assert_eq!(lines[0]["played"], "e5");
+        assert_eq!(lines[0]["ply"], 1);
+        assert_eq!(lines[1]["played"], "Nc6");
+        assert_eq!(lines[1]["ply"], 3);
+        assert_eq!(lines[1]["stale"], false);
+        assert_eq!(h.app.status_line(), "", "no failure to report");
+    }
+
+    #[test]
+    fn a_log_failure_is_shown_once() {
+        let dir = TempDir::new("debug-app");
+        // A folder where the file should be.
+        let mut h = debug_app(
+            FakeEngine::jev(),
+            (120, 40),
+            DebugLog::start(dir.path().to_path_buf()),
+        );
+        h.char('2');
+        h.moves(&["e4"]);
+        h.reply_traced("e7e5");
+        wait_for_status(&mut h, "debug log disabled: it is a folder: ");
+        assert!(h.app.message().is_some_and(|m| m.is_error));
+        assert!(
+            h.app
+                .status_line()
+                .ends_with(&dir.path().display().to_string())
+        );
+        h.moves(&["Nf3"]);
+        assert_eq!(h.app.status_line(), "", "a move clears it");
+        h.reply_traced("b8c6");
+        for _ in 0..10 {
+            std::thread::sleep(Duration::from_millis(5));
+            h.tick();
+        }
+        assert_eq!(h.app.status_line(), "", "reported once");
+        assert_eq!(h.app.exchanges().map(History::len), Some(2), "still kept");
+    }
+
+    #[test]
+    fn a_log_without_a_path_says_so_at_the_first_exchange() {
+        let mut h = debug_game();
+        h.moves(&["e4"]);
+        assert_eq!(h.app.status_line(), "");
+        h.reply_traced("e7e5");
+        assert_eq!(
+            h.app.status_line(),
+            "debug log disabled: no log file (set RCHESS_DEBUG_LOG, XDG_STATE_HOME or HOME)"
+        );
+    }
+
+    #[test]
+    fn the_api_key_never_reaches_the_screen_or_the_log() {
+        let dir = TempDir::new("debug-app");
+        let path = dir.join("jev.jsonl");
+        // A real player holding the key; building it is offline, and no move is asked of it.
+        let config = EngineConfig {
+            api_key: Some(SENTINEL_KEY.to_string()),
+            trace: true,
+            ..EngineConfig::default()
+        };
+        let engine = Arc::new(crate::engine::ComputerPlayer::from_config(config));
+        let mut app = App::new(engine, GlyphSet::Solid, true, Vec::new())
+            .with_home(None)
+            .with_debug(DebugLog::start(path.clone()));
+        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40))
+            .expect("test terminal");
+        let now = Instant::now();
+        let actions = app.handle(AppEvent::Term(key_event(KeyCode::Char('3'))), now);
+        let request = request(&actions);
+        let computer = traced_jev_move(request.game.position(), "e2e4");
+        let _ = app.handle(
+            AppEvent::Engine(EngineReply::new(&request, EngineOutcome::Move(computer))),
+            now,
+        );
+        let _ = app.handle(AppEvent::Term(key_event(KeyCode::Char('d'))), now);
+        let mut seen = String::new();
+        loop {
+            terminal.draw(|frame| app.render(frame, now)).expect("draw");
+            seen.push_str(&buffer_text(terminal.backend().buffer()));
+            let view = app.exchange_view().expect("open");
+            if view.scroll == view.max_scroll {
+                break;
+            }
+            let _ = app.handle(AppEvent::Term(key_event(KeyCode::PageDown)), now);
+        }
+        assert!(seen.contains("Authorization: Bearer <redacted>"), "{seen}");
+        app.close_debug_log(Duration::from_secs(10));
+        let log = fs::read_to_string(&path).expect("log written");
+        assert!(log.contains("Bearer <redacted>"));
+        for text in [seen, log] {
+            assert!(!text.contains(SENTINEL_KEY));
+        }
+    }
+
+    #[test]
+    fn the_help_lists_the_exchange_view() {
+        let mut h = debug_game();
+        h.char('?');
+        assert!(
+            h.screen()
+                .contains("d         exchange view (start with --debug)")
+        );
+    }
 }
diff --git a/src/tui/debug.rs b/src/tui/debug.rs
new file mode 100644
index 0000000000000000000000000000000000000000..2d7eded9d7289ca71b2439069e3b0941079e290d
--- /dev/null
+++ b/src/tui/debug.rs
@@ -0,0 +1,619 @@
+//! Debug mode (spec 9.4): every Jev request and its answers, kept for the exchange
+//! view and appended to a log file.
+//!
+//! Debug mode is on with `--debug` or `RCHESS_DEBUG` ([`enabled`]). The engine then
+//! records each HTTP exchange with Jev (`EngineConfig::trace`), and the worker turns
+//! it into an [`Exchange`] on the engine thread, where its body text is also rendered,
+//! once. The app keeps the last [`HISTORY_LEN`] of them in a [`History`], replies it
+//! discarded as stale included, and `d` shows them full screen ([`ExchangeView`]).
+//!
+//! Each exchange is also appended to the debug log ([`log_path`]) as one JSON object
+//! per line ([`log_line`]). A thread named `debug-log` does the writing, fed by a
+//! channel ([`DebugLog`]), so the UI never waits on the disk. The log file is created
+//! with the first exchange, in a folder made private to the user; the first error
+//! stops the log for the session and is reported once.
+//!
+//! The API key never reaches any of this: the engine redacts it while recording, so
+//! the `Authorization` header reads `Bearer <redacted>`.
+
+#[cfg(test)]
+mod tests {
+    use std::collections::HashMap;
+    use std::fs;
+    use std::time::Instant;
+
+    use serde_json::json;
+
+    use super::*;
+    use crate::engine::JevAttempt;
+    use crate::tui::test_support::TempDir;
+    use crate::tui::test_support::engine::{SENTINEL_KEY, jev_exchange, jev_move};
+
+    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
+        let map: HashMap<String, String> = pairs
+            .iter()
+            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
+            .collect();
+        move |key| map.get(key).cloned()
+    }
+
+    /// `ms` milliseconds after the epoch.
+    fn at_ms(ms: u64) -> SystemTime {
+        UNIX_EPOCH + Duration::from_millis(ms)
+    }
+
+    /// The exchange [`jev_exchange`] behind Jev's e4 in the start position.
+    fn exchange() -> Exchange {
+        let game = Game::new();
+        let computer = jev_move(game.position(), "e2e4");
+        Exchange::new(&game, &computer, jev_exchange())
+    }
+
+    fn record(stale: bool) -> Record {
+        let mut history = History::new();
+        history
+            .push(exchange(), stale, at_ms(1_790_000_000_123))
+            .clone()
+    }
+
+    fn texts(exchange: &Exchange) -> Vec<&str> {
+        exchange
+            .body()
+            .iter()
+            .map(|line| line.text.as_str())
+            .collect()
+    }
+
+    /// Asks `log` for its failure until one comes, for up to 10 s.
+    fn wait_for_failure(log: &mut DebugLog) -> LogFailure {
+        let deadline = Instant::now() + Duration::from_secs(10);
+        loop {
+            if let Some(failure) = log.failure() {
+                return failure;
+            }
+            assert!(Instant::now() < deadline, "no failure reported");
+            thread::sleep(Duration::from_millis(5));
+        }
+    }
+
+    #[cfg(unix)]
+    fn mode(path: &Path) -> u32 {
+        use std::os::unix::fs::PermissionsExt;
+        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
+    }
+
+    // ----- switches and paths -----
+
+    #[test]
+    fn debug_mode_is_on_with_the_flag_or_a_set_variable() {
+        assert!(!enabled(false, env(&[])));
+        assert!(enabled(true, env(&[])));
+        assert!(enabled(true, env(&[(DEBUG_ENV, "0")])), "the flag wins");
+        for on in ["1", "yes", "true", "00", " 1 ", "off"] {
+            assert!(enabled(false, env(&[(DEBUG_ENV, on)])), "{on:?}");
+        }
+        for off in ["", "0", " 0 ", "  "] {
+            assert!(!enabled(false, env(&[(DEBUG_ENV, off)])), "{off:?}");
+        }
+    }
+
+    #[test]
+    fn the_log_path_follows_the_variables_in_order() {
+        let log = |pairs: &[(&str, &str)]| log_path(env(pairs));
+        let all = [
+            (DEBUG_LOG_ENV, "/tmp/mine.jsonl"),
+            ("XDG_STATE_HOME", "/xdg/state"),
+            ("HOME", "/home/ana"),
+        ];
+        assert_eq!(log(&all), Some(PathBuf::from("/tmp/mine.jsonl")));
+        assert_eq!(
+            log(&[(DEBUG_LOG_ENV, "relative/debug.jsonl")]),
+            Some(PathBuf::from("relative/debug.jsonl")),
+            "used as given"
+        );
+        assert_eq!(
+            log(&all[1..]),
+            Some(PathBuf::from("/xdg/state/rchess/jev-debug.jsonl"))
+        );
+        assert_eq!(
+            log(&[(DEBUG_LOG_ENV, ""), ("XDG_STATE_HOME", "/xdg/state")]),
+            Some(PathBuf::from("/xdg/state/rchess/jev-debug.jsonl")),
+            "an empty RCHESS_DEBUG_LOG is unset"
+        );
+        assert_eq!(
+            log(&all[2..]),
+            Some(PathBuf::from(
+                "/home/ana/.local/state/rchess/jev-debug.jsonl"
+            ))
+        );
+        for xdg in ["", "relative/state"] {
+            assert_eq!(
+                log(&[("XDG_STATE_HOME", xdg), ("HOME", "/home/ana")]),
+                Some(PathBuf::from(
+                    "/home/ana/.local/state/rchess/jev-debug.jsonl"
+                )),
+                "XDG_STATE_HOME {xdg:?} is ignored"
+            );
+        }
+        assert_eq!(log(&[]), None);
+        assert_eq!(log(&[("HOME", ""), ("XDG_STATE_HOME", "state")]), None);
+    }
+
+    #[test]
+    fn times_are_rfc_3339_in_utc_with_milliseconds() {
+        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
+        assert_eq!(
+            rfc3339(at_ms(1_790_000_000_123)),
+            "2026-09-21T14:13:20.123Z"
+        );
+        // 2024-02-29 is a leap day; 23:59:59.999 is its last millisecond.
+        assert_eq!(
+            rfc3339(at_ms(1_709_251_199_999)),
+            "2024-02-29T23:59:59.999Z"
+        );
+        // Sub-millisecond parts are cut, also before the epoch.
+        assert_eq!(
+            rfc3339(UNIX_EPOCH + Duration::from_micros(1_999)),
+            "1970-01-01T00:00:00.001Z"
+        );
+        assert_eq!(
+            rfc3339(UNIX_EPOCH - Duration::from_micros(1)),
+            "1969-12-31T23:59:59.999Z"
+        );
+    }
+
+    // ----- exchanges -----
+
+    #[test]
+    fn an_exchange_names_the_move_it_was_for() {
+        let game =
+            crate::tui::test_support::game_from(crate::core::START_FEN, &["e2e4", "e7e5", "g1f3"]);
+        let computer = jev_move(game.position(), "b8c6");
+        let exchange = Exchange::new(&game, &computer, jev_exchange());
+        assert_eq!(exchange.ply, 3);
+        assert_eq!(exchange.fullmove, 2);
+        assert_eq!(exchange.san, "Nc6");
+        assert_eq!(exchange.source, "Jev");
+        assert_eq!(exchange.latency, Duration::from_millis(1234));
+        assert_eq!(exchange.http, jev_exchange());
+        assert_eq!(exchange.attempts(), 2);
+        assert_eq!(exchange.status(), "HTTP 200");
+    }
+
+    #[test]
+    fn the_body_shows_the_request_and_every_response() {
+        let exchange = exchange();
+        let lines = texts(&exchange);
+        let pretty_body = serde_json::to_string_pretty(&jev_exchange().body).unwrap();
+        let body_lines: Vec<&str> = pretty_body.lines().collect();
+        let mut expected = vec![
+            "REQUEST",
+            "POST https://api.typesafe.ai/v1/systemone",
+            "Authorization: Bearer <redacted>",
+            "Content-Type: application/json",
+            "",
+        ];
+        expected.extend(&body_lines);
+        expected.extend([
+            "",
+            "RESPONSE 1 · HTTP 503 · 120 ms",
+            "error: HTTP 503: busy \\u{1b}[2J now",
+            "busy \\u{1b}[2J now",
+            "\\ttry again",
+            "",
+            "RESPONSE 2 · HTTP 200 · 850 ms",
+            "{",
+            "  \"answers\": {",
+        ]);
+        assert_eq!(&lines[..expected.len()], &expected[..]);
+        assert_eq!(lines.last(), Some(&"}"));
+        let kinds: Vec<LineKind> = exchange.body().iter().map(|line| line.kind).collect();
+        assert_eq!(kinds[0], LineKind::Heading);
+        let error = expected.len() - 7;
+        assert_eq!(
+            kinds[error - 1..=error],
+            [LineKind::Heading, LineKind::Error]
+        );
+        assert!(
+            exchange
+                .body()
+                .iter()
+                .all(|line| !line.text.chars().any(char::is_control)),
+            "control characters are escaped"
+        );
+    }
+
+    #[test]
+    fn failed_and_empty_responses_say_so() {
+        let mut http = jev_exchange();
+        http.attempts = vec![
+            JevAttempt {
+                status: None,
+                response: None,
+                error: Some("network error: connection refused".to_string()),
+                elapsed: Duration::from_millis(5),
+            },
+            JevAttempt {
+                status: Some(502),
+                response: None,
+                error: Some("HTTP 502: <unreadable body>".to_string()),
+                elapsed: Duration::from_millis(7),
+            },
+            JevAttempt {
+                status: Some(500),
+                response: Some(String::new()),
+                error: Some("HTTP 500: ".to_string()),
+                elapsed: Duration::from_millis(9),
+            },
+        ];
+        let game = Game::new();
+        let exchange = Exchange::new(&game, &jev_move(game.position(), "e2e4"), http);
+        let lines = texts(&exchange);
+        let start = lines
+            .iter()
+            .position(|line| line.starts_with("RESPONSE 1"))
+            .expect("a response block");
+        assert_eq!(
+            lines[start..],
+            [
+                "RESPONSE 1 · no response · 5 ms",
+                "error: network error: connection refused",
+                "",
+                "RESPONSE 2 · HTTP 502 · 7 ms",
+                "error: HTTP 502: <unreadable body>",
+                "(body not readable)",
+                "",
+                "RESPONSE 3 · HTTP 500 · 9 ms",
+                "error: HTTP 500: ",
+                "(empty body)",
+            ]
+        );
+        assert_eq!(exchange.status(), "HTTP 500");
+        let mut http = jev_exchange();
+        http.attempts.truncate(1);
+        http.attempts[0].status = None;
+        let exchange = Exchange::new(&game, &jev_move(game.position(), "e2e4"), http.clone());
+        assert_eq!(exchange.status(), "no response");
+        http.attempts.clear();
+        let exchange = Exchange::new(&game, &jev_move(game.position(), "e2e4"), http);
+        assert_eq!(exchange.status(), "not sent");
+        assert_eq!(exchange.attempts(), 0);
+    }
+
+    #[test]
+    fn body_lines_wrap_between_characters() {
+        let line = BodyLine::new("abcdefghij".to_string(), LineKind::Text);
+        assert_eq!(line.width, 10);
+        assert_eq!(line.rows(4), 3);
+        assert_eq!(line.wrapped(4), ["abcd", "efgh", "ij"]);
+        assert_eq!(line.rows(10), 1);
+        assert_eq!(line.rows(0), 10, "at least one cell per row");
+        let empty = BodyLine::new(String::new(), LineKind::Text);
+        assert_eq!(empty.rows(8), 1);
+        assert_eq!(empty.wrapped(8), [""]);
+        // A wide character never straddles the edge.
+        let wide = BodyLine::new("ab日本cd".to_string(), LineKind::Text);
+        assert_eq!(wide.width, 8);
+        assert_eq!(wide.wrapped(3), ["ab", "日", "本c", "d"]);
+        assert_eq!(wide.rows(3), 4);
+    }
+
+    // ----- history and view -----
+
+    #[test]
+    fn the_history_keeps_the_last_fifty_and_numbers_them() {
+        let mut history = History::new();
+        assert!(history.is_empty());
+        for n in 1..=55_u64 {
+            let record = history.push(exchange(), n % 10 == 0, at_ms(n));
+            assert_eq!(record.number, n);
+        }
+        assert_eq!(history.len(), HISTORY_LEN);
+        assert_eq!(history.get(0).map(|r| r.number), Some(6));
+        assert_eq!(history.last().map(|r| r.number), Some(55));
+        assert_eq!(history.index_of(6), Some(0));
+        assert_eq!(history.index_of(5), None, "dropped");
+        assert_eq!(history.index_of(55), Some(49));
+        let stale: Vec<u64> = history
+            .iter()
+            .filter(|r| r.stale)
+            .map(|r| r.number)
+            .collect();
+        assert_eq!(stale, [10, 20, 30, 40, 50]);
+    }
+
+    #[test]
+    fn the_view_steps_between_exchanges_and_stays_on_its_own() {
+        let mut history = History::new();
+        let mut view = ExchangeView::open(&history);
+        assert_eq!(view.shown, None);
+        assert_eq!(view.index(&history), None);
+        view.older(&history);
+        view.newer(&history);
+        assert_eq!(view.shown, None);
+
+        let first = history.push(exchange(), false, at_ms(1)).number;
+        view.follow(first);
+        assert_eq!(view.shown, Some(1), "the first exchange is shown at once");
+        for n in 2..=3 {
+            let number = history.push(exchange(), false, at_ms(n)).number;
+            view.follow(number);
+        }
+        assert_eq!(view.shown, Some(1), "later ones do not move the view");
+        view.newer(&history);
+        view.newer(&history);
+        view.newer(&history);
+        assert_eq!(view.index(&history), Some(2), "stops at the newest");
+        view.older(&history);
+        assert_eq!(view.shown, Some(2));
+
+        let view = ExchangeView::open(&history);
+        assert_eq!(view.shown, Some(3), "opens on the newest");
+    }
+
+    #[test]
+    fn a_dropped_exchange_leaves_the_view_on_the_oldest() {
+        let mut history = History::new();
+        history.push(exchange(), false, at_ms(1));
+        let mut view = ExchangeView::open(&history);
+        for n in 2..=51 {
+            history.push(exchange(), false, at_ms(n));
+        }
+        assert_eq!(history.index_of(1), None);
+        assert_eq!(view.index(&history), Some(0));
+        view.newer(&history);
+        assert_eq!(view.shown, Some(3));
+    }
+
+    #[test]
+    fn scrolling_stops_at_the_ends_and_resets_on_another_exchange() {
+        let mut history = History::new();
+        for n in 1..=2 {
+            history.push(exchange(), false, at_ms(n));
+        }
+        let mut view = ExchangeView::open(&history);
+        view.page = 10;
+        view.max_scroll = 25;
+        view.down(1);
+        assert_eq!(view.scroll, 1);
+        view.page_down();
+        assert_eq!(view.scroll, 10, "a page keeps one row");
+        view.end();
+        assert_eq!(view.scroll, 25);
+        view.down(3);
+        view.page_down();
+        assert_eq!(view.scroll, 25);
+        view.up(1);
+        assert_eq!(view.scroll, 24, "one up from the end");
+        view.page_up();
+        assert_eq!(view.scroll, 15);
+        view.home();
+        assert_eq!(view.scroll, 0);
+        view.up(5);
+        view.page_up();
+        assert_eq!(view.scroll, 0);
+
+        view.page = 1;
+        view.page_down();
+        assert_eq!(view.scroll, 1, "a page is at least one row");
+        view.end();
+        view.older(&history);
+        assert_eq!(
+            view,
+            ExchangeView {
+                shown: Some(1),
+                ..ExchangeView::default()
+            }
+        );
+        view.older(&history);
+        view.max_scroll = 25;
+        view.end();
+        view.older(&history);
+        assert_eq!(view.scroll, 25, "already the oldest: nothing moves");
+    }
+
+    // ----- log records -----
+
+    #[test]
+    fn a_log_line_is_one_json_object_with_every_field_in_order() {
+        let line = log_line(&record(false)).expect("serializes");
+        assert!(!line.contains('\n'));
+        let keys = [
+            "\"time\"",
+            "\"ply\"",
+            "\"played\"",
+            "\"source\"",
+            "\"stale\"",
+            "\"request\"",
+            "\"attempts\"",
+        ];
+        let at: Vec<usize> = keys
+            .iter()
+            .map(|key| line.find(key).unwrap_or_else(|| panic!("{key} in {line}")))
+            .collect();
+        assert!(at.is_sorted(), "{line}");
+        let value: Value = serde_json::from_str(&line).expect("valid JSON");
+        assert_eq!(
+            value,
+            json!({
+                "time": "2026-09-21T14:13:20.123Z",
+                "ply": 0,
+                "played": "e4",
+                "source": "Jev",
+                "stale": false,
+                "request": {
+                    "method": "POST",
+                    "url": "https://api.typesafe.ai/v1/systemone",
+                    "headers": {
+                        "Authorization": "Bearer <redacted>",
+                        "Content-Type": "application/json",
+                    },
+                    "body": jev_exchange().body,
+                },
+                "attempts": [
+                    {
+                        "status": 503,
+                        "elapsed_ms": 120,
+                        "response": "busy \u{1b}[2J now\n\ttry again",
+                        "error": "HTTP 503: busy \u{1b}[2J now",
+                    },
+                    {
+                        "status": 200,
+                        "elapsed_ms": 850,
+                        "response": serde_json::from_str::<Value>(
+                            jev_exchange().attempts[1].response.as_deref().unwrap()
+                        ).unwrap(),
+                        "error": null,
+                    },
+                ],
+            })
+        );
+        let stale: Value = serde_json::from_str(&log_line(&record(true)).unwrap()).unwrap();
+        assert_eq!(stale["stale"], json!(true));
+    }
+
+    #[test]
+    fn a_failed_attempt_logs_nulls() {
+        let mut http = jev_exchange();
+        http.attempts = vec![JevAttempt {
+            status: None,
+            response: None,
+            error: Some("request timed out".to_string()),
+            elapsed: Duration::from_millis(5000),
+        }];
+        let game = Game::new();
+        let mut computer = jev_move(game.position(), "e2e4");
+        computer.source = crate::engine::MoveSource::Fallback;
+        let mut history = History::new();
+        let record = history.push(Exchange::new(&game, &computer, http), false, at_ms(0));
+        let value: Value = serde_json::from_str(&log_line(record).unwrap()).unwrap();
+        assert_eq!(value["source"], json!("local search"));
+        assert_eq!(
+            value["attempts"],
+            json!([{
+                "status": null,
+                "elapsed_ms": 5000,
+                "response": null,
+                "error": "request timed out",
+            }])
+        );
+    }
+
+    #[test]
+    fn the_key_is_nowhere_in_an_exchange_or_its_log_line() {
+        let record = record(false);
+        let line = log_line(&record).unwrap();
+        let text = texts(&record.exchange).join("\n");
+        for shown in [line, text, format!("{record:?}")] {
+            assert!(!shown.contains(SENTINEL_KEY), "{shown}");
+            assert!(shown.contains("Bearer <redacted>"), "{shown}");
+        }
+    }
+
+    // ----- the log thread -----
+
+    #[test]
+    fn the_log_is_created_private_and_gets_one_line_per_record() {
+        let dir = TempDir::new("debug-log");
+        let path = dir.path().join("state").join("rchess").join(LOG_FILE);
+        let mut log = DebugLog::start(path.clone());
+        assert!(
+            !dir.path().join("state").exists(),
+            "nothing before a record"
+        );
+        let (first, second) = (record(false), record(true));
+        log.write(&first);
+        log.write(&second);
+        log.close(Duration::from_secs(10));
+        let text = fs::read_to_string(&path).expect("log written");
+        let lines: Vec<&str> = text.lines().collect();
+        assert_eq!(
+            lines,
+            [log_line(&first).unwrap(), log_line(&second).unwrap()]
+        );
+        assert!(text.ends_with('\n'));
+        #[cfg(unix)]
+        {
+            assert_eq!(mode(&path), 0o600);
+            assert_eq!(mode(path.parent().unwrap()), 0o700);
+            assert_eq!(mode(&dir.path().join("state")), 0o700);
+        }
+    }
+
+    #[test]
+    fn an_existing_log_is_appended_to() {
+        let dir = TempDir::new("debug-log");
+        let path = dir.join("jev.jsonl");
+        fs::write(&path, "{\"earlier\":true}\n").unwrap();
+        let mut log = DebugLog::start(path.clone());
+        log.write(&record(false));
+        log.close(Duration::from_secs(10));
+        let text = fs::read_to_string(&path).unwrap();
+        assert_eq!(text.lines().count(), 2);
+        assert!(text.starts_with("{\"earlier\":true}\n{\"time\""));
+    }
+
+    #[test]
+    fn a_write_error_is_reported_once_and_ends_the_log() {
+        let dir = TempDir::new("debug-log");
+        // A folder where the file should be: opening it fails.
+        let path = dir.path().to_path_buf();
+        let mut log = DebugLog::start(path.clone());
+        assert_eq!(log.failure(), None, "no record, no error");
+        log.write(&record(false));
+        assert_eq!(
+            wait_for_failure(&mut log),
+            LogFailure {
+                reason: "it is a folder".to_string(),
+                path: Some(path),
+            }
+        );
+        log.write(&record(false));
+        thread::sleep(Duration::from_millis(20));
+        assert_eq!(log.failure(), None, "reported once");
+        log.close(Duration::from_secs(10));
+    }
+
+    #[test]
+    fn a_missing_path_is_reported_at_the_first_record() {
+        let mut log = DebugLog::open(None);
+        assert_eq!(log.failure(), None);
+        log.write(&record(false));
+        let failure = log.failure().expect("reported");
+        assert_eq!(failure.path, None);
+        assert!(failure.reason.contains(DEBUG_LOG_ENV), "{}", failure.reason);
+        log.write(&record(false));
+        assert_eq!(log.failure(), None);
+    }
+
+    #[test]
+    fn a_session_keeps_every_exchange_even_after_the_log_fails() {
+        let dir = TempDir::new("debug-log");
+        let mut session = DebugSession::new(DebugLog::start(dir.path().to_path_buf()));
+        assert_eq!(session.record(exchange(), false, at_ms(1)), 1);
+        let deadline = Instant::now() + Duration::from_secs(10);
+        while session.log_failure().is_none() {
+            assert!(Instant::now() < deadline, "no failure reported");
+            thread::sleep(Duration::from_millis(5));
+        }
+        assert_eq!(session.record(exchange(), true, at_ms(2)), 2);
+        assert_eq!(session.log_failure(), None);
+        assert_eq!(session.history().len(), 2);
+        assert!(session.history().last().is_some_and(|r| r.stale));
+        session.close_log(Duration::from_secs(10));
+        assert_eq!(session.record(exchange(), false, at_ms(3)), 3);
+    }
+
+    #[test]
+    fn a_closed_session_log_writes_nothing_more() {
+        let dir = TempDir::new("debug-log");
+        let path = dir.join("jev.jsonl");
+        let mut session = DebugSession::new(DebugLog::start(path.clone()));
+        session.record(exchange(), false, at_ms(1));
+        session.close_log(Duration::from_secs(10));
+        session.record(exchange(), false, at_ms(2));
+        assert_eq!(session.log_failure(), None);
+        let text = fs::read_to_string(&path).unwrap();
+        assert_eq!(text.lines().count(), 1);
+    }
+}
diff --git a/src/tui/event.rs b/src/tui/event.rs
index e72de8c89e692656770dae26d8779951c123ef5a..6b3b41b2d39528ed77fba14911ae7df59485bade 100644
--- a/src/tui/event.rs
+++ b/src/tui/event.rs
@@ -136,6 +136,7 @@ mod tests {
             generation,
             hash: 0,
             outcome: EngineOutcome::GameOver,
+            exchange: None,
         }
     }
 
diff --git a/src/tui/mod.rs b/src/tui/mod.rs
index 8b353b22fc18abc36ddd6b7fd0050086b85c150e..f5d84934a52156ed48a17637d733ed37e20bc588 100644
--- a/src/tui/mod.rs
+++ b/src/tui/mod.rs
@@ -9,6 +9,7 @@
 
 pub mod app;
 pub mod board;
+pub mod debug;
 pub mod event;
 pub mod files;
 pub mod glyphs;
@@ -436,7 +437,8 @@ mod tests {
     use super::board::CellSize;
     use super::glyphs::{GlyphSet, ImageSupport};
     use super::graphics::{Graphics, LateAnswers, picker_for};
-    use super::test_support::engine::{REPLY_TIMEOUT, chars, key, mouse};
+    use super::test_support::TempDir;
+    use super::test_support::engine::{FakeEngine, REPLY_TIMEOUT, Turn, chars, key, mouse};
     use super::test_support::{late_kitty_answer, uci_moves};
     use super::*;
     use crate::core::Color as Side;
@@ -452,6 +454,7 @@ mod tests {
         Cli::Play(Options {
             glyphs: glyphs.map(str::to_string),
             warnings: warnings.iter().map(|w| (*w).to_string()).collect(),
+            ..Options::default()
         })
     }
 
@@ -515,6 +518,49 @@ mod tests {
         );
     }
 
+    #[test]
+    fn debug_mode_can_be_asked_for() {
+        let debug = |list: &[&str]| match parse_args(args(list)) {
+            Cli::Play(options) => options,
+            Cli::Help => panic!("expected Play"),
+        };
+        assert!(!debug(&[]).debug);
+        assert_eq!(
+            debug(&["--debug", "--glyphs", "ascii"]),
+            Options {
+                glyphs: Some("ascii".to_string()),
+                debug: true,
+                warnings: Vec::new(),
+            }
+        );
+        assert_eq!(
+            debug(&["--glyphs", "--debug"]),
+            Options {
+                glyphs: None,
+                debug: true,
+                warnings: vec!["--glyphs needs a value: image, solid, outline or ascii".into()],
+            },
+            "--debug is not a glyph set"
+        );
+        assert_eq!(
+            debug(&["--debug=1"]).warnings,
+            [r#"ignored unknown argument "--debug=1" (see --help)"#]
+        );
+    }
+
+    #[test]
+    fn debug_mode_turns_the_engine_trace_on() {
+        assert!(engine_config(EngineConfig::default(), true).trace);
+        let config = EngineConfig {
+            model: "jev-x".to_string(),
+            trace: true,
+            ..EngineConfig::default()
+        };
+        let config = engine_config(config, false);
+        assert!(!config.trace);
+        assert_eq!(config.model, "jev-x", "nothing else changes");
+    }
+
     #[test]
     fn a_missing_glyphs_value_is_a_warning() {
         let missing = "--glyphs needs a value: image, solid, outline or ascii";
@@ -562,6 +608,7 @@ mod tests {
     fn usage_names_every_option_and_variable() {
         for name in [
             "--glyphs",
+            "--debug",
             "--help",
             "JEV_API_KEY",
             "TYPESAFE_API_KEY",
@@ -570,13 +617,15 @@ mod tests {
             "JEV_FILTER_LOSING",
             "RCHESS_GLYPHS",
             "RCHESS_IMAGES",
+            "RCHESS_DEBUG",
+            "RCHESS_DEBUG_LOG",
             "NO_COLOR",
             "COLORTERM",
         ] {
             assert!(USAGE.contains(name), "{name}");
         }
         assert!(USAGE.contains("Usage: chess "));
-        assert!(USAGE.contains("[--glyphs image|solid|outline|ascii]"));
+        assert!(USAGE.contains("[--glyphs image|solid|outline|ascii] [--debug]"));
         assert!(USAGE.lines().all(|line| line.chars().count() <= 80));
     }
 
@@ -668,6 +717,35 @@ mod tests {
         );
     }
 
+    #[test]
+    fn debug_mode_starts_the_log_where_the_environment_says() {
+        let dir = TempDir::new("debug-start");
+        let path = dir.join("log").join("jev.jsonl");
+        let log = path.display().to_string();
+        let vars = |debug: &str| {
+            let (debug, log) = (debug.to_string(), log.clone());
+            move |name: &str| match name {
+                "RCHESS_DEBUG" => Some(debug.clone()),
+                "RCHESS_DEBUG_LOG" => Some(log.clone()),
+                _ => None,
+            }
+        };
+        let off = Graphics::off(CellSize::DEFAULT);
+        let app = build_app(options(None, &[]), local_engine(), off.clone(), vars("0"));
+        assert!(!app.debug_mode());
+        let mut app = build_app(options(None, &[]), local_engine(), off.clone(), vars("1"));
+        assert!(app.debug_mode());
+        assert!(app.exchanges().is_some_and(debug::History::is_empty));
+        app.close_debug_log(Duration::from_secs(10));
+        assert!(!path.exists(), "no exchange, no file");
+        let flagged = Options {
+            debug: true,
+            ..Options::default()
+        };
+        let app = build_app(flagged, local_engine(), off, vars(""));
+        assert!(app.debug_mode(), "--debug alone");
+    }
+
     // ----- main loop -----
 
     /// One scripted `next_batch` result.
@@ -783,6 +861,42 @@ mod tests {
         assert!(uci_moves(app.game()).is_empty());
     }
 
+    #[test]
+    fn a_traced_move_reaches_the_exchange_view_and_the_log() {
+        // The worker thread moves the fake engine's exchange into its reply; the app keeps
+        // it for `d` and the log thread appends it.
+        let dir = TempDir::new("debug-loop");
+        let path = dir.join("jev.jsonl");
+        let engine = Arc::new(FakeEngine::jev().scripted([Turn::Traced("e2e4")]));
+        let mut app = App::new(engine, GlyphSet::Solid, true, Vec::new())
+            .with_home(None)
+            .with_debug(DebugLog::start(path.clone()));
+        let quit = AtomicI32::new(0);
+        let run = drive(
+            &mut app,
+            &quit,
+            vec![
+                Step::Events(chars("3")),
+                Step::AwaitEngine,
+                Step::Events(chars("d")),
+                Step::Events(vec![key(KeyCode::End)]),
+                Step::Signal,
+            ],
+        );
+        run.result.expect("loop ends cleanly");
+        assert_eq!(uci_moves(app.game()), ["e2e4"]);
+        let view = app.exchange_view().expect("the view is open");
+        assert_eq!(view.shown, Some(1));
+        assert_eq!(view.scroll, view.max_scroll);
+        assert!(view.max_scroll > 0);
+        app.close_debug_log(Duration::from_secs(10));
+        let log = std::fs::read_to_string(&path).expect("log written");
+        let line: serde_json::Value = serde_json::from_str(log.trim_end()).expect("one line");
+        assert_eq!(line["played"], "e4");
+        assert_eq!(line["ply"], 0);
+        assert_eq!(line["stale"], false);
+    }
+
     #[test]
     fn a_human_vs_human_session_plays_e4_and_quits_after_confirmation() {
         let mut app = new_app();
diff --git a/src/tui/panels.rs b/src/tui/panels.rs
index 35c9d2aaf9e998a4a78fa9bee80ebde5e43729ec..1de0ac94f362d3d55c43df3da0438fd25095a085 100644
--- a/src/tui/panels.rs
+++ b/src/tui/panels.rs
@@ -1359,12 +1359,14 @@ mod tests {
     use super::*;
     use crate::core::{START_FEN, Square};
     use crate::tui::board::{image_area, square_at, square_rect};
+    use crate::tui::debug::DebugLog;
     use crate::tui::event::AppEvent;
     use crate::tui::glyphs::{ImageSupport, initial_glyphs};
     use crate::tui::graphics::picker_for;
-    use crate::tui::test_support::engine::{FakeEngine, JEV_STATUS};
+    use crate::tui::test_support::engine::{FakeEngine, JEV_STATUS, traced_jev_move};
     use crate::tui::test_support::harness::Harness;
     use crate::tui::test_support::{PROMOTION_FEN, game_from, sq};
+    use crate::tui::worker::EngineOutcome;
     use crate::tui::worker::LOCAL_SEARCH_STATUS;
 
     /// An app playing against Jev (with a key), in a `width`×`height` terminal.
@@ -2472,6 +2474,83 @@ mod tests {
         insta::assert_snapshot!("local_vs_local_80x24", h.terminal.backend());
     }
 
+    /// Human vs Jev (the person plays White) at `width`×`height` in debug mode: Jev's
+    /// answer to 1. e4 came after an undo (stale), then its answer to 1. e4 played again.
+    fn two_exchanges(width: u16, height: u16) -> Harness {
+        let mut h = Harness::build(FakeEngine::jev(), (width, height), Vec::new(), |app| {
+            app.with_debug(DebugLog::open(None))
+        });
+        h.char('2');
+        h.moves(&["e4"]);
+        let first = h.request.take().expect("Jev asked");
+        h.char('u');
+        h.answer(
+            &first,
+            EngineOutcome::Move(traced_jev_move(first.game.position(), "e7e5")),
+        );
+        h.moves(&["e4"]);
+        h.reply_traced("c7c5");
+        assert_eq!(h.app.exchanges().map(|history| history.len()), Some(2));
+        h
+    }
+
+    #[test]
+    fn snapshot_exchange_view() {
+        let mut h = two_exchanges(80, 24);
+        h.char('d');
+        insta::assert_snapshot!("exchange_80x24", h.terminal.backend());
+    }
+
+    #[test]
+    fn snapshot_exchange_view_large_on_a_stale_reply() {
+        let mut h = two_exchanges(120, 40);
+        h.char('d');
+        h.press(KeyCode::Left);
+        h.press(KeyCode::PageDown);
+        insta::assert_snapshot!("exchange_120x40_stale", h.terminal.backend());
+    }
+
+    #[test]
+    fn the_exchange_header_breaks_between_items() {
+        let mut h = two_exchanges(60, 20);
+        h.char('d');
+        h.press(KeyCode::Left);
+        let rows: Vec<String> = h.screen().lines().map(str::to_string).collect();
+        assert_eq!(
+            rows[1].trim_matches(|c: char| c == '│' || c.is_whitespace()),
+            "exchange 1 of 2 · move 1 · e5 · Jev · HTTP 200"
+        );
+        assert_eq!(
+            rows[2].trim_matches(|c: char| c == '│' || c.is_whitespace()),
+            "2 attempts · 1234 ms · stale — not played"
+        );
+        assert_eq!(rows[3].trim_matches('│').trim(), "", "a blank row");
+        assert!(rows[4].contains("│ REQUEST"), "{}", rows[4]);
+        assert!(rows[0].contains(" 1-15 of "), "{}", rows[0]);
+        let bottom = &rows[19];
+        assert!(bottom.contains("Esc"), "{bottom}");
+    }
+
+    #[test]
+    fn the_exchange_view_draws_only_the_rows_it_shows() {
+        let h = two_exchanges(80, 24);
+        let record = h
+            .app
+            .exchanges()
+            .and_then(|history| history.last())
+            .unwrap();
+        let exchange = &record.exchange;
+        let all = body_rows(exchange, 20, 0, usize::MAX);
+        let total: usize = exchange.body().iter().map(|line| line.rows(20)).sum();
+        assert_eq!(all.len(), total);
+        assert!(all.iter().all(|row| row.width() <= 20));
+        for (scroll, page) in [(0, 5), (7, 10), (total - 3, 10), (total, 4)] {
+            let rows = body_rows(exchange, 20, scroll, page);
+            let expected: Vec<Line> = all.iter().skip(scroll).take(page).cloned().collect();
+            assert_eq!(rows, expected, "scroll {scroll}, page {page}");
+        }
+    }
+
     #[test]
     fn snapshot_too_small() {
         let h = jev(50, 12);
diff --git a/src/tui/snapshots/chess__tui__panels__tests__exchange_120x40_stale.snap b/src/tui/snapshots/chess__tui__panels__tests__exchange_120x40_stale.snap
new file mode 100644
index 0000000000000000000000000000000000000000..9c8aa6b9247e4ef471bc8840236e077f86894c52
--- /dev/null
+++ b/src/tui/snapshots/chess__tui__panels__tests__exchange_120x40_stale.snap
@@ -0,0 +1,44 @@
+---
+source: src/tui/panels.rs
+expression: h.terminal.backend()
+---
+"┌ Jev exchange ─────────────────────────────────────────────────────────────────────────────────────────── 15-50 of 50 ┐"
+"│ exchange 1 of 2 · move 1 · e5 · Jev · HTTP 200 · 2 attempts · 1234 ms · stale — not played                           │"
+"│                                                                                                                      │"
+"│         "e4": {                                                                                                      │"
+"│           "assessment": "good",                                                                                      │"
+"│           "effect": "opens the bishop's diagonal"                                                                    │"
+"│         }                                                                                                            │"
+"│       },                                                                                                             │"
+"│       "type": "choice"                                                                                               │"
+"│     }                                                                                                                │"
+"│   },                                                                                                                 │"
+"│   "state": {                                                                                                         │"
+"│     "note": "a quiet opening position",                                                                              │"
+"│     "side_to_move": "white"                                                                                          │"
+"│   }                                                                                                                  │"
+"│ }                                                                                                                    │"
+"│                                                                                                                      │"
+"│ RESPONSE 1 · HTTP 503 · 120 ms                                                                                       │"
+"│ error: HTTP 503: busy \u{1b}[2J now                                                                                  │"
+"│ busy \u{1b}[2J now                                                                                                   │"
+"│ \ttry again                                                                                                          │"
+"│                                                                                                                      │"
+"│ RESPONSE 2 · HTTP 200 · 850 ms                                                                                       │"
+"│ {                                                                                                                    │"
+"│   "answers": {                                                                                                       │"
+"│     "move": {                                                                                                        │"
+"│       "choice": "e4",                                                                                                │"
+"│       "confidence": 0.81,                                                                                            │"
+"│       "probabilities": {                                                                                             │"
+"│         "d4": 0.3,                                                                                                   │"
+"│         "e4": 0.62                                                                                                   │"
+"│       }                                                                                                              │"
+"│     }                                                                                                                │"
+"│   },                                                                                                                 │"
+"│   "model": "jev-test",                                                                                               │"
+"│   "usage": {                                                                                                         │"
+"│     "input_tokens": 512                                                                                              │"
+"│   }                                                                                                                  │"
+"│ }                                                                                                                    │"
+"└ ↑↓ PgUp PgDn Home End · ←→ older/newer · Esc closes ─────────────────────────────────────────────────────────────────┘"
diff --git a/src/tui/snapshots/chess__tui__panels__tests__exchange_80x24.snap b/src/tui/snapshots/chess__tui__panels__tests__exchange_80x24.snap
new file mode 100644
index 0000000000000000000000000000000000000000..1398d94cdf28a1c702e5aab1ca5d4f95da4c329c
--- /dev/null
+++ b/src/tui/snapshots/chess__tui__panels__tests__exchange_80x24.snap
@@ -0,0 +1,28 @@
+---
+source: src/tui/panels.rs
+expression: h.terminal.backend()
+---
+"┌ Jev exchange ──────────────────────────────────────────────────── 1-20 of 50 ┐"
+"│ exchange 2 of 2 · move 1 · c5 · Jev · HTTP 200 · 2 attempts · 1234 ms        │"
+"│                                                                              │"
+"│ REQUEST                                                                      │"
+"│ POST https://api.typesafe.ai/v1/systemone                                    │"
+"│ Authorization: Bearer <redacted>                                             │"
+"│ Content-Type: application/json                                               │"
+"│                                                                              │"
+"│ {                                                                            │"
+"│   "model": "jev-test",                                                       │"
+"│   "questions": {                                                             │"
+"│     "move": {                                                                │"
+"│       "criteria": {                                                          │"
+"│         "d4": {                                                              │"
+"│           "assessment": "good",                                              │"
+"│           "effect": "takes the centre"                                       │"
+"│         },                                                                   │"
+"│         "e4": {                                                              │"
+"│           "assessment": "good",                                              │"
+"│           "effect": "opens the bishop's diagonal"                            │"
+"│         }                                                                    │"
+"│       },                                                                     │"
+"│       "type": "choice"                                                       │"
+"└ ↑↓ PgUp PgDn Home End · ←→ older/newer · Esc closes ─────────────────────────┘"
diff --git a/src/tui/snapshots/chess__tui__panels__tests__help_80x24.snap b/src/tui/snapshots/chess__tui__panels__tests__help_80x24.snap
index 52cc1a24bade78865304ee3eac873e85810ffbdb..4d2f6f964571769723bd54ae591ace4578d8b8bb 100644
--- a/src/tui/snapshots/chess__tui__panels__tests__help_80x24.snap
+++ b/src/tui/snapshots/chess__tui__panels__tests__help_80x24.snap
@@ -16,12 +16,12 @@ expression: h.terminal.backend()
 "│         │             :fen <FEN>  :savefen <path>  :savepgn <path> │         │"
 "│ 3       │ u  f  n   undo, flip the board, new game                 │         │"
 "│         │ g  m  ?   glyph set, menu, this help                     │         │"
-"│ 2  ♟︎    │ Ctrl+S    save the game as PGN                           │         │"
-"│         │ q         quit (Ctrl+C works everywhere)                 │         │"
-"│ 1  ♜    │ Space     pause watching, or retry a failed engine       │         │"
-"│         │ +  -      watching: slower, faster                       │         │"
-"│    a    └───────────────────────────────────── Esc or click closes ┘         │"
-"│                                           ││                                 │"
+"│ 2  ♟︎    │ d         exchange view (start with --debug)             │         │"
+"│         │ Ctrl+S    save the game as PGN                           │         │"
+"│ 1  ♜    │ q         quit (Ctrl+C works everywhere)                 │         │"
+"│         │ Space     pause watching, or retry a failed engine       │         │"
+"│    a    │ +  -      watching: slower, faster                       │         │"
+"│         └───────────────────────────────────── Esc or click closes ┘         │"
 "└───────────────────────────────────────────┘├ Captured ───────────────────────┤"
 "┌ Command ──────────────────────────────────┐│ White                           │"
 "│ / move  : command  ? help                 ││ Black                           │"
diff --git a/src/tui/test_support/engine.rs b/src/tui/test_support/engine.rs
index 56f680c3787f1c37052a06a929f9069c0d5c52b7..0ede02a5a3f1e120b9fce369998e637a176cb0d4 100644
--- a/src/tui/test_support/engine.rs
+++ b/src/tui/test_support/engine.rs
@@ -10,9 +10,11 @@ use std::time::Duration;
 
 use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseEventKind};
 
+use serde_json::json;
+
 use super::{char_events, chord_event, key_event, mouse_event, paste_event};
 use crate::core::{Game, Move, Position as ChessPosition};
-use crate::engine::{ComputerMove, MoveSource};
+use crate::engine::{ComputerMove, JEV_ENDPOINT, JevAttempt, JevExchange, MoveSource};
 use crate::tui::event::AppEvent;
 use crate::tui::worker::{Engine, LOCAL_SEARCH_STATUS};
 
@@ -75,6 +77,68 @@ pub(crate) fn jev_move(pos: &ChessPosition, uci: &str) -> ComputerMove {
     }
 }
 
+/// An API key no recorded exchange, screen or log line may ever contain.
+pub(crate) const SENTINEL_KEY: &str = "sk-sentinel-7f3a9c";
+
+/// A Jev exchange as the engine records it, key redacted: a request body, a 503 answer
+/// whose text holds control characters, then a JSON answer.
+pub(crate) fn jev_exchange() -> JevExchange {
+    let answer = json!({
+        "model": "jev-test",
+        "answers": {
+            "move": {
+                "choice": "e4",
+                "probabilities": { "e4": 0.62, "d4": 0.3 },
+                "confidence": 0.81,
+            }
+        },
+        "usage": { "input_tokens": 512 },
+    });
+    JevExchange {
+        method: "POST".to_string(),
+        url: JEV_ENDPOINT.to_string(),
+        headers: vec![
+            ("Authorization".to_string(), "Bearer <redacted>".to_string()),
+            ("Content-Type".to_string(), "application/json".to_string()),
+        ],
+        body: json!({
+            "model": "jev-test",
+            "state": { "side_to_move": "white", "note": "a quiet opening position" },
+            "questions": {
+                "move": {
+                    "type": "choice",
+                    "criteria": {
+                        "d4": { "assessment": "good", "effect": "takes the centre" },
+                        "e4": { "assessment": "good", "effect": "opens the bishop's diagonal" },
+                    },
+                }
+            },
+        }),
+        attempts: vec![
+            JevAttempt {
+                status: Some(503),
+                response: Some("busy \u{1b}[2J now\n\ttry again".to_string()),
+                error: Some("HTTP 503: busy \u{1b}[2J now".to_string()),
+                elapsed: Duration::from_millis(120),
+            },
+            JevAttempt {
+                status: Some(200),
+                response: Some(answer.to_string()),
+                error: None,
+                elapsed: Duration::from_millis(850),
+            },
+        ],
+    }
+}
+
+/// [`jev_move`] with [`jev_exchange`] recorded, as a traced Jev move arrives.
+pub(crate) fn traced_jev_move(pos: &ChessPosition, uci: &str) -> ComputerMove {
+    ComputerMove {
+        exchange: Some(Box::new(jev_exchange())),
+        ..jev_move(pos, uci)
+    }
+}
+
 /// What a [`FakeEngine`] does with one request.
 pub(crate) enum Turn {
     /// Plays this UCI move, reported as [`jev_move`] reports it.
@@ -89,6 +153,8 @@ pub(crate) enum Turn {
     PanicOther,
     /// Sleeps this long, then panics with a `&str` payload.
     SlowPanic(Duration),
+    /// Plays this UCI move with [`jev_exchange`] recorded ([`traced_jev_move`]).
+    Traced(&'static str),
 }
 
 /// A scripted engine that never touches the network.
@@ -158,6 +224,7 @@ impl Engine for FakeEngine {
         let turn = self.script.lock().expect("script lock").pop_front();
         match turn {
             Some(Turn::Play(uci)) => Some(jev_move(game.position(), uci)),
+            Some(Turn::Traced(uci)) => Some(traced_jev_move(game.position(), uci)),
             Some(Turn::GameOver) => None,
             Some(Turn::Panic(message)) => panic::panic_any(message),
             Some(Turn::PanicString(message)) => panic::panic_any(message),
diff --git a/src/tui/test_support/harness.rs b/src/tui/test_support/harness.rs
index d00da9825588aa867ac006539ce22be0a643233c..d13daa9584663f8384b9f6561d1364ebea22a5ad 100644
--- a/src/tui/test_support/harness.rs
+++ b/src/tui/test_support/harness.rs
@@ -11,7 +11,7 @@ use ratatui::crossterm::event::{
     Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind,
 };
 
-use super::engine::{FakeEngine, chord, jev_move, key, mouse};
+use super::engine::{FakeEngine, chord, jev_exchange, jev_move, key, mouse};
 use super::{TEST_DATE, buffer_text, uci_moves};
 use crate::core::{Color as Side, Square};
 use crate::engine::ComputerMove;
@@ -231,11 +231,7 @@ impl Harness {
             Some(computer) => EngineOutcome::Move(computer),
             None => EngineOutcome::GameOver,
         };
-        EngineReply {
-            generation: request.generation,
-            hash: request.hash,
-            outcome,
-        }
+        EngineReply::new(request, outcome)
     }
 
     /// Answers `request` the way the fake engine would.
@@ -244,17 +240,14 @@ impl Harness {
         self.send(AppEvent::Engine(reply))
     }
 
-    /// Answers `request` with `outcome`.
+    /// Answers `request` with `outcome`, as the worker would (a traced move's exchange
+    /// travels beside it, see [`EngineReply::new`]).
     pub(crate) fn answer(
         &mut self,
         request: &EngineRequest,
         outcome: EngineOutcome,
     ) -> Vec<Action> {
-        self.send(AppEvent::Engine(EngineReply {
-            generation: request.generation,
-            hash: request.hash,
-            outcome,
-        }))
+        self.send(AppEvent::Engine(EngineReply::new(request, outcome)))
     }
 
     /// Answers the latest engine request with Jev playing `uci`.
@@ -262,6 +255,14 @@ impl Harness {
         self.reply_with(uci, |_| {})
     }
 
+    /// Answers the latest engine request with Jev playing `uci`, its exchange with Jev
+    /// recorded ([`jev_exchange`]), as in debug mode.
+    pub(crate) fn reply_traced(&mut self, uci: &str) -> Vec<Action> {
+        self.reply_with(uci, |computer| {
+            computer.exchange = Some(Box::new(jev_exchange()));
+        })
+    }
+
     /// Answers the latest engine request with Jev playing `uci`, after `adjust` changes how
     /// the move was chosen.
     pub(crate) fn reply_with(
diff --git a/src/tui/worker.rs b/src/tui/worker.rs
index 46cd83be0ed33d8f770a893a9e78817757d96ad2..a9a179db59f77d3c5cabb4f42dd68a5ff1208166 100644
--- a/src/tui/worker.rs
+++ b/src/tui/worker.rs
@@ -244,7 +244,10 @@ mod tests {
     use std::time::Duration;
 
     use crate::engine::JevClient;
-    use crate::tui::test_support::engine::{FakeEngine, REPLY_TIMEOUT, Turn, jev_move};
+    use crate::tui::test_support::engine::{
+        FakeEngine, REPLY_TIMEOUT, Turn, jev_exchange, jev_move,
+    };
+    use crate::tui::test_support::game_from;
 
     /// Fool's mate: White is checkmated, so nobody has a move.
     const MATED: &str = "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3";
@@ -303,6 +306,44 @@ mod tests {
         assert_eq!(engine.threads(), vec![Some("engine".to_string())]);
     }
 
+    #[test]
+    fn a_traced_move_sends_its_exchange_beside_it() {
+        let game = game_from(crate::core::START_FEN, &["e2e4"]);
+        let reply = round_trip(
+            fake(Turn::Traced("e7e5")),
+            EngineRequest::new(2, game.clone()),
+        );
+        assert_eq!(
+            reply.outcome,
+            EngineOutcome::Move(jev_move(game.position(), "e7e5")),
+            "the move no longer holds the exchange"
+        );
+        let exchange = reply.exchange.expect("the exchange travels in the reply");
+        assert_eq!(exchange.http, jev_exchange());
+        assert_eq!((exchange.ply, exchange.fullmove), (1, 1));
+        assert_eq!(exchange.san, "e5");
+        assert_eq!(exchange.source, "Jev");
+        assert!(!exchange.body().is_empty(), "rendered on the engine thread");
+    }
+
+    #[test]
+    fn untraced_replies_carry_no_exchange() {
+        let request = EngineRequest::new(1, Game::new());
+        assert_eq!(
+            round_trip(fake(Turn::Play("e2e4")), request.clone()).exchange,
+            None
+        );
+        assert_eq!(
+            round_trip(fake(Turn::GameOver), request.clone()).exchange,
+            None
+        );
+        let failed = EngineReply::new(&request, EngineOutcome::Failed("no thread".into()));
+        assert_eq!(
+            (failed.generation, failed.hash, failed.exchange),
+            (1, request.hash, None)
+        );
+    }
+
     #[test]
     fn no_move_means_game_over() {
         let reply = round_trip(fake(Turn::GameOver), EngineRequest::new(5, Game::new()));
@@ -428,6 +469,7 @@ mod tests {
             generation: 4,
             hash: 0xABCD,
             outcome: EngineOutcome::GameOver,
+            exchange: None,
         };
         assert!(is_current(&reply, 4, 0xABCD));
         assert!(!is_current(&reply, 5, 0xABCD), "generation moved on");
````

- [ ] **Step: Run the tests to verify they fail**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: FAIL. The test build does not compile: the tests use names the implementation patch adds, for example "cannot find struct, variant or union type ExchangeView in this scope"; "cannot find struct, variant or union type LogFailure in this scope"; "cannot find function body_rows in this scope"; "cannot find function enabled in this scope". Any other failure (a patch that does not apply, a test that fails at run time) is not the expected RED: stop and report.

- [ ] **Step: Apply the implementation patch**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/engine/jev.rs b/src/engine/jev.rs
index 253de9eaae7fbac80ed73a8cd3ac228713ab9d58..0a650f261f26653c913da2931b44f85ab069bd4a 100644
--- a/src/engine/jev.rs
+++ b/src/engine/jev.rs
@@ -157,7 +157,9 @@ pub struct JevAttempt {
     /// The HTTP status, or `None` when no response arrived.
     pub status: Option<u16>,
     /// The response body as text (read under the 1 MiB cap), or `None` when no
-    /// response arrived or its body could not be read.
+    /// response arrived or its body could not be read. A JSON body in which the key
+    /// had to be redacted after decoding is re-encoded, so it is not byte for byte
+    /// what the server sent.
     pub response: Option<String>,
     /// Why the attempt failed, as the `JevError` message; `None` for an answer.
     pub error: Option<String>,
@@ -273,6 +275,23 @@ fn redact(text: &str, secret: &str) -> String {
     }
 }
 
+/// A response body with the key redacted, both in the raw text and, when the body is
+/// JSON, in the text a parser decodes from it: a server can echo the key with `\u`
+/// escapes that the raw text does not match. When decoding shows the key, the body
+/// is re-encoded from the redacted JSON, so no later parse can bring it back.
+fn redact_body(text: &str, secret: &str) -> String {
+    let text = redact(text, secret);
+    let Ok(json) = serde_json::from_str::<Value>(&text) else {
+        return text;
+    };
+    let redacted = redact_value(&json, secret);
+    if redacted == json {
+        text
+    } else {
+        redacted.to_string()
+    }
+}
+
 /// `value` with [`redact`] applied to every string and object key in it.
 fn redact_value(value: &Value, secret: &str) -> Value {
     match value {
@@ -425,8 +444,8 @@ impl JevClient {
         }
     }
 
-    /// One HTTP attempt. The response body is redacted before it is parsed or cut
-    /// into an error snippet, so no error can carry part of the key.
+    /// One HTTP attempt. The response body is redacted ([`redact_body`]) before it is
+    /// parsed or cut into an error snippet, so no error can carry part of the key.
     fn post_once(&self, body: &Value) -> Attempt {
         let sent = self
             .agent
@@ -458,7 +477,7 @@ impl JevClient {
             .with_config()
             .limit(MAX_BODY_BYTES)
             .read_to_string()
-            .map(|text| redact(&text, &self.api_key));
+            .map(|text| redact_body(&text, &self.api_key));
         let (result, response) = match text {
             Ok(text) if status == 200 => (parse_answer(&text), Some(text)),
             Ok(text) => (Err(http_error(status, &text, retry_after)), Some(text)),
@@ -500,7 +519,7 @@ impl MoveChooser for JevClient {
 }
 
 #[cfg(test)]
-mod tests {
+pub(crate) mod tests {
     use super::*;
     use std::collections::BTreeSet;
     use std::io::{BufRead, BufReader, Read, Write};
@@ -1007,6 +1026,81 @@ mod tests {
         assert!(requests.lock().unwrap()[0].contains(SENTINEL_KEY));
     }
 
+    /// `text` with every character written as a JSON `\u` escape: text a JSON parser
+    /// reads back as `text`, though it does not contain it.
+    fn json_escaped(text: &str) -> String {
+        text.chars()
+            .map(|c| format!("\\u{:04x}", u32::from(c)))
+            .collect()
+    }
+
+    /// The exchange a traced client sending `api_key` records against a local server
+    /// that echoes the key in a 503 body, then JSON-escaped in another, then answers.
+    /// The TUI's key tests render and log it, so they check an exchange as the engine
+    /// really records it.
+    pub(crate) fn recorded_exchange(api_key: &str) -> JevExchange {
+        let echo = format!(r#"{{"error":"overloaded, key {api_key} is queued"}}"#);
+        let escaped = format!(
+            r#"{{"error":"overloaded, key {} is queued"}}"#,
+            json_escaped(api_key)
+        );
+        let (client, requests) = serve_with_key(
+            vec![
+                response("503 Service Unavailable", "application/json", &echo),
+                response("503 Service Unavailable", "application/json", &escaped),
+                response("200 OK", "application/json", ANSWER),
+            ],
+            api_key,
+        );
+        let mut trace = None;
+        client
+            .choose_traced(&request(), &mut trace)
+            .expect("the server answers the retry");
+        assert!(requests.lock().unwrap()[0].contains(api_key));
+        trace.expect("the client records the exchange")
+    }
+
+    #[test]
+    fn traced_exchange_redacts_a_key_the_server_escapes() {
+        // `\u` escapes do not match the key in the raw text, but every JSON parser, the
+        // exchange view's and the debug log's included, decodes them back into it.
+        let escaped = json_escaped(SENTINEL_KEY);
+        let (client, _requests) = serve_with_key(
+            vec![
+                response(
+                    "401 Unauthorized",
+                    "application/json",
+                    &format!(r#"{{"error":"invalid api key {escaped}"}}"#),
+                ),
+                response(
+                    "200 OK",
+                    "application/json",
+                    &ANSWER.replace("jev-1.13.0", &escaped),
+                ),
+            ],
+            SENTINEL_KEY,
+        );
+
+        let mut trace = None;
+        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
+        assert_eq!(
+            trace.unwrap().attempts[0].response.as_deref(),
+            Some(r#"{"error":"invalid api key <redacted>"}"#)
+        );
+        assert_eq!(
+            error.to_string(),
+            r#"HTTP 401: {"error":"invalid api key <redacted>"}"#
+        );
+
+        let mut trace = None;
+        let answer = client.choose_traced(&request(), &mut trace).unwrap();
+        assert_eq!(answer.model, "<redacted>");
+        let text = trace.unwrap().attempts[0].response.clone().unwrap();
+        let decoded: Value = serde_json::from_str(&text).unwrap();
+        assert_eq!(decoded["model"], "<redacted>");
+        assert_eq!(decoded["answers"]["move"]["choice"], "O-O");
+    }
+
     #[test]
     fn untraced_errors_redact_an_echoed_key_too() {
         let echo = format!("no such key: {SENTINEL_KEY}");
diff --git a/src/engine/mod.rs b/src/engine/mod.rs
index 53a454d9da380df66b0caa1d87eec09c78bbda68..281ac03097fe81d328e22be1ce4369f37fe33ab9 100644
--- a/src/engine/mod.rs
+++ b/src/engine/mod.rs
@@ -12,6 +12,9 @@ mod see;
 pub use annotate::{Annotation, Bucket};
 pub use config::EngineConfig;
 pub use describe::JevState;
+/// A really recorded, redacted exchange for the TUI's key tests.
+#[cfg(test)]
+pub(crate) use jev::tests::recorded_exchange;
 pub use jev::{
     ChoiceAnswer, ChoiceOption, ChoiceRequest, JEV_ENDPOINT, JevAttempt, JevClient, JevError,
     JevExchange, MoveChooser,
diff --git a/src/tui/app.rs b/src/tui/app.rs
index f81ecb027e7110b0a5a4b03a25bc0d12e2d6b4ea..be8c213b16dd1fdfa2bb4a40e4ccb8cf3531ae7e 100644
--- a/src/tui/app.rs
+++ b/src/tui/app.rs
@@ -39,11 +39,17 @@
 //!
 //! Actions that throw away a game in progress (quit, new game, menu, resign) ask first, and
 //! their confirmation defaults to No, so a stray Enter never confirms one.
+//!
+//! In debug mode ([`App::with_debug`]) every Jev exchange that comes back with an engine
+//! reply is kept in a [`History`], stale replies included, and queued for the debug log.
+//! `d` on the board opens the full-screen exchange view ([`ExchangeView`]), which takes the
+//! keys (after the dialogs and Ctrl+C) until Esc closes it; engine replies are still applied
+//! underneath. The first debug log failure is shown once as a status message.
 
 use std::fmt;
 use std::path::{Path, PathBuf};
 use std::sync::Arc;
-use std::time::{Duration, Instant};
+use std::time::{Duration, Instant, SystemTime};
 
 use ratatui::Frame;
 use ratatui::crossterm::event::{
@@ -53,6 +59,7 @@ use ratatui::layout::{Position as CellPosition, Rect};
 use ratatui_image::picker::Picker;
 
 use super::board::{BoardGeometry, CellSize, Highlights, PieceImages, square_at};
+use super::debug::{DebugLog, DebugSession, Exchange, ExchangeView, History};
 use super::event::AppEvent;
 use super::files::{SaveError, pgn_export, resolve_path, tilde_path, today, write_file};
 use super::glyphs::{self, GlyphSet, Palette};
@@ -83,6 +90,8 @@ pub const MAX_IN_FLIGHT: usize = 2;
 pub const ENGINE_FAILED: &str = "the engine failed; space retries unless typing";
 /// Status line while a new request waits for [`MAX_IN_FLIGHT`] to allow it.
 pub const WAITING_FOR_ENGINE: &str = "waiting for an old request";
+/// Status message for `d` when debug mode is off.
+pub const DEBUG_OFF: &str = "debug mode is off (start with --debug)";
 
 /// The computer player's name when it uses Jev.
 pub const JEV_NAME: &str = "Jev";
@@ -133,6 +142,8 @@ pub const GAME_OVER_BUTTONS: [Button; 3] = [Button::NewGame, Button::SavePgn, Bu
 const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];
 /// Milliseconds each spinner frame is shown.
 const SPINNER_FRAME_MS: u128 = 100;
+/// Rows one notch of the mouse wheel scrolls the exchange view.
+const WHEEL_ROWS: usize = 3;
 
 const GAME_IS_OVER: &str = "the game is over (u: undo, n: new game)";
 const NOTHING_TO_UNDO: &str = "nothing to undo";
@@ -608,6 +619,10 @@ pub struct App {
     cell_size: CellSize,
     /// The board's piece pictures, kept between draws.
     piece_images: PieceImages,
+    /// Debug mode's exchanges and log; `None` when debug mode is off.
+    debug: Option<DebugSession>,
+    /// The exchange view, while it is open (only in debug mode).
+    exchange_view: Option<ExchangeView>,
     pick_side: fn() -> Side,
     today: fn() -> String,
     home: Option<PathBuf>,
@@ -640,8 +655,9 @@ pub struct App {
     engine_failed: bool,
     /// Jev vs Jev is paused: no request is sent and no move is played.
     paused: bool,
-    /// An answer that arrived while paused, applied when play resumes.
-    held: Option<EngineOutcome>,
+    /// An answer that arrived while paused, applied when play resumes, with its Jev
+    /// exchange.
+    held: Option<(EngineOutcome, Option<Box<Exchange>>)>,
     step_delay: Duration,
     /// When the last Jev vs Jev move was applied; the next request is due one step
     /// delay later. `None` means due now.
@@ -684,6 +700,8 @@ impl App {
             picker: None,
             cell_size: CellSize::DEFAULT,
             piece_images: PieceImages::new(),
+            debug: None,
+            exchange_view: None,
             pick_side: random_side,
             today,
             home: std::env::var_os("HOME").map(PathBuf::from),
@@ -739,6 +757,15 @@ impl App {
         self.cell_size = cell_size;
     }
 
+    /// Turns debug mode on (spec 9.4): Jev exchanges that arrive with engine replies are
+    /// kept for the exchange view and written to `log`. The engine must record them
+    /// (`EngineConfig::trace`); without debug mode they are dropped.
+    #[must_use]
+    pub fn with_debug(mut self, log: DebugLog) -> App {
+        self.debug = Some(DebugSession::new(log));
+        self
+    }
+
     /// Replaces the coin flip used by "Human vs Jev: random side" (tests pass a fixed side).
     #[must_use]
     pub fn with_side_picker(mut self, pick: fn() -> Side) -> App {
@@ -919,6 +946,29 @@ impl App {
         &self.piece_images
     }
 
+    /// True in debug mode (see [`with_debug`](Self::with_debug)).
+    pub fn debug_mode(&self) -> bool {
+        self.debug.is_some()
+    }
+
+    /// The Jev exchanges kept in debug mode, oldest first; `None` when it is off.
+    pub fn exchanges(&self) -> Option<&History> {
+        self.debug.as_ref().map(DebugSession::history)
+    }
+
+    /// The exchange view, while it is open.
+    pub fn exchange_view(&self) -> Option<ExchangeView> {
+        self.exchange_view
+    }
+
+    /// Stops the debug log, waiting up to `grace` for it to write the exchanges still
+    /// queued (see [`DebugLog::close`]). Call it once the app is done.
+    pub fn close_debug_log(&mut self, grace: Duration) {
+        if let Some(debug) = &mut self.debug {
+            debug.close_log(grace);
+        }
+    }
+
     /// The command box text.
     pub fn command_text(&self) -> &str {
         self.command.text()
@@ -1089,6 +1139,7 @@ impl App {
             AppEvent::Term(_) | AppEvent::Tick => {}
             AppEvent::Engine(reply) => self.on_engine(reply, now),
         }
+        self.report_log_failure();
         self.release_held(now);
         self.engine_request(now)
             .map(Action::RequestEngine)
@@ -1129,6 +1180,10 @@ impl App {
             self.dialog_key(key);
             return;
         }
+        if self.exchange_view.is_some() {
+            self.exchange_key(key);
+            return;
+        }
         if ctrl_char(&key) == Some('s') && self.screen != Screen::Menu {
             self.open_input(InputPurpose::Save(SaveKind::Pgn));
             return;
@@ -1253,6 +1308,7 @@ impl App {
             'f' => self.flipped = !self.flipped,
             'n' => self.ask_new_game(),
             'g' => self.cycle_glyphs(),
+            'd' => self.open_exchanges(),
             'm' => self.ask_menu(),
             '?' => self.dialogs.push(Dialog::Help),
             'q' => self.request_quit(),
@@ -1274,6 +1330,35 @@ impl App {
         }
     }
 
+    /// Opens the exchange view on the newest exchange, or says debug mode is off.
+    fn open_exchanges(&mut self) {
+        match &self.debug {
+            Some(debug) => self.exchange_view = Some(ExchangeView::open(debug.history())),
+            None => self.show(Message::info(DEBUG_OFF)),
+        }
+    }
+
+    /// Keys in the exchange view: ↑/↓, PgUp/PgDn and Home/End scroll, ←/→ show the
+    /// older or newer exchange, Esc closes it.
+    fn exchange_key(&mut self, key: KeyEvent) {
+        let (Some(view), Some(debug)) = (&mut self.exchange_view, &self.debug) else {
+            return;
+        };
+        let history = debug.history();
+        match key.code {
+            KeyCode::Up => view.up(1),
+            KeyCode::Down => view.down(1),
+            KeyCode::PageUp => view.page_up(),
+            KeyCode::PageDown => view.page_down(),
+            KeyCode::Home => view.home(),
+            KeyCode::End => view.end(),
+            KeyCode::Left => view.older(history),
+            KeyCode::Right => view.newer(history),
+            KeyCode::Esc => self.exchange_view = None,
+            _ => {}
+        }
+    }
+
     fn command_key(&mut self, key: KeyEvent) {
         // No command or move starts with a space, so in an empty box space does what it
         // does on the board: retry a failed engine, or pause Jev vs Jev.
@@ -1365,6 +1450,18 @@ impl App {
 
     fn on_mouse(&mut self, mouse: MouseEvent) {
         let (column, row) = (mouse.column, mouse.row);
+        if self.dialogs.is_empty()
+            && let Some(view) = &mut self.exchange_view
+        {
+            // Nothing under the view can be clicked; the wheel scrolls it.
+            match mouse.kind {
+                MouseEventKind::ScrollUp => view.up(WHEEL_ROWS),
+                MouseEventKind::ScrollDown => view.down(WHEEL_ROWS),
+                _ => {}
+            }
+            self.drag = None;
+            return;
+        }
         match mouse.kind {
             MouseEventKind::Down(MouseButton::Left) => self.mouse_down(column, row),
             MouseEventKind::Up(MouseButton::Left) => {
@@ -1445,6 +1542,9 @@ impl App {
             }
             return;
         }
+        if self.exchange_view.is_some() {
+            return;
+        }
         let to_box = match self.screen {
             Screen::Playing => true,
             Screen::GameOver => self.command_focused,
@@ -1468,6 +1568,7 @@ impl App {
                 "discarding engine reply for generation {}: no request pending",
                 reply.generation
             );
+            self.record_exchange(reply.exchange, true);
             return;
         }
         if !is_current(&reply, self.generation, self.game.position().hash()) {
@@ -1476,28 +1577,37 @@ impl App {
                 reply.generation,
                 self.generation
             );
+            self.record_exchange(reply.exchange, true);
             return;
         }
         self.pending = None;
         if self.paused {
             // Paused while the engine was thinking: nothing is played until space resumes.
-            self.held = Some(reply.outcome);
+            self.held = Some((reply.outcome, reply.exchange));
             return;
         }
-        self.apply_outcome(reply.outcome, now);
+        self.apply_outcome(reply.outcome, reply.exchange, now);
     }
 
-    /// Plays a move the worker produced, or records why there is none.
-    fn apply_outcome(&mut self, outcome: EngineOutcome, now: Instant) {
+    /// Plays a move the worker produced, or records why there is none. `exchange` is the
+    /// Jev exchange behind the move, kept as played or, for an illegal move, as not.
+    fn apply_outcome(
+        &mut self,
+        outcome: EngineOutcome,
+        exchange: Option<Box<Exchange>>,
+        now: Instant,
+    ) {
         match outcome {
             EngineOutcome::Move(computer) => {
                 if let Err(error) = self.game.play(computer.mv) {
+                    self.record_exchange(exchange, true);
                     self.engine_failure(&format!(
                         "engine returned an illegal move {}: {error}",
                         computer.mv
                     ));
                     return;
                 }
+                self.record_exchange(exchange, false);
                 let recovered = computer.note.as_deref() == Some(ENGINE_ERROR_NOTE);
                 self.last_computer = Some((self.game.moves().len(), computer));
                 // Notes about the turn just played (a move typed too early, an undo) are
@@ -1526,12 +1636,38 @@ impl App {
     /// Plays the answer held while paused, once play has resumed.
     fn release_held(&mut self, now: Instant) {
         if !self.paused
-            && let Some(outcome) = self.held.take()
+            && let Some((outcome, exchange)) = self.held.take()
         {
-            self.apply_outcome(outcome, now);
+            self.apply_outcome(outcome, exchange, now);
+        }
+    }
+
+    /// Keeps `exchange` in debug mode (`stale` when its move was not played) and shows it
+    /// in an open view that had nothing to show. Without debug mode it is dropped.
+    fn record_exchange(&mut self, exchange: Option<Box<Exchange>>, stale: bool) {
+        let (Some(debug), Some(exchange)) = (&mut self.debug, exchange) else {
+            return;
+        };
+        let number = debug.record(*exchange, stale, SystemTime::now());
+        if let Some(view) = &mut self.exchange_view {
+            view.follow(number);
         }
     }
 
+    /// Shows the debug log's failure, the one time it is reported.
+    fn report_log_failure(&mut self) {
+        let Some(failure) = self.debug.as_mut().and_then(DebugSession::log_failure) else {
+            return;
+        };
+        let text = format!("debug log disabled: {}", failure.reason);
+        let message = match &failure.path {
+            Some(path) => Message::error(format!("{text}: "))
+                .with_path(tilde_path(path, self.home.as_deref())),
+            None => Message::error(text),
+        };
+        self.show(message);
+    }
+
     /// True when it is the engine's turn, nothing stops it, and in Jev vs Jev the step
     /// delay since the last move has passed.
     fn engine_due(&self, now: Instant) -> bool {
@@ -1623,6 +1759,7 @@ impl App {
     fn go_to_menu(&mut self) {
         self.screen = Screen::Menu;
         self.dialogs.clear();
+        self.exchange_view = None;
         self.command.clear();
         self.command_focused = false;
         self.cursor = None;
@@ -1631,12 +1768,14 @@ impl App {
     }
 
     /// Forgets everything tied to the old position: in-flight requests (by bumping the
-    /// generation), a held answer, an engine failure, the selection, the move-list scroll,
-    /// and the Jev panel's move once it has been taken back.
+    /// generation), a held answer (its exchange kept as stale), an engine failure, the
+    /// selection, the move-list scroll, and the Jev panel's move once it has been taken back.
     fn invalidate(&mut self) {
         self.generation = self.generation.wrapping_add(1);
         self.pending = None;
-        self.held = None;
+        if let Some((_, exchange)) = self.held.take() {
+            self.record_exchange(exchange, true);
+        }
         self.engine_failed = false;
         self.selected = None;
         self.drag = None;
@@ -2205,7 +2344,8 @@ impl App {
 
     /// Draws the current state (see [`panels::draw`]) and records the [`HitMap`] used by
     /// the next mouse event. Also clamps the move-list scroll to what the list can show,
-    /// and notes whether only [`TOO_SMALL`] fitted (input is ignored until more does).
+    /// keeps the exchange view's page size and scroll limit for its keys, and notes whether
+    /// only [`TOO_SMALL`] fitted (input is ignored until more does).
     pub fn render(&mut self, frame: &mut Frame, now: Instant) {
         self.too_small = is_too_small(frame.area());
         // Lent to the draw, which reads the rest of the app and keeps new pictures in it.
@@ -2214,6 +2354,9 @@ impl App {
         self.piece_images = images;
         self.hits = drawn.hits;
         self.move_scroll = drawn.move_scroll;
+        if self.exchange_view.is_some() {
+            self.exchange_view = drawn.exchange_view;
+        }
     }
 }
 
@@ -2350,12 +2493,13 @@ mod tests {
     use super::*;
     use crate::core::{Piece, Position as ChessPosition, START_FEN};
     use crate::engine::EngineConfig;
-    use crate::engine::{MoveSource, analyse};
+    use crate::engine::{MoveSource, analyse, recorded_exchange};
     use crate::tui::board::square_rect;
     use crate::tui::graphics;
     use crate::tui::panels::HELP_LINES;
     use crate::tui::test_support::engine::{
-        FakeEngine, REPLY_TIMEOUT, SENTINEL_KEY, chord, key, mouse, paste, traced_jev_move,
+        FakeEngine, REPLY_TIMEOUT, SENTINEL_KEY, chord, jev_move, key, mouse, paste,
+        traced_jev_move,
     };
     use crate::tui::test_support::harness::{Harness, request};
     use crate::tui::test_support::{PROMOTION_FEN, TempDir, buffer_text, game_from, key_event, sq};
@@ -4827,7 +4971,9 @@ mod tests {
     fn the_api_key_never_reaches_the_screen_or_the_log() {
         let dir = TempDir::new("debug-app");
         let path = dir.join("jev.jsonl");
-        // A real player holding the key; building it is offline, and no move is asked of it.
+        // The exchange the engine records against a server that echoes the key, carried
+        // by a traced move as the worker delivers it. The player is only the app's engine;
+        // building it is offline, and no move is asked of it.
         let config = EngineConfig {
             api_key: Some(SENTINEL_KEY.to_string()),
             trace: true,
@@ -4842,7 +4988,10 @@ mod tests {
         let now = Instant::now();
         let actions = app.handle(AppEvent::Term(key_event(KeyCode::Char('3'))), now);
         let request = request(&actions);
-        let computer = traced_jev_move(request.game.position(), "e2e4");
+        let computer = ComputerMove {
+            exchange: Some(Box::new(recorded_exchange(SENTINEL_KEY))),
+            ..jev_move(request.game.position(), "e2e4")
+        };
         let _ = app.handle(
             AppEvent::Engine(EngineReply::new(&request, EngineOutcome::Move(computer))),
             now,
diff --git a/src/tui/debug.rs b/src/tui/debug.rs
index 2d7eded9d7289ca71b2439069e3b0941079e290d..943c12922240ce39f9a84599f9dadf81fdd9c9ad 100644
--- a/src/tui/debug.rs
+++ b/src/tui/debug.rs
@@ -16,6 +16,778 @@
 //! The API key never reaches any of this: the engine redacts it while recording, so
 //! the `Authorization` header reads `Bearer <redacted>`.
 
+use std::collections::VecDeque;
+use std::fs::{DirBuilder, File, OpenOptions};
+use std::io::{self, Write as _};
+use std::path::{Path, PathBuf};
+use std::sync::Arc;
+use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
+use std::thread;
+use std::time::{Duration, SystemTime, UNIX_EPOCH};
+
+use serde::{Serialize, Serializer};
+use serde_json::Value;
+
+use super::files::{civil_date, describe};
+use super::glyphs::char_width;
+use crate::core::Game;
+use crate::engine::{ComputerMove, JevExchange};
+
+/// Turns debug mode on unless empty or `0` (see [`enabled`]).
+pub const DEBUG_ENV: &str = "RCHESS_DEBUG";
+/// The debug log file, when set (see [`log_path`]).
+pub const DEBUG_LOG_ENV: &str = "RCHESS_DEBUG_LOG";
+/// Exchanges kept for the view; older ones are dropped.
+pub const HISTORY_LEN: usize = 50;
+/// What the exchange view says before the first exchange.
+pub const NO_EXCHANGES: &str = "no Jev requests yet";
+
+/// Name of the thread that writes the log.
+const LOG_THREAD: &str = "debug-log";
+/// The log's folder under the state folder.
+const LOG_DIR: &str = "rchess";
+/// The log's file name.
+const LOG_FILE: &str = "jev-debug.jsonl";
+/// Why there is no log when no path could be worked out.
+const NO_LOG_PATH: &str = "no log file (set RCHESS_DEBUG_LOG, XDG_STATE_HOME or HOME)";
+
+/// True when debug mode is on: `--debug` was given (`flag`), or `RCHESS_DEBUG` is set
+/// to anything but empty or `0` (surrounding whitespace ignored).
+///
+/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
+pub fn enabled(flag: bool, get: impl Fn(&str) -> Option<String>) -> bool {
+    flag || get(DEBUG_ENV).is_some_and(|value| !matches!(value.trim(), "" | "0"))
+}
+
+/// The debug log file: `RCHESS_DEBUG_LOG` when set, else
+/// `$XDG_STATE_HOME/rchess/jev-debug.jsonl`, else `~/.local/state/rchess/jev-debug.jsonl`
+/// (on macOS too). Empty variables count as unset, and so does a relative
+/// `XDG_STATE_HOME` (the XDG base directory rules say to ignore one). `None` when none
+/// of the three is set.
+///
+/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
+pub fn log_path(get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
+    let set = |name: &str| get(name).filter(|value| !value.is_empty());
+    if let Some(path) = set(DEBUG_LOG_ENV) {
+        return Some(PathBuf::from(path));
+    }
+    let state = set("XDG_STATE_HOME")
+        .map(PathBuf::from)
+        .filter(|dir| dir.is_absolute())
+        .or_else(|| set("HOME").map(|home| Path::new(&home).join(".local").join("state")))?;
+    Some(state.join(LOG_DIR).join(LOG_FILE))
+}
+
+/// `time` in UTC as RFC 3339 with milliseconds: `2026-09-27T14:03:05.120Z`. Times before
+/// 1970 count back from the epoch.
+pub fn rfc3339(time: SystemTime) -> String {
+    const MILLIS_PER_DAY: i128 = 86_400_000;
+    let nanos = match time.duration_since(UNIX_EPOCH) {
+        Ok(since) => i128::try_from(since.as_nanos()).unwrap_or(i128::MAX),
+        Err(before) => i128::try_from(before.duration().as_nanos()).map_or(i128::MIN, |n| -n),
+    };
+    let millis = nanos.div_euclid(1_000_000);
+    let days = i64::try_from(millis.div_euclid(MILLIS_PER_DAY)).unwrap_or(if millis < 0 {
+        i64::MIN
+    } else {
+        i64::MAX
+    });
+    let of_day = millis.rem_euclid(MILLIS_PER_DAY);
+    let (year, month, day) = civil_date(days);
+    let (hours, minutes) = (of_day / 3_600_000, of_day / 60_000 % 60);
+    let (seconds, millis) = (of_day / 1_000 % 60, of_day % 1_000);
+    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
+}
+
+// ----- exchanges -----
+
+/// One Jev exchange as debug mode shows it: the HTTP request and every attempt, as the
+/// engine recorded them (the key already redacted), and the move they were for.
+#[derive(Clone, Debug, PartialEq)]
+pub struct Exchange {
+    /// Plies played in the game when the engine was asked.
+    pub ply: usize,
+    /// The full-move number of the position the engine was asked about.
+    pub fullmove: u16,
+    /// The chosen move in SAN; played unless the reply was stale.
+    pub san: String,
+    /// How the move was chosen: the `MoveSource` label (`Jev`,
+    /// `vetoed (Jev picked Qh4)` or `local search`).
+    pub source: String,
+    /// Time for the whole engine call.
+    pub latency: Duration,
+    /// The request and its attempts.
+    pub http: JevExchange,
+    /// The view's text, rendered once.
+    body: Vec<BodyLine>,
+}
+
+impl Exchange {
+    /// The exchange `http` that chose `computer` in `game` (the game the engine was
+    /// asked about). Renders the view's body text ([`Exchange::body`]).
+    pub fn new(game: &Game, computer: &ComputerMove, http: JevExchange) -> Exchange {
+        Exchange {
+            ply: game.moves().len(),
+            fullmove: game.position().fullmove_number(),
+            san: computer.san.clone(),
+            source: computer.source.to_string(),
+            latency: computer.latency,
+            body: body_lines(&http),
+            http,
+        }
+    }
+
+    /// The view's text: `REQUEST` (method and URL, the headers, then the JSON body
+    /// pretty-printed), then one `RESPONSE` block per attempt (its status or `no
+    /// response`, the time it took, the error, and the body: pretty-printed when it is
+    /// JSON, else the text as sent). Control characters are escaped (`\u{1b}`), so
+    /// nothing a server sends can act on the terminal.
+    pub fn body(&self) -> &[BodyLine] {
+        &self.body
+    }
+
+    /// Attempts made, retries included.
+    pub fn attempts(&self) -> usize {
+        self.http.attempts.len()
+    }
+
+    /// How the last attempt ended: `HTTP <status>`, `no response`, or `not sent` when
+    /// there was no attempt.
+    pub fn status(&self) -> String {
+        match self.http.attempts.last() {
+            Some(attempt) => attempt_status(attempt.status),
+            None => "not sent".to_string(),
+        }
+    }
+}
+
+/// `HTTP <status>`, or `no response`.
+fn attempt_status(status: Option<u16>) -> String {
+    status.map_or_else(
+        || "no response".to_string(),
+        |status| format!("HTTP {status}"),
+    )
+}
+
+/// How a [`BodyLine`] is drawn.
+#[derive(Clone, Copy, Debug, PartialEq, Eq)]
+pub enum LineKind {
+    /// `REQUEST` and `RESPONSE` block headings.
+    Heading,
+    /// Why an attempt failed.
+    Error,
+    /// Everything else.
+    Text,
+}
+
+/// One line of an exchange's body text, free of control characters.
+#[derive(Clone, Debug, PartialEq, Eq)]
+pub struct BodyLine {
+    /// The text.
+    pub text: String,
+    /// How it is drawn.
+    pub kind: LineKind,
+    /// Cells the text takes on one row.
+    pub width: usize,
+    /// Every character is one cell wide, so rows are simple to count.
+    narrow: bool,
+}
+
+impl BodyLine {
+    /// `text` (which must hold no control characters) drawn as `kind`.
+    fn new(text: String, kind: LineKind) -> BodyLine {
+        let (width, narrow) = text.chars().fold((0, true), |(width, narrow), c| {
+            let w = char_width(c);
+            (width + w, narrow && w == 1)
+        });
+        BodyLine {
+            text,
+            kind,
+            width,
+            narrow,
+        }
+    }
+
+    /// Rows the line takes when wrapped at `width` cells (at least one).
+    pub fn rows(&self, width: usize) -> usize {
+        let width = width.max(1);
+        if self.narrow {
+            self.width.div_ceil(width).max(1)
+        } else {
+            self.wrapped(width).len()
+        }
+    }
+
+    /// The line cut into rows of at most `width` cells, between any two characters (a
+    /// wide character that would straddle the edge starts the next row).
+    pub fn wrapped(&self, width: usize) -> Vec<String> {
+        let width = width.max(1);
+        let mut rows = vec![String::new()];
+        let mut used = 0;
+        for c in self.text.chars() {
+            let w = char_width(c);
+            if used > 0 && used + w > width {
+                rows.push(String::new());
+                used = 0;
+            }
+            if let Some(row) = rows.last_mut() {
+                row.push(c);
+            }
+            used += w;
+        }
+        rows
+    }
+}
+
+/// The body text of `http` (see [`Exchange::body`]).
+fn body_lines(http: &JevExchange) -> Vec<BodyLine> {
+    let mut lines = Vec::new();
+    let mut push = |text: &str, kind: LineKind| push_text(&mut lines, text, kind);
+    push("REQUEST", LineKind::Heading);
+    push(&format!("{} {}", http.method, http.url), LineKind::Text);
+    for (name, value) in &http.headers {
+        push(&format!("{name}: {value}"), LineKind::Text);
+    }
+    push("", LineKind::Text);
+    push(&pretty(&http.body), LineKind::Text);
+    for (index, attempt) in http.attempts.iter().enumerate() {
+        push("", LineKind::Text);
+        push(
+            &format!(
+                "RESPONSE {} · {} · {} ms",
+                index + 1,
+                attempt_status(attempt.status),
+                attempt.elapsed.as_millis()
+            ),
+            LineKind::Heading,
+        );
+        if let Some(error) = &attempt.error {
+            push(&format!("error: {error}"), LineKind::Error);
+        }
+        match (&attempt.response, attempt.status) {
+            (Some(text), _) if text.is_empty() => push("(empty body)", LineKind::Text),
+            (Some(text), _) => match serde_json::from_str::<Value>(text) {
+                Ok(json) => push(&pretty(&json), LineKind::Text),
+                Err(_) => push(text, LineKind::Text),
+            },
+            (None, Some(_)) => push("(body not readable)", LineKind::Text),
+            (None, None) => {}
+        }
+    }
+    lines
+}
+
+/// Appends `text` to `lines`, one [`BodyLine`] per line of it, control characters escaped.
+fn push_text(lines: &mut Vec<BodyLine>, text: &str, kind: LineKind) {
+    for line in text.split('\n') {
+        let line = line.strip_suffix('\r').unwrap_or(line);
+        lines.push(BodyLine::new(escape_controls(line), kind));
+    }
+}
+
+/// `text` with every control character written as its Rust escape (`\t`, `\u{1b}`).
+fn escape_controls(text: &str) -> String {
+    let mut escaped = String::with_capacity(text.len());
+    for c in text.chars() {
+        if c.is_control() {
+            escaped.extend(c.escape_default());
+        } else {
+            escaped.push(c);
+        }
+    }
+    escaped
+}
+
+/// `json` pretty-printed with two-space indents.
+fn pretty(json: &Value) -> String {
+    serde_json::to_string_pretty(json).unwrap_or_else(|_| json.to_string())
+}
+
+// ----- history -----
+
+/// One exchange in the [`History`].
+#[derive(Clone, Debug, PartialEq)]
+pub struct Record {
+    /// 1 for the session's first exchange, counting up; never reused.
+    pub number: u64,
+    /// When the reply reached the UI.
+    pub time: SystemTime,
+    /// The reply was discarded, not played (the game had moved on, or it was an illegal move).
+    pub stale: bool,
+    /// The exchange, shared with the log thread.
+    pub exchange: Arc<Exchange>,
+}
+
+/// The last [`HISTORY_LEN`] exchanges, oldest first.
+#[derive(Clone, Debug, Default, PartialEq)]
+pub struct History {
+    records: VecDeque<Record>,
+    recorded: u64,
+}
+
+impl History {
+    /// An empty history.
+    #[must_use]
+    pub fn new() -> History {
+        History::default()
+    }
+
+    /// Adds `exchange` as the newest record, dropping the oldest beyond [`HISTORY_LEN`].
+    pub fn push(&mut self, exchange: Exchange, stale: bool, time: SystemTime) -> &Record {
+        if self.records.len() == HISTORY_LEN {
+            self.records.pop_front();
+        }
+        self.recorded += 1;
+        self.records.push_back(Record {
+            number: self.recorded,
+            time,
+            stale,
+            exchange: Arc::new(exchange),
+        });
+        &self.records[self.records.len() - 1]
+    }
+
+    /// Records kept.
+    pub fn len(&self) -> usize {
+        self.records.len()
+    }
+
+    /// True before the first exchange.
+    pub fn is_empty(&self) -> bool {
+        self.records.is_empty()
+    }
+
+    /// The record at `index`, oldest first.
+    pub fn get(&self, index: usize) -> Option<&Record> {
+        self.records.get(index)
+    }
+
+    /// The newest record.
+    pub fn last(&self) -> Option<&Record> {
+        self.records.back()
+    }
+
+    /// Where the record numbered `number` is, if it is still kept.
+    pub fn index_of(&self, number: u64) -> Option<usize> {
+        self.records
+            .iter()
+            .position(|record| record.number == number)
+    }
+
+    /// The records, oldest first.
+    pub fn iter(&self) -> impl Iterator<Item = &Record> {
+        self.records.iter()
+    }
+}
+
+// ----- the view -----
+
+/// Where the exchange view is: which exchange it shows and how far its body is
+/// scrolled. The page size and the scroll limit come from the last draw, so the keys
+/// stop where the screen does.
+#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
+pub struct ExchangeView {
+    /// The [`Record::number`] shown; `None` while there are no exchanges.
+    pub shown: Option<u64>,
+    /// Body rows scrolled off the top.
+    pub scroll: usize,
+    /// Body rows on screen at the last draw.
+    pub page: usize,
+    /// The largest `scroll` at the last draw: the body's last row at the bottom.
+    pub max_scroll: usize,
+}
+
+impl ExchangeView {
+    /// The view on the newest exchange in `history`, scrolled to the top.
+    #[must_use]
+    pub fn open(history: &History) -> ExchangeView {
+        ExchangeView {
+            shown: history.last().map(|record| record.number),
+            ..ExchangeView::default()
+        }
+    }
+
+    /// Where the exchange shown is in `history`: the oldest one kept when it has been
+    /// dropped since, `None` when there is none.
+    pub fn index(&self, history: &History) -> Option<usize> {
+        let number = self.shown?;
+        history
+            .index_of(number)
+            .or_else(|| (!history.is_empty()).then_some(0))
+    }
+
+    /// A new exchange numbered `number` has arrived: shown when nothing is, otherwise
+    /// the view stays where it is.
+    pub fn follow(&mut self, number: u64) {
+        if self.shown.is_none() {
+            self.show(Some(number));
+        }
+    }
+
+    /// Shows the next older exchange, if any.
+    pub fn older(&mut self, history: &History) {
+        if let Some(index) = self.index(history) {
+            let number = history.get(index.saturating_sub(1)).map(|r| r.number);
+            self.show(number);
+        }
+    }
+
+    /// Shows the next newer exchange, if any.
+    pub fn newer(&mut self, history: &History) {
+        if let Some(index) = self.index(history) {
+            let newest = history.len().saturating_sub(1);
+            let number = history.get((index + 1).min(newest)).map(|r| r.number);
+            self.show(number);
+        }
+    }
+
+    fn show(&mut self, number: Option<u64>) {
+        if number != self.shown {
+            *self = ExchangeView {
+                shown: number,
+                ..ExchangeView::default()
+            };
+        }
+    }
+
+    /// Scrolls `rows` rows up.
+    pub fn up(&mut self, rows: usize) {
+        self.scroll = self.scroll.saturating_sub(rows);
+    }
+
+    /// Scrolls `rows` rows down, no further than the body's end.
+    pub fn down(&mut self, rows: usize) {
+        self.scroll = self.scroll.saturating_add(rows).min(self.max_scroll);
+    }
+
+    /// Scrolls a page up; a page keeps one row of the last one.
+    pub fn page_up(&mut self) {
+        self.up(self.page_step());
+    }
+
+    /// Scrolls a page down; a page keeps one row of the last one.
+    pub fn page_down(&mut self) {
+        self.down(self.page_step());
+    }
+
+    /// Scrolls to the top.
+    pub fn home(&mut self) {
+        self.scroll = 0;
+    }
+
+    /// Scrolls to the end.
+    pub fn end(&mut self) {
+        self.scroll = self.max_scroll;
+    }
+
+    fn page_step(&self) -> usize {
+        self.page.saturating_sub(1).max(1)
+    }
+}
+
+// ----- the log -----
+
+/// A log record: the fields in the order the file shows them.
+#[derive(Serialize)]
+struct LogRecord<'a> {
+    time: String,
+    ply: usize,
+    played: &'a str,
+    source: &'a str,
+    stale: bool,
+    request: LogRequest<'a>,
+    attempts: Vec<LogAttempt<'a>>,
+}
+
+#[derive(Serialize)]
+struct LogRequest<'a> {
+    method: &'a str,
+    url: &'a str,
+    headers: Headers<'a>,
+    body: &'a Value,
+}
+
+/// Headers as a JSON object, in the order they were sent.
+struct Headers<'a>(&'a [(String, String)]);
+
+impl Serialize for Headers<'_> {
+    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
+        serializer.collect_map(self.0.iter().map(|(name, value)| (name, value)))
+    }
+}
+
+#[derive(Serialize)]
+struct LogAttempt<'a> {
+    status: Option<u16>,
+    elapsed_ms: u64,
+    response: Option<Value>,
+    error: Option<&'a str>,
+}
+
+/// `record` as one line of JSON (without the line break): `time` (RFC 3339, UTC),
+/// `ply`, `played` (SAN), `source`, `stale`, `request` (`method`, `url`, `headers` as an
+/// object, `body`) and `attempts` (each `status` or null, `elapsed_ms`, `response` as
+/// JSON when it parses, else as a string, or null, and `error` or null).
+///
+/// # Errors
+///
+/// Only if serde_json cannot write the record, which it always can.
+pub fn log_line(record: &Record) -> serde_json::Result<String> {
+    let exchange = &record.exchange;
+    let http = &exchange.http;
+    serde_json::to_string(&LogRecord {
+        time: rfc3339(record.time),
+        ply: exchange.ply,
+        played: &exchange.san,
+        source: &exchange.source,
+        stale: record.stale,
+        request: LogRequest {
+            method: &http.method,
+            url: &http.url,
+            headers: Headers(&http.headers),
+            body: &http.body,
+        },
+        attempts: http
+            .attempts
+            .iter()
+            .map(|attempt| LogAttempt {
+                status: attempt.status,
+                elapsed_ms: u64::try_from(attempt.elapsed.as_millis()).unwrap_or(u64::MAX),
+                response: attempt.response.as_deref().map(|text| {
+                    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
+                }),
+                error: attempt.error.as_deref(),
+            })
+            .collect(),
+    })
+}
+
+/// Why the debug log stopped.
+#[derive(Clone, Debug, PartialEq, Eq)]
+pub struct LogFailure {
+    /// A short reason, such as `permission denied`.
+    pub reason: String,
+    /// The log file, when there is one.
+    pub path: Option<PathBuf>,
+}
+
+/// The debug log: records go over a channel to the `debug-log` thread, which appends
+/// each as a line ([`log_line`]) and flushes it. The file and any missing folders are
+/// created with the first record: folders with mode 0700, the file with mode 0600 (an
+/// existing file is appended to and keeps its mode).
+///
+/// The first error ends the thread; [`DebugLog::failure`] then reports it once and
+/// nothing more is written. Records never wait: [`DebugLog::write`] only sends.
+pub struct DebugLog {
+    state: LogState,
+}
+
+enum LogState {
+    /// The thread is writing.
+    Running {
+        records: Sender<Record>,
+        failures: Receiver<LogFailure>,
+        /// Disconnects when the thread ends.
+        finished: Receiver<()>,
+        path: PathBuf,
+    },
+    /// No log could be started; reported once a record is written.
+    Unavailable(LogFailure),
+    /// Nothing more is written; the failure is kept until it is reported.
+    Stopped(Option<LogFailure>),
+}
+
+impl DebugLog {
+    /// The log at `path` ([`log_path`]); without one, a log that reports the missing
+    /// path at the first record.
+    #[must_use]
+    pub fn open(path: Option<PathBuf>) -> DebugLog {
+        match path {
+            Some(path) => DebugLog::start(path),
+            None => DebugLog {
+                state: LogState::Unavailable(LogFailure {
+                    reason: NO_LOG_PATH.to_string(),
+                    path: None,
+                }),
+            },
+        }
+    }
+
+    /// Starts the `debug-log` thread appending to `path`. Nothing touches the disk
+    /// until the first record.
+    #[must_use]
+    pub fn start(path: PathBuf) -> DebugLog {
+        let (records, queued) = mpsc::channel();
+        let (failed, failures) = mpsc::channel();
+        let (done, finished) = mpsc::channel::<()>();
+        let file = path.clone();
+        let spawned = thread::Builder::new()
+            .name(LOG_THREAD.to_string())
+            .spawn(move || {
+                // Dropped when the thread ends, however it ends.
+                let _done = done;
+                if let Err(failure) = write_records(&file, &queued) {
+                    // Nobody is left to tell once the log has been closed.
+                    let _ = failed.send(failure);
+                }
+            });
+        let state = match spawned {
+            Ok(_) => LogState::Running {
+                records,
+                failures,
+                finished,
+                path,
+            },
+            Err(error) => LogState::Unavailable(LogFailure {
+                reason: format!("cannot start the {LOG_THREAD} thread ({error})"),
+                path: Some(path),
+            }),
+        };
+        DebugLog { state }
+    }
+
+    /// Queues `record` for the log; never blocks.
+    pub fn write(&mut self, record: &Record) {
+        match &mut self.state {
+            LogState::Running { records, .. } => {
+                // Fails only once the thread has stopped, and `failure` says why.
+                let _ = records.send(record.clone());
+            }
+            LogState::Unavailable(failure) => {
+                let failure = failure.clone();
+                self.state = LogState::Stopped(Some(failure));
+            }
+            LogState::Stopped(_) => {}
+        }
+    }
+
+    /// Why the log stopped, the first time this is asked after it did; `None` while it
+    /// works and ever after.
+    pub fn failure(&mut self) -> Option<LogFailure> {
+        match &mut self.state {
+            LogState::Running { failures, path, .. } => {
+                let failure = match failures.try_recv() {
+                    Ok(failure) => failure,
+                    Err(TryRecvError::Empty) => return None,
+                    // The thread ended without saying why: it panicked.
+                    Err(TryRecvError::Disconnected) => LogFailure {
+                        reason: format!("the {LOG_THREAD} thread stopped"),
+                        path: Some(path.clone()),
+                    },
+                };
+                self.state = LogState::Stopped(None);
+                Some(failure)
+            }
+            LogState::Stopped(failure) => failure.take(),
+            LogState::Unavailable(_) => None,
+        }
+    }
+
+    /// Stops taking records and waits up to `grace` for the thread to write the ones
+    /// queued, so the last exchanges of a session reach the file.
+    pub fn close(self, grace: Duration) {
+        if let LogState::Running {
+            records, finished, ..
+        } = self.state
+        {
+            drop(records);
+            // Disconnected once the thread has written everything and ended.
+            let _ = finished.recv_timeout(grace);
+        }
+    }
+}
+
+/// The `debug-log` thread's work: appends every record from `queued` to `path` until
+/// the channel closes or a write fails.
+fn write_records(path: &Path, queued: &Receiver<Record>) -> Result<(), LogFailure> {
+    let fail = |error: io::Error| LogFailure {
+        reason: describe(&error),
+        path: Some(path.to_path_buf()),
+    };
+    let mut file = None;
+    for record in queued {
+        let file = match &mut file {
+            Some(file) => file,
+            empty => empty.insert(open_log(path).map_err(fail)?),
+        };
+        append(file, &record).map_err(fail)?;
+    }
+    Ok(())
+}
+
+/// Opens `path` for appending, creating it (mode 0600) and its missing folders (mode
+/// 0700) as needed.
+fn open_log(path: &Path) -> io::Result<File> {
+    if let Some(folder) = path
+        .parent()
+        .filter(|folder| !folder.as_os_str().is_empty())
+    {
+        let mut builder = DirBuilder::new();
+        builder.recursive(true);
+        #[cfg(unix)]
+        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
+        builder.create(folder)?;
+    }
+    let mut options = OpenOptions::new();
+    options.append(true).create(true);
+    #[cfg(unix)]
+    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
+    options.open(path)
+}
+
+/// Writes `record` as one line and flushes it.
+fn append(file: &mut File, record: &Record) -> io::Result<()> {
+    let mut line = log_line(record).map_err(io::Error::other)?;
+    line.push('\n');
+    file.write_all(line.as_bytes())?;
+    file.flush()
+}
+
+// ----- the session -----
+
+/// Debug mode's state in the app: the history and the log.
+pub struct DebugSession {
+    history: History,
+    log: DebugLog,
+}
+
+impl DebugSession {
+    /// A session with no exchanges yet, logging to `log`.
+    #[must_use]
+    pub fn new(log: DebugLog) -> DebugSession {
+        DebugSession {
+            history: History::new(),
+            log,
+        }
+    }
+
+    /// The exchanges kept.
+    pub fn history(&self) -> &History {
+        &self.history
+    }
+
+    /// Keeps `exchange` (received at `time`, `stale` when it was not played) and queues
+    /// it for the log. Returns its [`Record::number`].
+    pub fn record(&mut self, exchange: Exchange, stale: bool, time: SystemTime) -> u64 {
+        let record = self.history.push(exchange, stale, time);
+        self.log.write(record);
+        record.number
+    }
+
+    /// See [`DebugLog::failure`].
+    pub fn log_failure(&mut self) -> Option<LogFailure> {
+        self.log.failure()
+    }
+
+    /// Closes the log ([`DebugLog::close`]); later exchanges are kept but not logged.
+    pub fn close_log(&mut self, grace: Duration) {
+        let stopped = DebugLog {
+            state: LogState::Stopped(None),
+        };
+        std::mem::replace(&mut self.log, stopped).close(grace);
+    }
+}
+
 #[cfg(test)]
 mod tests {
     use std::collections::HashMap;
@@ -25,7 +797,7 @@ mod tests {
     use serde_json::json;
 
     use super::*;
-    use crate::engine::JevAttempt;
+    use crate::engine::{JevAttempt, recorded_exchange};
     use crate::tui::test_support::TempDir;
     use crate::tui::test_support::engine::{SENTINEL_KEY, jev_exchange, jev_move};
 
@@ -501,7 +1273,14 @@ mod tests {
 
     #[test]
     fn the_key_is_nowhere_in_an_exchange_or_its_log_line() {
-        let record = record(false);
+        // An exchange the engine recorded against a server that echoes the key.
+        let game = Game::new();
+        let computer = jev_move(game.position(), "e2e4");
+        let http = recorded_exchange(SENTINEL_KEY);
+        let mut history = History::new();
+        let record = history
+            .push(Exchange::new(&game, &computer, http), false, at_ms(0))
+            .clone();
         let line = log_line(&record).unwrap();
         let text = texts(&record.exchange).join("\n");
         for shown in [line, text, format!("{record:?}")] {
diff --git a/src/tui/files.rs b/src/tui/files.rs
index 712e046c8fa7bc0e2fd5b13e647ec8c104978743..a8901295f6b2faa9d981bed9a809da16219643c4 100644
--- a/src/tui/files.rs
+++ b/src/tui/files.rs
@@ -181,7 +181,7 @@ fn sync_parent(path: &Path) {
 }
 
 /// A short, user-facing reason for a failed file operation.
-fn describe(err: &io::Error) -> String {
+pub fn describe(err: &io::Error) -> String {
     match err.kind() {
         ErrorKind::NotFound => "folder does not exist".to_string(),
         ErrorKind::PermissionDenied => "permission denied".to_string(),
diff --git a/src/tui/mod.rs b/src/tui/mod.rs
index f5d84934a52156ed48a17637d733ed37e20bc588..ad6f0075e93df937df0566347e163464ef2a29f1 100644
--- a/src/tui/mod.rs
+++ b/src/tui/mod.rs
@@ -5,7 +5,8 @@
 //! [`run`] is the whole program: it reads the command line and the environment,
 //! builds the computer player, sets up the terminal, asks it about graphics
 //! ([`graphics::detect`]) and runs the main loop until the user quits or a signal
-//! asks it to stop.
+//! asks it to stop. In debug mode ([`debug::enabled`]) the computer player records
+//! its exchanges with Jev, and the app keeps them and writes the debug log.
 
 pub mod app;
 pub mod board;
@@ -38,6 +39,7 @@ use crate::core::Game;
 use crate::engine::{ComputerMove, ComputerPlayer, EngineConfig};
 
 use self::app::{Action, App};
+use self::debug::DebugLog;
 use self::event::AppEvent;
 use self::graphics::{Graphics, LateAnswers};
 use self::worker::{Engine, EngineOutcome, EngineReply};
@@ -50,18 +52,23 @@ const TICK: Duration = Duration::from_millis(50);
 /// arrives, and the process should then end by that signal rather than by the error.
 const SIGNAL_GRACE: Duration = Duration::from_millis(100);
 
+/// How long quitting waits for the debug log to write the exchanges still queued.
+const LOG_GRACE: Duration = Duration::from_millis(500);
+
 /// The `--help` text.
 const USAGE: &str = concat!(
     "rchess: chess in the terminal, against a person or the Jev computer player\n",
     "\n",
     "Usage: ",
     env!("CARGO_PKG_NAME"),
-    " [--glyphs image|solid|outline|ascii]\n",
+    " [--glyphs image|solid|outline|ascii] [--debug]\n",
     "\n",
     "Options:\n",
     "  --glyphs <set>     how pieces look: image (pictures; the default when the\n",
     "                     terminal can show them), solid (otherwise the default),\n",
     "                     outline or ascii; `g` cycles them during a game\n",
+    "  --debug            debug mode: keep every Jev request and answer, show them\n",
+    "                     with `d` during a game and append them to the debug log\n",
     "  -h, --help         show this help and exit\n",
     "\n",
     "Environment:\n",
@@ -72,6 +79,10 @@ const USAGE: &str = concat!(
     "  JEV_FILTER_LOSING  keep losing moves off Jev's shortlist (default true)\n",
     "  RCHESS_GLYPHS      glyph set when --glyphs is not given\n",
     "  RCHESS_IMAGES      off: no piece pictures, and no graphics query at start\n",
+    "  RCHESS_DEBUG       debug mode as with --debug, unless empty or 0\n",
+    "  RCHESS_DEBUG_LOG   the debug log file; the default is\n",
+    "                     $XDG_STATE_HOME/rchess/jev-debug.jsonl, else\n",
+    "                     ~/.local/state/rchess/jev-debug.jsonl\n",
     "  NO_COLOR           no colours: start with outline glyphs unless a set is\n",
     "                     chosen, mark board highlights with text, no pictures\n",
     "  COLORTERM          truecolor or 24bit selects 24-bit colours\n",
@@ -87,7 +98,8 @@ const USAGE: &str = concat!(
 /// program: they are listed as warnings on the menu, like invalid engine
 /// settings, and so is a graphics query that failed. The computer player comes
 /// from `EngineConfig::from_env()`; with no `JEV_API_KEY` it plays by local
-/// search and never uses the network.
+/// search and never uses the network. In debug mode (`--debug` or `RCHESS_DEBUG`)
+/// it records its exchanges with Jev (`EngineConfig::trace`).
 ///
 /// Unless images are off ([`glyphs::images_wanted`]), the terminal is asked about
 /// graphics right after it is set up, which takes up to [`graphics::QUERY_TIMEOUT`]
@@ -118,8 +130,8 @@ pub fn run(args: impl IntoIterator<Item = String>) -> io::Result<()> {
     let env = |name: &str| std::env::var(name).ok();
     let images = glyphs::images_wanted(options.glyphs.as_deref(), env);
     let fault = injected_fault(env("RCHESS_FAULT").as_deref());
-    let mut engine: Arc<dyn Engine> =
-        Arc::new(ComputerPlayer::from_config(EngineConfig::from_env()));
+    let config = engine_config(EngineConfig::from_env(), debug::enabled(options.debug, env));
+    let mut engine: Arc<dyn Engine> = Arc::new(ComputerPlayer::from_config(config));
     if fault == Some(Fault::EnginePanic) {
         engine = Arc::new(PanickingEngine(engine));
     }
@@ -142,6 +154,14 @@ pub fn run(args: impl IntoIterator<Item = String>) -> io::Result<()> {
     result
 }
 
+/// `config` with the exchange recording debug mode needs (`trace`) on when `debug` is.
+fn engine_config(config: EngineConfig, debug: bool) -> EngineConfig {
+    EngineConfig {
+        trace: debug,
+        ..config
+    }
+}
+
 /// Why the UI cannot run, given whether stdin and stdout are terminals: it reads keys
 /// from one and draws on the other.
 fn terminal_problem(stdin_is_terminal: bool, stdout_is_terminal: bool) -> Option<&'static str> {
@@ -157,7 +177,8 @@ fn terminal_problem(stdin_is_terminal: bool, stdout_is_terminal: bool) -> Option
 /// The app for the options and environment (`get`), once the terminal has been
 /// asked about graphics: the starting glyph set follows [`Graphics::support`], and
 /// the menu lists the command-line warnings, then the glyph warnings, then the
-/// graphics query's (after the engine's own, which `App::new` adds).
+/// graphics query's (after the engine's own, which `App::new` adds). In debug mode
+/// it starts the debug log at [`debug::log_path`].
 fn build_app(
     options: Options,
     engine: Arc<dyn Engine>,
@@ -172,13 +193,16 @@ fn build_app(
     let mut app = App::new(engine, glyph_set, glyphs::detect_truecolor(&get), warnings)
         .with_no_color(glyphs::no_color(&get))
         .with_picker(graphics.picker);
+    if debug::enabled(options.debug, &get) {
+        app = app.with_debug(DebugLog::open(debug::log_path(&get)));
+    }
     app.set_cell_size(graphics.cell_size);
     app
 }
 
 /// Sets up the terminal, asks it about graphics when `images` is true, builds the
-/// app from the result, runs the main loop and restores the terminal (also on an
-/// error) before returning.
+/// app from the result, runs the main loop, gives the debug log a moment to finish,
+/// and restores the terminal (also on an error) before returning.
 fn play(
     quit: &AtomicI32,
     images: bool,
@@ -204,7 +228,7 @@ fn play(
     screen.backend_mut().clear()?;
     let mut late = LateAnswers::after(&graphics, Instant::now());
     let mut app = build(graphics);
-    run_loop(
+    let result = run_loop(
         &mut app,
         quit,
         |app| {
@@ -220,7 +244,9 @@ fn play(
             }
             Ok(batch)
         },
-    )
+    );
+    app.close_debug_log(LOG_GRACE);
+    result
 }
 
 /// Removes from `batch` the key presses of a graphics answer that came after the
@@ -327,11 +353,13 @@ struct Options {
     /// The last `--glyphs` value, still unchecked: `glyphs::initial_glyphs`
     /// validates it and warns about a bad one.
     glyphs: Option<String>,
+    /// `--debug` was given ([`debug::enabled`] also reads `RCHESS_DEBUG`).
+    debug: bool,
     /// Notes about ignored arguments, shown on the menu.
     warnings: Vec<String>,
 }
 
-/// Reads `--glyphs <set>`, `--glyphs=<set>`, `-h` and `--help`. The last
+/// Reads `--glyphs <set>`, `--glyphs=<set>`, `--debug`, `-h` and `--help`. The last
 /// `--glyphs` wins. A `--glyphs` followed by nothing or by another option has no
 /// value; that and any other argument become warnings rather than errors.
 fn parse_args(args: impl IntoIterator<Item = String>) -> Cli {
@@ -340,6 +368,7 @@ fn parse_args(args: impl IntoIterator<Item = String>) -> Cli {
     while let Some(arg) = args.next() {
         match arg.as_str() {
             "-h" | "--help" => return Cli::Help,
+            "--debug" => options.debug = true,
             "--glyphs" => match args.next_if(|value| !value.starts_with('-')) {
                 Some(value) => options.glyphs = Some(value),
                 None => options
@@ -414,6 +443,7 @@ fn perform(action: Action, engine: &Arc<dyn Engine>, replies: &Sender<EngineRepl
                     outcome: EngineOutcome::Failed(format!(
                         "cannot start the engine thread: {error}"
                     )),
+                    exchange: None,
                 };
                 // The receiver lives in `run_loop`, which is still running.
                 let _ = replies.send(failed);
diff --git a/src/tui/panels.rs b/src/tui/panels.rs
index 1de0ac94f362d3d55c43df3da0438fd25095a085..f91df4a246a18f957e63da353848cbccbb779710 100644
--- a/src/tui/panels.rs
+++ b/src/tui/panels.rs
@@ -1,9 +1,10 @@
 //! Drawing every screen (spec sections 6.2, 6.3 and 9.2).
 //!
 //! [`draw`] renders an [`App`] through its public accessors only: the menu, the playing
-//! screen's panels, the game-over overlay, the top dialog and the too-small notice. It
-//! returns the [`HitMap`] of everything clickable, which the app keeps for the next mouse
-//! event, and the move-list scroll clamped to what the list can show. The board's piece
+//! screen's panels, the game-over overlay, the Jev exchange view (debug mode), the top
+//! dialog and the too-small notice. It returns the [`HitMap`] of everything clickable,
+//! which the app keeps for the next mouse event, the move-list scroll clamped to what the
+//! list can show, and the exchange view's page size and scroll limit. The board's piece
 //! pictures (the Image style) are kept in the [`PieceImages`] the app lends it, and are
 //! not drawn where the game-over box or a dialog will cover them.
 //!
@@ -20,6 +21,12 @@
 //! latency, model and the note) and takes the rows from Moves, which gets the rest and
 //! keeps at least [`MOVES_MIN_ROWS`]; when even that is not enough, only the note is cut
 //! short, ending in `…`. Menu, dialogs, help and the game-over overlay stay centred boxes.
+//! In debug mode the Status panel's border says `DEBUG`.
+//!
+//! The exchange view takes the whole screen instead of the playing screen (nothing of the
+//! board is drawn under it, so no piece picture is lost under it), with dialogs still on
+//! top. Its body text comes ready-made with the exchange ([`Exchange::body`]); a frame only
+//! wraps the rows it shows.
 //!
 //! The snapshot tests at the bottom of this file render every screen; their files in
 //! `src/tui/snapshots/` (`chess__tui__panels__tests__*.snap`) show the exact output at
@@ -46,6 +53,7 @@ use super::app::{
     is_too_small, move_rows, outcome_text,
 };
 use super::board::{BoardGeometry, BoardView, CellSize, PieceImages, layout_board};
+use super::debug::{Exchange, ExchangeView, LineKind, NO_EXCHANGES, Record};
 use super::glyphs::{self, ELLIPSIS, GlyphSet, Palette, char_width};
 use super::input::LineEditor;
 use crate::core::{Color as Side, Game, Piece, PieceKind, Position as ChessPosition};
@@ -54,7 +62,7 @@ use crate::engine::{ComputerMove, MoveSource};
 /// The help dialog's text. The first [`HELP_KEY_WIDTH`] characters of each line are the
 /// key column (drawn bold); no line is wider than 56 cells, so the dialog fits a 60-column
 /// terminal.
-pub const HELP_LINES: [&str; 13] = [
+pub const HELP_LINES: [&str; 14] = [
     "Mouse     click a piece, then a square, or drag it there",
     "Arrows    move the cursor; Enter picks up and puts down",
     "Esc       drop the piece, or leave the command box",
@@ -64,6 +72,7 @@ pub const HELP_LINES: [&str; 13] = [
     "            :fen <FEN>  :savefen <path>  :savepgn <path>",
     "u  f  n   undo, flip the board, new game",
     "g  m  ?   glyph set, menu, this help",
+    "d         exchange view (start with --debug)",
     "Ctrl+S    save the game as PGN",
     "q         quit (Ctrl+C works everywhere)",
     "Space     pause watching, or retry a failed engine",
@@ -99,6 +108,10 @@ const PROMPT: &str = "> ";
 const PROMPT_WIDTH: u16 = 2;
 /// Dialog borders.
 const ACCENT: Color = Color::Cyan;
+/// The Status panel's border tag in debug mode.
+const DEBUG_TAG: &str = " DEBUG ";
+/// The exchange view's keys, on its bottom border.
+const EXCHANGE_KEYS: &str = " ↑↓ PgUp PgDn Home End · ←→ older/newer · Esc closes ";
 /// Pawn, knight, bishop, rook, queen and king values for the material count; the king
 /// never leaves the board, so its value is irrelevant.
 const PIECE_VALUES: [i32; 6] = [1, 3, 3, 5, 9, 0];
@@ -111,6 +124,9 @@ pub struct Drawn {
     /// The move-list scroll, clamped so the list never scrolls past its first row.
     /// Unchanged when the move list was not drawn.
     pub move_scroll: usize,
+    /// The exchange view with its scroll clamped and its page size and scroll limit as
+    /// drawn. Unchanged when the view was not drawn.
+    pub exchange_view: Option<ExchangeView>,
 }
 
 /// Draws `app` into `frame` and returns the click targets and the clamped move-list
@@ -120,6 +136,7 @@ pub fn draw(app: &App, images: &mut PieceImages, frame: &mut Frame, now: Instant
     let mut drawn = Drawn {
         hits: HitMap::default(),
         move_scroll: app.move_scroll(),
+        exchange_view: app.exchange_view(),
     };
     let area = frame.area();
     if is_too_small(area) {
@@ -127,10 +144,13 @@ pub fn draw(app: &App, images: &mut PieceImages, frame: &mut Frame, now: Instant
         return drawn;
     }
     let overlays = overlays(app, area);
-    match app.screen() {
-        Screen::Menu => menu(frame, area, app, &mut drawn.hits),
-        Screen::Playing => playing(frame, area, app, images, &overlays, now, &mut drawn),
-        Screen::GameOver => {
+    match (app.exchange_view(), app.screen()) {
+        (Some(view), _) => drawn.exchange_view = Some(exchange_screen(frame, area, app, view)),
+        (None, Screen::Menu) => menu(frame, area, app, &mut drawn.hits),
+        (None, Screen::Playing) => {
+            playing(frame, area, app, images, &overlays, now, &mut drawn);
+        }
+        (None, Screen::GameOver) => {
             playing(frame, area, app, images, &overlays, now, &mut drawn);
             game_over(frame, area, app, &mut drawn.hits);
         }
@@ -504,15 +524,22 @@ fn command_panel(frame: &mut Frame, area: Rect, editor: &LineEditor, focused: bo
     }
 }
 
-/// Status: the mode on the top border (the first of [`App::mode_labels`] that fits, so a
-/// narrow panel shows `You (W) vs Local` rather than nothing), then whose turn it is, the
-/// Jev vs Jev pace, the thinking spinner and the latest message.
+/// Status: `DEBUG` in debug mode and the mode on the top border (the first of
+/// [`App::mode_labels`] that fits, so a narrow panel shows `You (W) vs Local` rather than
+/// nothing), then whose turn it is, the Jev vs Jev pace, the thinking spinner and the
+/// latest message.
 fn status_panel(frame: &mut Frame, area: Rect, app: &App, now: Instant) {
     const TITLE: &str = " Status ";
-    let block = side_block("Status");
+    let mut block = side_block("Status");
+    let mut titles = TITLE.len();
+    if app.debug_mode() {
+        block = block.title_top(Line::from(DEBUG_TAG).yellow().bold());
+        // One border cell between the two titles.
+        titles += DEBUG_TAG.len() + 1;
+    }
     let inner = block.inner(area);
     // Two corners and at least two border cells between the titles.
-    let room = usize::from(area.width).saturating_sub(TITLE.len() + 4);
+    let room = usize::from(area.width).saturating_sub(titles + 4);
     let mode = app
         .mode_labels()
         .into_iter()
@@ -947,6 +974,135 @@ fn piece_span(piece: Piece, glyph_set: GlyphSet, palette: &Palette) -> Span<'sta
     )
 }
 
+// ----- exchange view -----
+
+/// The Jev exchange view on the whole of `area` (spec 9.4): the exchange `view` shows,
+/// with a header saying which one it is and how it went, then its body, wrapped and
+/// scrolled; or [`NO_EXCHANGES`]. Returns `view` with its scroll clamped and its page size
+/// and scroll limit for this size.
+fn exchange_screen(frame: &mut Frame, area: Rect, app: &App, view: ExchangeView) -> ExchangeView {
+    let block = Block::bordered()
+        .title(Line::from(" Jev exchange ").bold())
+        .title_bottom(Line::from(EXCHANGE_KEYS).dim())
+        .padding(Padding::horizontal(1));
+    let inner = block.inner(area);
+    let history = app.exchanges();
+    let shown = history.and_then(|history| {
+        let index = view.index(history)?;
+        Some((index, history.len(), history.get(index)?))
+    });
+    let Some((index, count, record)) = shown else {
+        frame.render_widget(block, area);
+        frame.render_widget(Line::from(NO_EXCHANGES).dim(), row_of(inner, 0));
+        return ExchangeView {
+            scroll: 0,
+            page: usize::from(inner.height),
+            max_scroll: 0,
+            ..view
+        };
+    };
+    let header = pack(exchange_header(record, index, count), " · ", inner.width);
+    let header_rows = header
+        .iter()
+        .map(|line| wrapped_height(&line.to_string(), inner.width))
+        .fold(0u16, u16::saturating_add)
+        .min(inner.height);
+    // One blank row between the header and the body.
+    let body_top = header_rows.saturating_add(1).min(inner.height);
+    let body_area = Rect {
+        y: inner.y + body_top,
+        height: inner.height - body_top,
+        ..inner
+    };
+    let exchange = &record.exchange;
+    let width = usize::from(body_area.width.max(1));
+    let page = usize::from(body_area.height);
+    let total: usize = exchange.body().iter().map(|line| line.rows(width)).sum();
+    let max_scroll = total.saturating_sub(page);
+    let scroll = view.scroll.min(max_scroll);
+    let block = if total > page {
+        let last = (scroll + page).min(total);
+        block.title_top(Line::from(format!(" {}-{last} of {total} ", scroll + 1)).right_aligned())
+    } else {
+        block
+    };
+    frame.render_widget(block, area);
+    frame.render_widget(
+        Paragraph::new(header).wrap(Wrap { trim: true }),
+        Rect {
+            height: header_rows,
+            ..inner
+        },
+    );
+    frame.render_widget(
+        Paragraph::new(body_rows(exchange, width, scroll, page)),
+        body_area,
+    );
+    ExchangeView {
+        scroll,
+        page,
+        max_scroll,
+        ..view
+    }
+}
+
+/// The exchange view's header for `record`, the `index`th of `count` (from 0), as items
+/// for [`pack`]: `exchange N of M`, `move <fullmove>`, the move, how it was chosen, the
+/// last status, the attempts and the latency, and `stale — not played` when it was not.
+fn exchange_header(record: &Record, index: usize, count: usize) -> Vec<Vec<Span<'static>>> {
+    let exchange = &record.exchange;
+    let attempts = match exchange.attempts() {
+        1 => "1 attempt".to_string(),
+        n => format!("{n} attempts"),
+    };
+    let mut items = vec![
+        vec![Span::raw(format!("exchange {} of {count}", index + 1)).bold()],
+        vec![Span::raw(format!("move {}", exchange.fullmove))],
+        vec![Span::raw(exchange.san.clone()).bold()],
+        vec![Span::raw(exchange.source.clone())],
+        vec![Span::raw(exchange.status())],
+        vec![Span::raw(attempts)],
+        vec![Span::raw(format!("{} ms", exchange.latency.as_millis()))],
+    ];
+    if record.stale {
+        items.push(vec![Span::raw("stale — not played").yellow()]);
+    }
+    items
+}
+
+/// The rows of `exchange`'s body wrapped at `width` cells, from row `scroll`, at most
+/// `page` of them. Only the lines on screen are wrapped.
+fn body_rows(exchange: &Exchange, width: usize, scroll: usize, page: usize) -> Vec<Line<'static>> {
+    let mut rows = Vec::new();
+    let mut top = 0;
+    for line in exchange.body() {
+        if rows.len() == page {
+            break;
+        }
+        let height = line.rows(width);
+        if top + height <= scroll {
+            top += height;
+            continue;
+        }
+        let style = match line.kind {
+            LineKind::Heading => Style::new().bold(),
+            LineKind::Error => Style::new().red(),
+            LineKind::Text => Style::new(),
+        };
+        let skip = scroll.saturating_sub(top);
+        let room = page - rows.len();
+        rows.extend(
+            line.wrapped(width)
+                .into_iter()
+                .skip(skip)
+                .take(room)
+                .map(|row| Line::styled(row, style)),
+        );
+        top += height;
+    }
+    rows
+}
+
 // ----- overlays -----
 
 /// Where the boxes drawn over the playing screen go in `area`: the game-over box (on its
diff --git a/src/tui/worker.rs b/src/tui/worker.rs
index a9a179db59f77d3c5cabb4f42dd68a5ff1208166..28e468eff95fa345f6def9e8949ad1a55f5f15d8 100644
--- a/src/tui/worker.rs
+++ b/src/tui/worker.rs
@@ -12,6 +12,10 @@
 //! replies with its best move, noted [`ENGINE_ERROR_NOTE`]. The UI thread only
 //! ever applies ready-made moves: a search can take tens of seconds on a crowded
 //! position, and a panic in it must not reach the UI.
+//!
+//! In debug mode the engine records its exchange with Jev in the move; the thread
+//! moves it into the reply as a [`debug::Exchange`](super::debug::Exchange), whose
+//! text for the exchange view is rendered there too, off the UI thread.
 
 use std::any::Any;
 use std::io;
@@ -21,6 +25,7 @@ use std::sync::mpsc::Sender;
 use std::thread;
 use std::time::Instant;
 
+use super::debug::Exchange;
 use crate::core::Game;
 use crate::engine::{ComputerMove, ComputerPlayer, EngineConfig, MoveChooser, MoveSource, analyse};
 
@@ -131,6 +136,31 @@ pub struct EngineReply {
     pub hash: u64,
     /// What the engine produced.
     pub outcome: EngineOutcome,
+    /// The exchange with Jev behind a move, when the engine recorded one (debug mode).
+    /// Taken out of the move, so [`EngineOutcome::Move`] never holds it.
+    pub exchange: Option<Box<Exchange>>,
+}
+
+impl EngineReply {
+    /// The reply to `request` with `outcome`. The exchange a traced move carries
+    /// (`ComputerMove::exchange`) moves into [`EngineReply::exchange`], with the move
+    /// and the position it was for.
+    #[must_use]
+    pub fn new(request: &EngineRequest, mut outcome: EngineOutcome) -> EngineReply {
+        let exchange = match &mut outcome {
+            EngineOutcome::Move(computer) => computer
+                .exchange
+                .take()
+                .map(|http| Box::new(Exchange::new(&request.game, computer, *http))),
+            EngineOutcome::GameOver | EngineOutcome::Failed(_) => None,
+        };
+        EngineReply {
+            generation: request.generation,
+            hash: request.hash,
+            outcome,
+            exchange,
+        }
+    }
 }
 
 /// Runs `request` on a new detached thread named "engine" and sends the reply on
@@ -150,18 +180,9 @@ pub fn spawn_request(
     thread::Builder::new()
         .name(ENGINE_THREAD.to_string())
         .spawn(move || {
-            let EngineRequest {
-                generation,
-                hash,
-                game,
-            } = request;
-            let outcome = run_engine(engine.as_ref(), &game, local_search_move);
+            let outcome = run_engine(engine.as_ref(), &request.game, local_search_move);
             // A send error means the UI has gone; nobody is left to tell.
-            let _ = tx.send(EngineReply {
-                generation,
-                hash,
-                outcome,
-            });
+            let _ = tx.send(EngineReply::new(&request, outcome));
         })
         // Detached: quitting never waits for a slow engine call.
         .map(drop)
````

- [ ] **Step: Run the tests to verify they pass**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::` and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test --lib engine::`
Expected: tui:: 416 passed, engine:: 110 passed; `find src -name '*.snap.new'` prints nothing.

Run: `cargo fmt --check` and `cargo clippy --all-targets 2>&1 | grep -A5 -E "src/tui|src/engine|src/main.rs"`
Expected: no output from either.

- [ ] **Step: Update the handoff file**

Edit `docs/handoff/HANDOFF.md`. In `## Current`, set:

```
Sub-project: tui-polish | Plan: docs/superpowers/plans/2026-09-27-tui-polish.md
Branch: feat/tui-polish
Last completed task: tui-polish task 6 (Debug mode: exchange history, view and log)
Next task: tui-polish task 7 (pty smoke scenarios)
State: green (except 3 pre-existing old-code test failures)
```

Set `## Verify before continuing` to:

```
env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui:: && cargo test --lib engine::
```

Add at the top of `## Log (newest first)`: `- <today's date> tui-polish task 6 done: Debug mode: exchange history, view and log`. Keep every other section. Record under `## Notes / decisions made during work` any decision this plan did not specify.

- [ ] **Step: Refresh the knowledge graph**

Run: `graphify update .`
Expected: completes without error. Never commit `graphify-out/`, `.claude/` or `.superpowers/`.

- [ ] **Step: Commit**

```bash
git add docs/handoff/HANDOFF.md src/engine/jev.rs src/engine/mod.rs src/tui/app.rs src/tui/debug.rs src/tui/event.rs src/tui/files.rs src/tui/mod.rs src/tui/panels.rs src/tui/snapshots/chess__tui__panels__tests__exchange_120x40_stale.snap src/tui/snapshots/chess__tui__panels__tests__exchange_80x24.snap src/tui/snapshots/chess__tui__panels__tests__help_80x24.snap src/tui/test_support/engine.rs src/tui/test_support/harness.rs src/tui/worker.rs
git commit -m "feat(tui): keep the Jev exchanges in debug mode, show them with d and log them

Co-Authored-By: <the model you are> <noreply@anthropic.com>"
```

Replace `<the model you are>` with your own model name.

---

### Task 7: Pseudo-terminal scenarios for the graphics query and debug mode

**Files:**
- Modify: `src/tui/mod.rs`, `tests/pty_smoke.py`

**Interfaces:**
- Consumes: the binary (`cargo run`), `RCHESS_IMAGES`, `--glyphs`, `--debug`, `RCHESS_DEBUG_LOG`.
- Produces: new `tests/pty_smoke.py` scenarios: a terminal that never answers the query, one that answers late, one that answers like Kitty, one that answers Sixel, and debug mode; the existing scenarios run with the query skipped. A run-loop test that debug mode from the environment logs an exchange privately.

- [ ] **Step: Confirm the starting point**

Run: `git status --short` (only untracked `.claude/`, `graphify-out/` and user files) and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: 416 passed (engine: `cargo test --lib engine::` 110 passed).

- [ ] **Step: Apply the tests patch**

This task adds tests only: Tasks 4 and 6 already wired everything they check, so the tests pass as soon as they are applied (there is no RED step). The prototype proved they can fail by breaking the code on purpose: making the log 0644, removing the late-answer filter, and running the query after mouse and paste are enabled each made a named check fail.

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/tui/mod.rs b/src/tui/mod.rs
index ad6f0075e93df937df0566347e163464ef2a29f1..fe27341f8243be29a66453f4393e99b9d02dbf0e 100644
--- a/src/tui/mod.rs
+++ b/src/tui/mod.rs
@@ -927,6 +927,70 @@ mod tests {
         assert_eq!(line["stale"], false);
     }
 
+    #[test]
+    fn debug_mode_from_the_environment_logs_an_exchange_privately() {
+        // What `run` does with `RCHESS_DEBUG=1`: `build_app` finds the log path in the
+        // environment, and the first traced move creates the folders and the file. The
+        // binary cannot make a Jev request offline, so a fake engine records one.
+        let dir = TempDir::new("debug-env");
+        let (state, home) = (dir.join("state"), dir.join("home"));
+        let explicit = state.join("explicit").join("jev.jsonl");
+        let default = state.join("rchess").join("jev-debug.jsonl");
+        for (log_var, expected) in [(Some(&explicit), &explicit), (None, &default)] {
+            let mut vars = vec![
+                ("RCHESS_DEBUG", "1".to_string()),
+                ("XDG_STATE_HOME", state.display().to_string()),
+                ("HOME", home.display().to_string()),
+            ];
+            if let Some(path) = log_var {
+                vars.push(("RCHESS_DEBUG_LOG", path.display().to_string()));
+            }
+            let get = |name: &str| {
+                vars.iter()
+                    .find(|(var, _)| *var == name)
+                    .map(|(_, value)| value.clone())
+            };
+            let engine = Arc::new(FakeEngine::jev().scripted([Turn::Traced("e2e4")]));
+            let mut app = build_app(
+                Options::default(),
+                engine,
+                Graphics::off(CellSize::DEFAULT),
+                get,
+            );
+            assert!(app.debug_mode(), "RCHESS_DEBUG=1 without --debug");
+            let run = drive(
+                &mut app,
+                &AtomicI32::new(0),
+                vec![Step::Events(chars("3")), Step::AwaitEngine, Step::Signal],
+            );
+            run.result.expect("loop ends cleanly");
+            assert_eq!(uci_moves(app.game()), ["e2e4"]);
+            app.close_debug_log(Duration::from_secs(10));
+
+            let log = std::fs::read_to_string(expected).expect("log written");
+            let lines: Vec<&str> = log.lines().collect();
+            assert_eq!(lines.len(), 1, "{log}");
+            let line: serde_json::Value = serde_json::from_str(lines[0]).expect("JSON line");
+            assert_eq!(line["played"], "e4");
+            assert_eq!(line["stale"], false);
+            #[cfg(unix)]
+            {
+                use std::os::unix::fs::PermissionsExt;
+                let mode = |path: &std::path::Path| {
+                    std::fs::metadata(path)
+                        .expect("exists")
+                        .permissions()
+                        .mode()
+                        & 0o777
+                };
+                assert_eq!(mode(expected), 0o600, "{}", expected.display());
+                assert_eq!(mode(expected.parent().expect("folder")), 0o700);
+                assert_eq!(mode(&state), 0o700, "missing folders are made private");
+            }
+        }
+        assert!(!home.exists(), "HOME is the last resort");
+    }
+
     #[test]
     fn a_human_vs_human_session_plays_e4_and_quits_after_confirmation() {
         let mut app = new_app();
diff --git a/tests/pty_smoke.py b/tests/pty_smoke.py
index 68dd7820f6e36a8cdf6bbb1fa796705e92c18aa7..6813c139260bbe57d62edf5a63f0f6582a739db3 100644
--- a/tests/pty_smoke.py
+++ b/tests/pty_smoke.py
@@ -1,5 +1,5 @@
 #!/usr/bin/env python3
-"""Pseudo-terminal smoke test for the rchess TUI (spec 6.6/6.7, contract A7).
+"""Pseudo-terminal smoke test for the rchess TUI (spec 6.6/6.7 and 9.3/9.4).
 
 Not run by cargo: `python3 tests/pty_smoke.py [--release] [--no-build]`.
 
@@ -9,11 +9,29 @@ nothing touches the network), drives it with keystrokes or signals, and checks:
 
 * setup writes ?1049h, then ?1000h ?1002h ?1006h (click-and-drag mouse, SGR)
   and ?2004h (bracketed paste), and never ?1003h (any-motion) or ?1015h;
+* RCHESS_IMAGES=off is set unless a scenario is about the graphics query, so
+  the query is skipped and none of its bytes are written;
 * the menu and the game render (a tiny terminal emulator rebuilds the screen);
 * `1`, `/e4<Enter>`, `Esc`, `q`, `y` plays 1. e4 and quits after confirmation,
   and `qh5<Enter>` typed on the board does not quit (the question defaults to No);
 * `3` (play Black against the computer) gets a first move from the engine
   thread, and without a key the screen calls the computer "Local search";
+  without debug mode `d` says so;
+* --debug (and RCHESS_DEBUG=1) against the local search: the Status border says
+  DEBUG, `d` shows an exchange view with "no Jev requests yet", and no debug log
+  is created, since no Jev request was made;
+* the graphics query (spec 9.3), on a pty that
+  - never answers: the query is written once, in raw mode, between ?1049h and
+    ?1000h; start-up goes on after about 1 s with a menu warning, keys typed
+    afterwards work, half-block pictures are in the `g` cycle, and the terminal
+    is restored;
+  - answers late, while the menu is up: the answer does not act as key presses
+    (its `3` would start a game);
+  - gets SIGTERM or hangs up while the query waits: the process ends by that
+    signal at once, SIGTERM with the terminal restored;
+  - answers like Kitty: start-up goes on at once, the answer is not echoed,
+    Image is the starting style and pieces are kitty pictures (unicode
+    placeholders);
 * exit writes ?1006l ?1002l ?1000l ?2004l and then ?1049l, then only shows the
   cursor again (?25h) and draws nothing on the main screen, exits with status 0,
   and leaves the pty's termios exactly as it was before the program started
@@ -61,6 +79,17 @@ SETUP = [b"\x1b[?1049h", b"\x1b[?1000h", b"\x1b[?1002h", b"\x1b[?1006h", b"\x1b[
 TEARDOWN = [b"\x1b[?1006l", b"\x1b[?1002l", b"\x1b[?1000l", b"\x1b[?2004l", b"\x1b[?1049l"]
 FORBIDDEN = [b"\x1b[?1003h", b"\x1b[?1015h"]
 SHOW_CURSOR = b"\x1b[?25h"
+# The graphics query (ratatui-image's Parser::query): the kitty probe comes first
+# and the status report request ends it.
+QUERY_START = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"
+QUERY_END = b"\x1b[5n"
+QUERY_PARTS = [QUERY_START, b"\x1b[c", b"\x1b[16t", QUERY_END]
+# What Kitty answers: graphics OK, device attributes without sixel, a 9x18 pixel
+# cell and the status report.
+KITTY_ANSWER = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c\x1b[6;18;9t\x1b[0n"
+QUERY_WARNING = "graphics query: no answer within 1 s"
+# Kitty's unicode placeholder: every cell of a kitty picture holds one.
+PLACEHOLDER = "\U0010EEEE"
 SECRET_VARS = ("JEV_API_KEY", "TYPESAFE_API_KEY")
 
 failures = []
@@ -93,8 +122,10 @@ CSI = re.compile(r"\x1b\[([0-9;?]*)[ -/]*([@-~])")
 class Screen:
     """Replays the byte stream onto a grid: cursor moves, clears and printable text.
 
-    Colours are ignored. Zero-width characters (such as U+FE0E after the pawn) join
-    the previous cell. Only the alternate screen is kept.
+    Colours are ignored. Zero-width characters (such as U+FE0E after the pawn, or
+    the diacritics after a kitty placeholder) join the previous cell. Kitty and
+    sixel pictures (APC and DCS strings) are skipped; their placeholder cells are
+    text. Only the alternate screen is kept.
     """
 
     def __init__(self, text):
@@ -166,6 +197,12 @@ class Screen:
                     if not ends:
                         return
                     i = min(ends) + (1 if text[min(ends)] == "\x07" else 2)
+                elif text[i + 1 : i + 2] in ("_", "P", "^", "X"):
+                    # APC (kitty graphics), DCS (sixel), PM and SOS run to ST.
+                    end = text.find("\x1b\\", i + 2)
+                    if end < 0:
+                        return
+                    i = end + 2
                 else:
                     i += 2
                 continue
@@ -198,6 +235,8 @@ HOME = tempfile.TemporaryDirectory(prefix="rchess-pty-home-")
 
 
 def child_env(extra=None):
+    """The child's whole environment. Images are off, so the graphics query is
+    skipped, unless `extra` maps RCHESS_IMAGES to None (None removes a variable)."""
     env = {
         "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
         "HOME": HOME.name,
@@ -205,12 +244,21 @@ def child_env(extra=None):
         "LANG": "en_US.UTF-8",
         "LC_ALL": "en_US.UTF-8",
         "COLORTERM": "truecolor",
+        "RCHESS_IMAGES": "off",
     }
-    env.update(extra or {})
+    for name, value in (extra or {}).items():
+        if value is None:
+            env.pop(name, None)
+        else:
+            env[name] = value
     assert not any(v in env for v in SECRET_VARS)
     return env
 
 
+# The environment for the scenarios that ask the terminal about graphics.
+QUERY_ENV = {"RCHESS_IMAGES": None}
+
+
 class App:
     """The binary running on its own pty, with everything it wrote recorded."""
 
@@ -328,7 +376,9 @@ class App:
 # ----- checks shared by the scenarios -----
 
 
-def check_setup(app):
+def check_setup(app, query=False):
+    """Checks the setup sequences; with `query`, that the graphics query is written
+    once, between ?1049h and ?1000h, and otherwise that none of it is written."""
     check(app.wait_bytes(SETUP[-1]), "setup sequences arrive")
     offsets = ordered(app.stream, SETUP)
     check(
@@ -336,6 +386,17 @@ def check_setup(app):
         "setup order: ?1049h, ?1000h, ?1002h, ?1006h, ?2004h",
         f"offsets {offsets}",
     )
+    if query:
+        offsets = ordered(app.stream, SETUP[:1] + QUERY_PARTS + SETUP[1:2])
+        check(
+            offsets is not None,
+            "graphics query between ?1049h and ?1000h, status report last",
+            f"offsets {offsets}",
+        )
+        check(app.stream.count(QUERY_START) == 1, "the query is written once")
+    else:
+        sent = [part for part in QUERY_PARTS if part in app.stream]
+        check(not sent, "images off: no graphics query", f"found {sent}")
 
 
 def check_teardown(app, after, label):
@@ -429,7 +490,13 @@ def scenario_computer_opens(binary):
             "Jev" not in text.replace("JEV_API_KEY", ""),
             "without a key the screen never says Jev",
         )
+        check("DEBUG" not in text, "no DEBUG tag without debug mode")
         app.screen().show("computer opened")
+        app.send(b"d")
+        check(
+            app.wait_screen("debug mode is off (start with"),
+            "d says debug mode is off",
+        )
         app.send(b"q")
         check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
         quit_at = len(app.stream)
@@ -622,6 +689,222 @@ def scenario_arguments(binary):
         app.close()
 
 
+def scenario_debug(binary, via_env):
+    how = "RCHESS_DEBUG=1" if via_env else "--debug"
+    print(f"scenario: {how} against the local search: DEBUG tag, an empty exchange view, no log")
+    state = tempfile.TemporaryDirectory(prefix="rchess-pty-debug-")
+    log = os.path.join(state.name, "logs", "jev.jsonl")
+    env = {"RCHESS_DEBUG_LOG": log}
+    if via_env:
+        env["RCHESS_DEBUG"] = "1"
+    app = App(binary, args=[] if via_env else ["--debug"], env=env)
+    try:
+        check_setup(app)
+        check(app.wait_screen("1. Human vs Human"), "menu renders")
+        app.send(b"3", settle=0)
+        check(app.wait_screen("Black to move", timeout=10.0), "the computer played White's first move")
+        check("DEBUG" in app.screen().text(), "the Status border says DEBUG")
+        app.send(b"d")
+        check(app.wait_screen("no Jev requests yet"), "d opens the exchange view, which has nothing yet")
+        app.screen().show("exchange view")
+        app.send(b"\x1b", settle=0.3)
+        text = app.screen().text()
+        check(
+            "no Jev requests yet" not in text and "Black to move" in text,
+            "Esc goes back to the board",
+        )
+        app.send(b"q")
+        check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
+        quit_at = len(app.stream)
+        app.send(b"y", settle=0)
+        check(app.wait_exit(), "process exits after y")
+        check(app.status == 0, "exit status 0", f"status {app.status}")
+        check_teardown(app, quit_at, f"quit in {how}")
+        check(
+            not os.path.exists(os.path.dirname(log)),
+            "no Jev request, so no debug log (not even its folder)",
+        )
+        check(
+            not os.path.exists(os.path.join(HOME.name, ".local")),
+            "nothing under ~/.local/state either",
+        )
+    finally:
+        app.close()
+        state.cleanup()
+
+
+def wait_for_query(app):
+    """Waits for the whole graphics query; the time it arrived, or None."""
+    if not app.wait_bytes(QUERY_END):
+        return None
+    return time.monotonic()
+
+
+def check_no_query_text(app, label):
+    """The rebuilt screen shows no piece of the query or of an answer as text."""
+    text = app.screen().text()
+    parts = ("Gi=31", "AAAA", "[16t", "[5n", "i=31;OK", "62;c", "6;18;9t", "[0n")
+    shown = [part for part in parts if part in text]
+    check(not shown, f"{label}: no query or answer text on screen", f"found {shown}")
+
+
+def scenario_query_unanswered(binary):
+    print("scenario: the graphics query on a pty that never answers; start-up goes on after 1 s")
+    app = App(binary, env=QUERY_ENV)
+    try:
+        asked = wait_for_query(app)
+        check(asked is not None, "the graphics query is written")
+        lflag = termios.tcgetattr(app.master)[3]
+        check(
+            not lflag & termios.ECHO and not lflag & termios.ICANON,
+            "the query is written in raw mode, so no answer would be echoed",
+        )
+        check(app.wait_bytes(SETUP[1]), "mouse capture follows the query")
+        if asked is not None:
+            waited = time.monotonic() - asked
+            check(0.9 <= waited <= 2.0, "start-up goes on after about 1 s", f"{waited:.2f}s")
+        check_setup(app, query=True)
+        check(app.wait_screen("1. Human vs Human"), "menu renders")
+        check(QUERY_WARNING in app.screen().text(), "the menu warns that the query got no answer")
+        check_no_query_text(app, "menu")
+        app.screen().show("menu after an unanswered query")
+        app.send(b"1")
+        check(app.wait_screen("White to move"), "keys typed after start-up work (1 starts a game)")
+        app.send(b"g")
+        check(app.wait_screen("glyphs: outline"), "the style was Solid (g goes on to Outline)")
+        app.send(b"gg")
+        check(app.wait_screen("glyphs: image"), "Image is still in the g cycle")
+        text = app.screen().text()
+        check("▀" in text or "▄" in text, "pieces are drawn with half-blocks")
+        app.screen().show("half-block pictures")
+        app.send(b"q")
+        check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
+        quit_at = len(app.stream)
+        app.send(b"y", settle=0)
+        check(app.wait_exit(), "process exits after y")
+        check(app.status == 0, "exit status 0", f"status {app.status}")
+        check_teardown(app, quit_at, "quit after an unanswered query")
+    finally:
+        app.close()
+
+
+def scenario_query_late_answer(binary):
+    print("scenario: the terminal answers the graphics query after the deadline, on the menu")
+    app = App(binary, env=QUERY_ENV)
+    try:
+        asked = wait_for_query(app)
+        check(asked is not None, "the graphics query is written")
+        check(app.wait_screen(QUERY_WARNING), "the menu is up, with the query warning")
+        if asked is not None:
+            app.idle(max(0.0, asked + 1.3 - time.monotonic()))
+        # Read as keys, the answer's `3` would start Human vs Jev as Black.
+        os.write(app.master, KITTY_ANSWER)
+        app.idle(1.0)
+        text = app.screen().text()
+        check(
+            "1. Human vs Human" in text and "to move" not in text,
+            "the late answer does not act as key presses",
+        )
+        check_no_query_text(app, "menu after the late answer")
+        quit_at = len(app.stream)
+        sent = time.monotonic()
+        app.send(b"q", settle=0)
+        check(app.wait_exit(), "q quits from the menu")
+        check(app.status == 0, "exit status 0", f"status {app.status}")
+        if app.exited_at is not None:
+            check(app.exited_at - sent < 1.0, "at once", f"{app.exited_at - sent:.2f}s")
+        check_teardown(app, quit_at, "quit after a late answer")
+    finally:
+        app.close()
+
+
+def scenario_query_signal(binary):
+    print("scenario: SIGTERM while the graphics query waits; restored at once, dies by SIGTERM")
+    app = App(binary, env=QUERY_ENV)
+    try:
+        check(wait_for_query(app) is not None, "the graphics query is written")
+        app.idle(0.2)
+        signal_at = len(app.stream)
+        sent = time.monotonic()
+        os.kill(app.pid, signal.SIGTERM)
+        exited = app.wait_exit(timeout=3.0)
+        check(exited, "process exits after SIGTERM")
+        if exited:
+            took = app.exited_at - sent
+            check(app.status == -signal.SIGTERM, "terminated by SIGTERM", f"status {app.status}")
+            # The query checks for a quit signal every 50 ms instead of waiting out its 1 s.
+            check(took < 0.5, "without waiting for the query's deadline", f"{took:.2f}s")
+        check_teardown(app, signal_at, "SIGTERM during the query")
+    finally:
+        app.close()
+
+
+def scenario_query_hangup(binary):
+    print("scenario: the terminal hangs up while the graphics query waits; it dies by SIGHUP")
+    app = App(binary, env=QUERY_ENV)
+    try:
+        check(wait_for_query(app) is not None, "the graphics query is written")
+        app.idle(0.2)
+        for name in ("slave", "master"):
+            os.close(getattr(app, name))
+            setattr(app, name, None)
+        sent = time.monotonic()
+        status = None
+        while status is None and time.monotonic() < sent + 5.0:
+            pid, raw = os.waitpid(app.pid, os.WNOHANG)
+            if pid:
+                status = os.waitstatus_to_exitcode(raw)
+            else:
+                time.sleep(0.01)
+        app.status = status
+        check(status is not None, "process exits after the hangup")
+        if status is not None:
+            took = time.monotonic() - sent
+            check(status == -signal.SIGHUP, "terminated by SIGHUP", f"status {status}, {took:.3f}s")
+    finally:
+        app.close()
+
+
+def scenario_query_kitty(binary):
+    print("scenario: the pty answers the graphics query like Kitty; pieces are pictures")
+    app = App(binary, env=QUERY_ENV)
+    try:
+        check(wait_for_query(app) is not None, "the graphics query is written")
+        answered_at = len(app.stream)
+        answered = time.monotonic()
+        os.write(app.master, KITTY_ANSWER)
+        check(app.wait_bytes(SETUP[1]), "mouse capture follows the answer")
+        waited = time.monotonic() - answered
+        check(waited < 0.5, "start-up goes on as soon as the answer is complete", f"{waited:.2f}s")
+        check_setup(app, query=True)
+        check(app.wait_screen("1. Human vs Human"), "menu renders")
+        check(QUERY_WARNING not in app.screen().text(), "no query warning")
+        app.send(b"1")
+        check(app.wait_screen("White to move"), "Human vs Human starts")
+        check(
+            b"i=31;OK" not in app.stream and b"6;18;9t" not in app.stream,
+            "the answer is not echoed",
+        )
+        check(b"a=T,U=1" in app.stream[answered_at:], "pieces are sent as kitty pictures")
+        check_no_query_text(app, "board")
+        board = app.screen().text()
+        check(board.count(PLACEHOLDER) > 0, "the board shows the pictures' placeholder cells")
+        check("♜" not in board, "no solid glyphs while the pictures are shown")
+        app.screen().show("kitty pictures (placeholders show as their character)")
+        app.send(b"g")
+        check(app.wait_screen("glyphs: solid"), "the style was Image (g goes on to Solid)")
+        check("♜" in app.screen().text(), "then the pieces are solid glyphs")
+        app.send(b"q")
+        check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
+        quit_at = len(app.stream)
+        app.send(b"y", settle=0)
+        check(app.wait_exit(), "process exits after y")
+        check(app.status == 0, "exit status 0", f"status {app.status}")
+        check_teardown(app, quit_at, "quit after kitty pictures")
+    finally:
+        app.close()
+
+
 def scenario_help(binary):
     print("scenario: --help prints usage without touching the terminal")
     app = App(binary, args=["--help"])
@@ -698,10 +981,17 @@ def main():
     if not os.access(binary, os.X_OK):
         sys.exit(f"no binary at {binary}; build it first")
     print(f"binary: {binary}")
-    print(f"pty: {COLS}x{ROWS}, env without {' / '.join(SECRET_VARS)}")
+    print(f"pty: {COLS}x{ROWS}, env without {' / '.join(SECRET_VARS)}; RCHESS_IMAGES=off but for the query")
 
     scenario_play_and_quit(binary)
     scenario_computer_opens(binary)
+    for via_env in (False, True):
+        scenario_debug(binary, via_env)
+    scenario_query_unanswered(binary)
+    scenario_query_late_answer(binary)
+    scenario_query_signal(binary)
+    scenario_query_hangup(binary)
+    scenario_query_kitty(binary)
     for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
         scenario_signal(binary, signum)
     scenario_signal(binary, signal.SIGTERM, repeat=2)
````

- [ ] **Step: Run the tests to verify they pass**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::` and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test --lib engine::`
Expected: tui:: 417 passed, engine:: 110 passed; `find src -name '*.snap.new'` prints nothing.

Run: `cargo fmt --check` and `cargo clippy --all-targets 2>&1 | grep -A5 -E "src/tui|src/engine|src/main.rs"`
Expected: no output from either.

- [ ] **Step: Run the pseudo-terminal smoke test**

Run: `cargo build && env -u JEV_API_KEY -u TYPESAFE_API_KEY python3 tests/pty_smoke.py --no-build`
Expected: every scenario passes and the script prints `ALL CHECKS PASSED`.

- [ ] **Step: Update the handoff file**

Edit `docs/handoff/HANDOFF.md`. In `## Current`, set:

```
Sub-project: tui-polish | Plan: docs/superpowers/plans/2026-09-27-tui-polish.md
Branch: feat/tui-polish
Last completed task: tui-polish task 7 (Pseudo-terminal scenarios for the graphics query and debug mode)
Next task: tui-polish task 8 (review fixes)
State: green (except 3 pre-existing old-code test failures)
```

Set `## Verify before continuing` to:

```
env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui:: && cargo test --lib engine::
```

Add at the top of `## Log (newest first)`: `- <today's date> tui-polish task 7 done: Pseudo-terminal scenarios for the graphics query and debug mode`. Keep every other section. Record under `## Notes / decisions made during work` any decision this plan did not specify.

- [ ] **Step: Refresh the knowledge graph**

Run: `graphify update .`
Expected: completes without error. Never commit `graphify-out/`, `.claude/` or `.superpowers/`.

- [ ] **Step: Commit**

```bash
git add docs/handoff/HANDOFF.md src/tui/mod.rs tests/pty_smoke.py
git commit -m "test(tui): pty scenarios for the graphics query and debug mode

Co-Authored-By: <the model you are> <noreply@anthropic.com>"
```

Replace `<the model you are>` with your own model name.

---

### Task 8: Whole-branch review fixes

**Files:**
- Create: `src/tui/snapshots/chess__tui__panels__tests__jev_vs_jev_debug_60x20.snap`
- Modify: `src/engine/jev.rs`, `src/tui/app.rs`, `src/tui/board.rs`, `src/tui/debug.rs`, `src/tui/files.rs`, `src/tui/glyphs.rs`, `src/tui/graphics.rs`, `src/tui/mod.rs`, `src/tui/panels.rs`, `src/tui/pieces.rs`, `src/tui/terminal.rs`, `src/tui/test_support/harness.rs`, `src/tui/test_support/mod.rs`, `tests/pty_smoke.py`

**Interfaces:**
- Fixes found by the whole-prototype review and the controller's follow-up rounds, each with its test: font sizes outside 1..=256 px ignored and pictures capped at 4096 px; the font measured again after a resize for Sixel and iTerm2 (`Action::MeasureFont` or equivalent in the patch); half-block pictures only on squares of at least 11×5 (`MIN_HALFBLOCK_SQUARE`); pictures never under dialogs; each piece's picture checked against its own file; the Jev record checked against the wire and the key redacted when echoed as `\/`; `DEBUG` never pushes out the mode title; an existing debug log made private, `~` expanded in `RCHESS_DEBUG_LOG`; a log failure found on the menu kept until a game shows it; the exchange view follows its exchange when old ones are dropped; Kitty pictures deleted before leaving the alternate screen; the test environment reader shared through `test_support`.

- [ ] **Step: Confirm the starting point**

Run: `git status --short` (only untracked `.claude/`, `graphify-out/` and user files) and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::`
Expected: 417 passed (engine: `cargo test --lib engine::` 110 passed).

- [ ] **Step: Apply the fix patch (tests and fixes together)**

Extract the fenced block below into a file with a script (never retype it; the patch has exact whitespace and, in Task 1, binary image data), then from the repository root run `git apply --check <file> && git apply <file>`. If `--check` fails, stop and report: do not edit the patch by hand.

````diff
diff --git a/src/engine/jev.rs b/src/engine/jev.rs
index 0a650f261f26653c913da2931b44f85ab069bd4a..a1d3350744bfd2d9f281e3b189c5560f92984ef9 100644
--- a/src/engine/jev.rs
+++ b/src/engine/jev.rs
@@ -31,6 +31,10 @@ const MAX_RETRY_AFTER: Duration = Duration::from_secs(2);
 const ERROR_SNIPPET_CHARS: usize = 200;
 /// Largest response body read, in bytes; a real answer is a few kilobytes.
 const MAX_BODY_BYTES: u64 = 1 << 20;
+/// The header that carries the API key, as `Bearer <key>` ([`bearer`]).
+const AUTHORIZATION: &str = "Authorization";
+/// The header naming the body's type.
+const CONTENT_TYPE_HEADER: &str = "Content-Type";
 /// The request's `Content-Type`, as the TypeSafe quickstart sends it.
 const CONTENT_TYPE: &str = "application/json";
 /// What a recorded exchange shows in place of the API key.
@@ -278,9 +282,16 @@ fn redact(text: &str, secret: &str) -> String {
 /// A response body with the key redacted, both in the raw text and, when the body is
 /// JSON, in the text a parser decodes from it: a server can echo the key with `\u`
 /// escapes that the raw text does not match. When decoding shows the key, the body
-/// is re-encoded from the redacted JSON, so no later parse can bring it back.
+/// is re-encoded from the redacted JSON, so no later parse can bring it back. The
+/// raw text also loses the key with `/` written as `\/`, JSON's other way to write
+/// it, in case a reader decodes part of a body that is not JSON as a whole.
 fn redact_body(text: &str, secret: &str) -> String {
     let text = redact(text, secret);
+    let text = if secret.contains('/') {
+        redact(&text, &secret.replace('/', "\\/"))
+    } else {
+        text
+    };
     let Ok(json) = serde_json::from_str::<Value>(&text) else {
         return text;
     };
@@ -308,6 +319,12 @@ fn redact_value(value: &Value, secret: &str) -> Value {
     }
 }
 
+/// The `Authorization` value for `key`. [`JevClient`] sends it with the real key and
+/// records it with `<redacted>`, so the record shows what was sent.
+fn bearer(key: &str) -> String {
+    format!("Bearer {key}")
+}
+
 /// `error` with [`redact`] applied to its text.
 fn redact_error(error: JevError, secret: &str) -> JevError {
     match error {
@@ -400,8 +417,8 @@ impl JevClient {
             method: "POST".to_string(),
             url: self.url.clone(),
             headers: vec![
-                ("Authorization".to_string(), format!("Bearer {REDACTED}")),
-                ("Content-Type".to_string(), CONTENT_TYPE.to_string()),
+                (AUTHORIZATION.to_string(), bearer(REDACTED)),
+                (CONTENT_TYPE_HEADER.to_string(), CONTENT_TYPE.to_string()),
             ],
             body: redact_value(body, &self.api_key),
             attempts: Vec::new(),
@@ -450,7 +467,7 @@ impl JevClient {
         let sent = self
             .agent
             .post(&self.url)
-            .header("Authorization", &format!("Bearer {}", self.api_key))
+            .header(AUTHORIZATION, &bearer(&self.api_key))
             // `send_json` alone would send `application/json; charset=utf-8`; the
             // TypeSafe quickstart sends plain `application/json`.
             .content_type(CONTENT_TYPE)
@@ -944,12 +961,95 @@ pub(crate) mod tests {
             ok.elapsed
         );
 
-        // Only the record is redacted: the wire carries the real key.
+        // Only the record is redacted: the wire carries the real key. Every recorded
+        // header is one the server read, with the same value once the key is put back.
         for raw in requests.lock().unwrap().iter() {
             assert!(raw.contains("Bearer test-key"), "{raw}");
+            let head: Vec<(&str, &str)> = raw
+                .lines()
+                .take_while(|line| !line.is_empty())
+                .filter_map(|line| line.split_once(':'))
+                .map(|(name, value)| (name.trim(), value.trim()))
+                .collect();
+            for (name, value) in &exchange.headers {
+                let sent = value.replace(REDACTED, "test-key");
+                assert!(
+                    head.iter()
+                        .any(|(n, v)| n.eq_ignore_ascii_case(name) && *v == sent),
+                    "{name}: {sent} not in {raw}"
+                );
+            }
         }
     }
 
+    #[test]
+    fn traced_exchange_redacts_the_key_in_the_request_body() {
+        let ok = response("200 OK", "application/json", ANSWER);
+        let (client, requests) = serve_with_key(vec![ok], SENTINEL_KEY);
+        let mut asked = request();
+        asked.state = json!({ "note": format!("key {SENTINEL_KEY}") });
+        asked.guidance = format!("Never repeat {SENTINEL_KEY}.");
+        let mut trace = None;
+        client.choose_traced(&asked, &mut trace).unwrap();
+
+        let body = trace.unwrap().body;
+        assert_eq!(body["state"]["note"], "key <redacted>");
+        assert_eq!(
+            body["questions"]["move"]["instructions"]["guidance"],
+            "Never repeat <redacted>."
+        );
+        assert!(!body.to_string().contains(SENTINEL_KEY), "{body}");
+        // The wire carries the request as it was asked.
+        let raw = &requests.lock().unwrap()[0];
+        assert!(raw.contains(&format!("key {SENTINEL_KEY}")), "{raw}");
+        assert!(
+            raw.contains(&format!("Never repeat {SENTINEL_KEY}.")),
+            "{raw}"
+        );
+    }
+
+    #[test]
+    fn traced_exchange_redacts_a_key_with_an_escaped_solidus() {
+        // JSON may write `/` as `\/`: a parser reads it back as the key, though the raw
+        // text does not contain it. Text that is not JSON is redacted in that form too.
+        let key = "sentinel/key-7Qx9";
+        let escaped = key.replace('/', "\\/");
+        let (client, _requests) = serve_with_key(
+            vec![
+                response(
+                    "401 Unauthorized",
+                    "application/json",
+                    &format!(r#"{{"error":"invalid api key {escaped}"}}"#),
+                ),
+                response(
+                    "401 Unauthorized",
+                    "text/plain",
+                    &format!("no such key: {escaped} ("),
+                ),
+            ],
+            key,
+        );
+
+        let mut trace = None;
+        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
+        assert_eq!(
+            trace.unwrap().attempts[0].response.as_deref(),
+            Some(r#"{"error":"invalid api key <redacted>"}"#)
+        );
+        assert_eq!(
+            error.to_string(),
+            r#"HTTP 401: {"error":"invalid api key <redacted>"}"#
+        );
+
+        let mut trace = None;
+        let error = client.choose_traced(&request(), &mut trace).unwrap_err();
+        assert_eq!(
+            trace.unwrap().attempts[0].response.as_deref(),
+            Some("no such key: <redacted> (")
+        );
+        assert_eq!(error.to_string(), "HTTP 401: no such key: <redacted> (");
+    }
+
     #[test]
     fn traced_exchange_records_a_failed_connection_without_a_status() {
         // Nothing listens on the port once the listener is dropped.
@@ -1130,7 +1230,7 @@ pub(crate) mod tests {
     }
 
     #[test]
-    fn choose_traced_by_default_answers_without_a_trace() {
+    fn default_choose_traced_forwards_to_choose_and_records_nothing() {
         struct Plain;
         impl MoveChooser for Plain {
             fn choose(&self, _: &ChoiceRequest) -> Result<ChoiceAnswer, JevError> {
diff --git a/src/tui/app.rs b/src/tui/app.rs
index be8c213b16dd1fdfa2bb4a40e4ccb8cf3531ae7e..241d816fd26c530ad08bf67cad38ded7a4394c95 100644
--- a/src/tui/app.rs
+++ b/src/tui/app.rs
@@ -3,8 +3,9 @@
 //! [`App`] owns the game and all UI state. The run loop feeds it [`AppEvent`]s through
 //! [`App::handle`] and draws it with [`App::render`]; nothing else changes it. It never
 //! blocks and never spawns threads: when the computer should move, `handle` returns an
-//! [`Action::RequestEngine`] and the run loop starts the worker. The only I/O it does is
-//! writing the files the user asks it to save.
+//! [`Action::RequestEngine`] and the run loop starts the worker; when a resize may have
+//! zoomed the font, it returns an [`Action::MeasureFont`] and the run loop asks the
+//! terminal. The only I/O it does is writing the files the user asks it to save.
 //!
 //! Screens are [`Screen::Menu`], [`Screen::Playing`] and [`Screen::GameOver`] (an overlay on
 //! the playing screen), with a stack of [`Dialog`]s on top. Input goes to the topmost focus
@@ -56,13 +57,14 @@ use ratatui::crossterm::event::{
     Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
 };
 use ratatui::layout::{Position as CellPosition, Rect};
-use ratatui_image::picker::Picker;
+use ratatui_image::picker::{Picker, ProtocolType};
 
 use super::board::{BoardGeometry, CellSize, Highlights, PieceImages, square_at};
 use super::debug::{DebugLog, DebugSession, Exchange, ExchangeView, History};
 use super::event::AppEvent;
 use super::files::{SaveError, pgn_export, resolve_path, tilde_path, today, write_file};
 use super::glyphs::{self, GlyphSet, Palette};
+use super::graphics;
 use super::input::{Command, LineEditor, parse_command, unquote};
 use super::movetext::{MoveTextError, parse_move};
 use super::panels;
@@ -247,6 +249,12 @@ pub enum Action {
     /// every request must be answered exactly once, or the app waits forever (and counts
     /// it against [`MAX_IN_FLIGHT`]).
     RequestEngine(EngineRequest),
+    /// Ask the terminal for its font size again (`graphics::FontMeter::measure`) once the
+    /// batch is handled, and hand the result to [`App::font_measured`]. Returned for a
+    /// resize while pictures are Sixel or iTerm2, which are encoded at the pixel size
+    /// of their cells, and not again until the result is in, so a batch of resizes
+    /// asks once.
+    MeasureFont,
 }
 
 /// Which side a Human vs Jev game gives the person.
@@ -617,6 +625,8 @@ pub struct App {
     picker: Option<Picker>,
     /// The terminal's font size, which shapes the board's squares.
     cell_size: CellSize,
+    /// An [`Action::MeasureFont`] was returned and its result is not in yet.
+    measuring_font: bool,
     /// The board's piece pictures, kept between draws.
     piece_images: PieceImages,
     /// Debug mode's exchanges and log; `None` when debug mode is off.
@@ -699,6 +709,7 @@ impl App {
             glyphs,
             picker: None,
             cell_size: CellSize::DEFAULT,
+            measuring_font: false,
             piece_images: PieceImages::new(),
             debug: None,
             exchange_view: None,
@@ -752,9 +763,40 @@ impl App {
     }
 
     /// Sets the terminal's font size, which is known only once the terminal has been asked
-    /// (default [`CellSize::DEFAULT`]). The next draw shapes the board's squares for it.
+    /// (default [`CellSize::DEFAULT`]). The next draw shapes the board's squares for it,
+    /// and a picker for another font size is replaced by one for this size (same
+    /// protocol) and the pictures made for the old one are dropped, so pictures are
+    /// encoded for the cells they are drawn in.
     pub fn set_cell_size(&mut self, cell_size: CellSize) {
         self.cell_size = cell_size;
+        if let Some(picker) = &self.picker {
+            let font = picker.font_size();
+            if (font.width, font.height) != (cell_size.width(), cell_size.height()) {
+                self.picker = Some(graphics::picker_for(picker.protocol_type(), cell_size));
+                self.piece_images.clear();
+            }
+        }
+    }
+
+    /// The result of an [`Action::MeasureFont`]: the font size the terminal has now, or
+    /// `None` when it could not say (the size stays). Only a size other than the current
+    /// one changes anything ([`set_cell_size`](Self::set_cell_size)).
+    pub fn font_measured(&mut self, measured: Option<CellSize>) {
+        self.measuring_font = false;
+        if let Some(cell_size) = measured
+            && cell_size != self.cell_size
+        {
+            self.set_cell_size(cell_size);
+        }
+    }
+
+    /// True when the pictures are encoded at a pixel size (Sixel and iTerm2), so a font
+    /// zoom must be followed; Kitty placeholders and half-blocks scale with the cells.
+    fn pictures_follow_the_font(&self) -> bool {
+        matches!(
+            self.picker.as_ref().map(Picker::protocol_type),
+            Some(ProtocolType::Sixel | ProtocolType::Iterm2)
+        )
     }
 
     /// Turns debug mode on (spec 9.4): Jev exchanges that arrive with engine replies are
@@ -1126,6 +1168,7 @@ impl App {
     /// event was collected.
     #[must_use = "the run loop must execute the returned actions"]
     pub fn handle(&mut self, event: AppEvent, now: Instant) -> Vec<Action> {
+        let mut actions = Vec::new();
         match event {
             AppEvent::Term(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                 if !self.too_small {
@@ -1136,15 +1179,20 @@ impl App {
             }
             AppEvent::Term(Event::Mouse(mouse)) if !self.too_small => self.on_mouse(mouse),
             AppEvent::Term(Event::Paste(text)) if !self.too_small => self.on_paste(&text),
+            AppEvent::Term(Event::Resize(..)) => {
+                // A resize may be a font zoom, which only the terminal can tell.
+                if self.pictures_follow_the_font() && !self.measuring_font {
+                    self.measuring_font = true;
+                    actions.push(Action::MeasureFont);
+                }
+            }
             AppEvent::Term(_) | AppEvent::Tick => {}
             AppEvent::Engine(reply) => self.on_engine(reply, now),
         }
         self.report_log_failure();
         self.release_held(now);
-        self.engine_request(now)
-            .map(Action::RequestEngine)
-            .into_iter()
-            .collect()
+        actions.extend(self.engine_request(now).map(Action::RequestEngine));
+        actions
     }
 
     fn on_key(&mut self, key: KeyEvent) {
@@ -1648,14 +1696,19 @@ impl App {
         let (Some(debug), Some(exchange)) = (&mut self.debug, exchange) else {
             return;
         };
-        let number = debug.record(*exchange, stale, SystemTime::now());
+        debug.record(*exchange, stale, SystemTime::now());
         if let Some(view) = &mut self.exchange_view {
-            view.follow(number);
+            view.follow(debug.history());
         }
     }
 
-    /// Shows the debug log's failure, the one time it is reported.
+    /// Shows the debug log's failure, the one time it is reported. On the menu, which has
+    /// no status line (and starting a game clears it), the failure waits in the log until
+    /// a game is shown.
     fn report_log_failure(&mut self) {
+        if self.screen == Screen::Menu {
+            return;
+        }
         let Some(failure) = self.debug.as_mut().and_then(DebugSession::log_failure) else {
             return;
         };
@@ -2488,13 +2541,13 @@ mod tests {
     use std::sync::mpsc;
 
     use ratatui::style::Modifier;
-    use ratatui_image::picker::ProtocolType;
 
     use super::*;
     use crate::core::{Piece, Position as ChessPosition, START_FEN};
     use crate::engine::EngineConfig;
     use crate::engine::{MoveSource, analyse, recorded_exchange};
     use crate::tui::board::square_rect;
+    use crate::tui::debug::NO_LOG_PATH;
     use crate::tui::graphics;
     use crate::tui::panels::HELP_LINES;
     use crate::tui::test_support::engine::{
@@ -3356,10 +3409,133 @@ mod tests {
         assert_eq!(h.app.status_line(), "glyphs: image");
     }
 
+    /// A font size and protocol as the picker reports them.
+    fn picker_format(app: &App) -> Option<(ProtocolType, (u16, u16))> {
+        app.picker().map(|picker| {
+            let font = picker.font_size();
+            (picker.protocol_type(), (font.width, font.height))
+        })
+    }
+
+    /// A game on an app whose picker draws with `protocol` for a `font` the terminal
+    /// reported.
+    fn picture_app(protocol: ProtocolType, font: CellSize) -> Harness {
+        let picker = graphics::picker_for(protocol, font);
+        let mut h = Harness::build(FakeEngine::local(), (80, 24), Vec::new(), |app| {
+            app.with_picker(Some(picker))
+        });
+        h.app.set_cell_size(font);
+        h.char('1');
+        h
+    }
+
+    /// Resizes the terminal to `columns`×`rows` and sends the resize.
+    fn resize(h: &mut Harness, (columns, rows): (u16, u16)) -> Vec<Action> {
+        h.terminal.backend_mut().resize(columns, rows);
+        h.send(AppEvent::Term(Event::Resize(columns, rows)))
+    }
+
+    /// How many font measurements `actions` ask for.
+    fn measurements(actions: &[Action]) -> usize {
+        actions
+            .iter()
+            .filter(|action| matches!(action, Action::MeasureFont))
+            .count()
+    }
+
+    #[test]
+    fn a_resize_measures_the_font_only_for_sixel_and_iterm2_pictures() {
+        for protocol in [ProtocolType::Sixel, ProtocolType::Iterm2] {
+            let mut h = picture_app(protocol, CellSize::DEFAULT);
+            assert_eq!(measurements(&resize(&mut h, (100, 30))), 1, "{protocol:?}");
+        }
+        // Kitty placeholders and half-blocks scale with the cells.
+        for protocol in [ProtocolType::Kitty, ProtocolType::Halfblocks] {
+            let mut h = picture_app(protocol, CellSize::DEFAULT);
+            assert_eq!(measurements(&resize(&mut h, (100, 30))), 0, "{protocol:?}");
+        }
+        // Images off: no picker, nothing to follow.
+        let mut h = Harness::new();
+        h.char('1');
+        assert_eq!(measurements(&resize(&mut h, (100, 30))), 0);
+        // Only a resize asks.
+        let mut h = picture_app(ProtocolType::Sixel, CellSize::DEFAULT);
+        let mut actions = h.tick();
+        actions.extend(h.char('g'));
+        actions.extend(h.command("e4"));
+        assert_eq!(measurements(&actions), 0);
+    }
+
+    #[test]
+    fn a_batch_of_resizes_measures_the_font_once() {
+        let mut h = picture_app(ProtocolType::Sixel, CellSize::DEFAULT);
+        let mut asked = 0;
+        for size in [(100, 30), (90, 26), (120, 40), (120, 40)] {
+            asked += measurements(&resize(&mut h, size));
+        }
+        asked += measurements(&h.tick());
+        assert_eq!(asked, 1, "one measurement until its result is in");
+        h.app.font_measured(None);
+        assert_eq!(
+            measurements(&resize(&mut h, (80, 24))),
+            1,
+            "the next resize measures again"
+        );
+    }
+
+    #[test]
+    fn measuring_the_font_it_already_has_changes_nothing() {
+        let mut h = picture_app(ProtocolType::Sixel, CellSize::DEFAULT);
+        for _ in 0..3 {
+            h.char('g');
+        }
+        assert_eq!(h.app.glyphs(), GlyphSet::Image);
+        let pictures = h.app.piece_images().len();
+        assert!(pictures > 0, "the pieces are pictures");
+        for measured in [Some(CellSize::DEFAULT), None] {
+            let _ = resize(&mut h, (80, 24));
+            h.app.font_measured(measured);
+            assert_eq!(h.app.cell_size(), CellSize::DEFAULT, "{measured:?}");
+            assert_eq!(picker_format(&h.app), Some((ProtocolType::Sixel, (10, 20))));
+            assert_eq!(
+                h.app.piece_images().len(),
+                pictures,
+                "the pictures are kept"
+            );
+        }
+    }
+
+    #[test]
+    fn a_new_font_rebuilds_the_picker_and_drops_the_pictures() {
+        let mut h = picture_app(ProtocolType::Sixel, CellSize::DEFAULT);
+        for _ in 0..3 {
+            h.char('g');
+        }
+        assert!(!h.app.piece_images().is_empty());
+        // A font zoom: the window keeps its pixels and gets more cells, now 8×16 each.
+        assert_eq!(measurements(&resize(&mut h, (100, 30))), 1);
+        h.app.font_measured(Some(CellSize::new(8, 16)));
+        assert_eq!(h.app.cell_size(), CellSize::new(8, 16));
+        assert_eq!(
+            picker_format(&h.app),
+            Some((ProtocolType::Sixel, (8, 16))),
+            "the pictures are encoded for the new font"
+        );
+        assert!(h.app.piece_images().is_empty(), "the old pictures are gone");
+        h.draw();
+        assert!(!h.app.piece_images().is_empty(), "and drawn again");
+
+        // Without images the font size still shapes the squares.
+        let mut h = Harness::new();
+        h.app.font_measured(Some(CellSize::new(8, 16)));
+        assert_eq!(h.app.cell_size(), CellSize::new(8, 16));
+        assert_eq!(picker_format(&h.app), None, "images stay off");
+    }
+
     #[test]
     fn the_image_style_draws_pictures_and_keeps_them_between_draws() {
         let picker = graphics::picker_for(ProtocolType::Halfblocks, CellSize::DEFAULT);
-        let mut h = Harness::build(FakeEngine::local(), (120, 40), Vec::new(), |app| {
+        let mut h = Harness::build(FakeEngine::local(), (200, 60), Vec::new(), |app| {
             app.with_picker(Some(picker))
         });
         h.char('1');
@@ -3401,7 +3577,7 @@ mod tests {
         assert_eq!(h.app.piece_images().len(), 22);
 
         // A bigger terminal gets bigger squares: only what is on the board now is built.
-        h.terminal.backend_mut().resize(200, 60);
+        h.terminal.backend_mut().resize(300, 100);
         h.draw();
         assert_eq!(h.app.piece_images().len(), 21);
         assert!(!e1(&h).contains(king));
@@ -4596,7 +4772,11 @@ mod tests {
     /// An 80×24 Human vs Jev game (the person plays White) in debug mode, with a log that
     /// has no file (it reports that at the first exchange).
     fn debug_game() -> Harness {
-        let mut h = debug_app(FakeEngine::jev(), (80, 24), DebugLog::open(None));
+        let mut h = debug_app(
+            FakeEngine::jev(),
+            (80, 24),
+            DebugLog::open(Err(NO_LOG_PATH.to_string())),
+        );
         h.char('2');
         h
     }
@@ -4727,7 +4907,11 @@ mod tests {
 
     #[test]
     fn an_answer_held_while_paused_is_kept_when_played_or_dropped() {
-        let mut h = debug_app(FakeEngine::jev(), (80, 24), DebugLog::open(None));
+        let mut h = debug_app(
+            FakeEngine::jev(),
+            (80, 24),
+            DebugLog::open(Err(NO_LOG_PATH.to_string())),
+        );
         h.char('5');
         h.reply("e2e4");
         h.at_ms(5_000);
@@ -4740,7 +4924,11 @@ mod tests {
         assert_eq!(h.uci(), Vec::<String>::new());
         assert_eq!(kept(&h), [("e5".to_string(), true)], "undo dropped it");
 
-        let mut h = debug_app(FakeEngine::jev(), (80, 24), DebugLog::open(None));
+        let mut h = debug_app(
+            FakeEngine::jev(),
+            (80, 24),
+            DebugLog::open(Err(NO_LOG_PATH.to_string())),
+        );
         h.char('5');
         h.reply("e2e4");
         h.at_ms(5_000);
@@ -4767,7 +4955,11 @@ mod tests {
 
     #[test]
     fn replies_are_played_under_the_view_and_it_stays_on_its_exchange() {
-        let mut h = debug_app(FakeEngine::jev(), (80, 24), DebugLog::open(None));
+        let mut h = debug_app(
+            FakeEngine::jev(),
+            (80, 24),
+            DebugLog::open(Err(NO_LOG_PATH.to_string())),
+        );
         h.char('5');
         h.char('d');
         h.reply_traced("e2e4");
@@ -4893,7 +5085,11 @@ mod tests {
         let mut h = Harness::sized(FakeEngine::jev(), 120, 40);
         h.char('2');
         assert!(!top_row(&h).contains("DEBUG"));
-        let mut h = debug_app(FakeEngine::jev(), (120, 40), DebugLog::open(None));
+        let mut h = debug_app(
+            FakeEngine::jev(),
+            (120, 40),
+            DebugLog::open(Err(NO_LOG_PATH.to_string())),
+        );
         h.char('2');
         let row = top_row(&h);
         assert!(row.contains("┌ Status ─ DEBUG ─"), "{row}");
@@ -4967,6 +5163,26 @@ mod tests {
         );
     }
 
+    #[test]
+    fn a_log_failure_found_on_the_menu_waits_for_the_next_game() {
+        let mut h = debug_game();
+        h.moves(&["e4"]);
+        h.char('m');
+        h.char('y');
+        assert_eq!(h.app.screen(), Screen::Menu);
+        // Jev's answer comes on the menu: stale, but kept and logged, and the log fails.
+        h.reply_traced("e7e5");
+        h.tick();
+        assert_eq!(h.app.exchanges().map(History::len), Some(1));
+        assert_eq!(h.app.status_line(), "", "the menu shows no status line");
+        h.char('2');
+        assert_eq!(h.app.screen(), Screen::Playing);
+        assert_eq!(
+            h.app.status_line(),
+            "debug log disabled: no log file (set RCHESS_DEBUG_LOG, XDG_STATE_HOME or HOME)"
+        );
+    }
+
     #[test]
     fn the_api_key_never_reaches_the_screen_or_the_log() {
         let dir = TempDir::new("debug-app");
diff --git a/src/tui/board.rs b/src/tui/board.rs
index 8ce50bd1d630822fa361d98bc190b2ac9d0a76a0..e3d17af1bf848903254948e46bcb3ac502fa2c12 100644
--- a/src/tui/board.rs
+++ b/src/tui/board.rs
@@ -16,8 +16,9 @@
 //! composited onto the square's colour
 //! ([`composite`](super::pieces::composite)) and drawn by the terminal's
 //! graphics protocol through a ratatui-image [`Picker`], in the square's
-//! [`image_area`]. Squares smaller than [`MIN_IMAGE_SQUARE`] show the Solid
-//! glyph instead. [`PieceImages`] keeps the encoded pictures between frames.
+//! [`image_area`]. Squares smaller than [`min_picture_square`] for the protocol
+//! ([`MIN_IMAGE_SQUARE`], or [`MIN_HALFBLOCK_SQUARE`] for half blocks) show the
+//! Solid glyph instead. [`PieceImages`] keeps the encoded pictures between frames.
 
 use std::fmt;
 
@@ -42,10 +43,25 @@ use crate::core::{Color as Side, Piece, PieceKind, Position as ChessPosition, Sq
 pub const MIN_SQUARE: (u16, u16) = (3, 1);
 
 /// The smallest square `(width, height)` in cells that shows a piece as a picture
-/// in the [`GlyphSet::Image`] style; smaller squares show the Solid glyph. Its
-/// [`image_area`] is 3×2 cells.
+/// in the [`GlyphSet::Image`] style with a pixel protocol (Kitty, iTerm2, Sixel);
+/// smaller squares show the Solid glyph. Its [`image_area`] is 3×2 cells.
 pub const MIN_IMAGE_SQUARE: (u16, u16) = (5, 2);
 
+/// The smallest square `(width, height)` in cells that shows a piece as a picture
+/// drawn in half blocks, which have two pixels per cell: below an [`image_area`] of
+/// 9×5 cells the pieces cannot be told apart, so smaller squares show the Solid
+/// glyph.
+pub const MIN_HALFBLOCK_SQUARE: (u16, u16) = (11, 5);
+
+/// The smallest square that shows a picture drawn with `protocol`:
+/// [`MIN_HALFBLOCK_SQUARE`] for half blocks, else [`MIN_IMAGE_SQUARE`].
+pub fn min_picture_square(protocol: ProtocolType) -> (u16, u16) {
+    match protocol {
+        ProtocolType::Halfblocks => MIN_HALFBLOCK_SQUARE,
+        ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2 => MIN_IMAGE_SQUARE,
+    }
+}
+
 /// A terminal cell's size in pixels, which is the font size. It makes squares
 /// look square: see [`square_width`].
 #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
@@ -204,6 +220,11 @@ pub fn image_area(square: Rect) -> Option<Rect> {
         .then(|| Rect::new(square.x + 1, square.y, square.width - 2, square.height))
 }
 
+/// The largest picture built, in pixels per side. Real image areas are a few hundred
+/// pixels; a bigger one could only come from a bogus font size, and would cost
+/// seconds and hundreds of megabytes on the UI thread.
+const MAX_PICTURE_PX: u32 = 4096;
+
 /// Piece pictures kept between frames for the [`GlyphSet::Image`] style: the
 /// ratatui-image [`Protocol`] a picker made of each
 /// [`composite`](super::pieces::composite), one per [`ImageKey`], so a picture is
@@ -248,8 +269,15 @@ impl PieceImages {
         self.cache.is_empty()
     }
 
+    /// Drops every picture, as when the font changed.
+    pub fn clear(&mut self) {
+        self.cache.clear();
+        self.made_for = None;
+    }
+
     /// The picture of `piece` on `background` (RGB) for an image area of `cells`,
-    /// drawn by `picker`: made on first use, `None` when the picker cannot encode it.
+    /// drawn by `picker`: made on first use, `None` when the picker cannot encode it
+    /// or it would be over [`MAX_PICTURE_PX`] on a side.
     fn picture(
         &mut self,
         picker: &Picker,
@@ -275,6 +303,9 @@ impl PieceImages {
             u32::from(cells.width) * u32::from(font.width),
             u32::from(cells.height) * u32::from(font.height),
         );
+        if key.width_px > MAX_PICTURE_PX || key.height_px > MAX_PICTURE_PX {
+            return None;
+        }
         self.cache
             .get_or_insert_with(key, |key| {
                 let image = DynamicImage::ImageRgba8(key.composite());
@@ -457,7 +488,7 @@ impl BoardView<'_> {
 
     /// The picture of `piece` on the square at `rect`, whose background is `bg`, and
     /// the area it goes in. Only in the Image style with a picker, on a square of at
-    /// least [`MIN_IMAGE_SQUARE`] whose image area lies wholly inside `clip` and meets
+    /// least [`min_picture_square`] for its protocol whose image area lies wholly inside `clip` and meets
     /// none of the [`overlays`](Self::overlays), and on a background with a known RGB
     /// value ([`glyphs::xterm_rgb`]); `None` otherwise, and the square shows the glyph.
     fn picture<'i>(
@@ -472,6 +503,10 @@ impl BoardView<'_> {
             return None;
         }
         let picker = self.picker?;
+        let (min_width, min_height) = min_picture_square(picker.protocol_type());
+        if rect.width < min_width || rect.height < min_height {
+            return None;
+        }
         let area = image_area(rect).filter(|&area| {
             clip.intersection(area) == area
                 && !self.overlays.iter().any(|overlay| overlay.intersects(area))
@@ -1607,7 +1642,7 @@ mod tests {
     fn pictures_are_kept_and_reused_on_squares_of_the_same_colour() {
         let mut images = PieceImages::new();
         // White's king on dark e1, Black's on light e8, a white pawn on light a2.
-        let mut scene = Scene::new(57, 25, "4k3/8/8/8/8/8/P7/4K3 w - - 0 1");
+        let mut scene = Scene::new(89, 41, "4k3/8/8/8/8/8/P7/4K3 w - - 0 1");
         let picker = scene.picker.clone().expect("a picker");
         scene.draw(&mut images);
         assert_eq!(images.len(), 3);
@@ -1634,7 +1669,7 @@ mod tests {
 
     #[test]
     fn each_highlight_colour_gets_its_own_picture() {
-        let mut scene = Scene::new(57, 25, ROOK_CHECK);
+        let mut scene = Scene::new(89, 41, ROOK_CHECK);
         let picker = scene.picker.clone().expect("a picker");
         let pal = scene.palette;
         let mut images = PieceImages::new();
@@ -1695,18 +1730,74 @@ mod tests {
         assert_eq!(images.len(), 7);
     }
 
+    #[test]
+    fn half_blocks_need_bigger_squares_than_pixel_protocols() {
+        assert_eq!(MIN_IMAGE_SQUARE, (5, 2));
+        assert_eq!(MIN_HALFBLOCK_SQUARE, (11, 5));
+        assert_eq!(
+            image_area(Rect::new(0, 0, 11, 5)).map(|area| area.as_size()),
+            Some(Size::new(9, 5))
+        );
+        assert_eq!(
+            min_picture_square(ProtocolType::Halfblocks),
+            MIN_HALFBLOCK_SQUARE
+        );
+        for protocol in [
+            ProtocolType::Kitty,
+            ProtocolType::Iterm2,
+            ProtocolType::Sixel,
+        ] {
+            assert_eq!(
+                min_picture_square(protocol),
+                MIN_IMAGE_SQUARE,
+                "{protocol:?}"
+            );
+        }
+    }
+
     #[test]
     fn squares_too_small_for_a_picture_show_the_solid_glyph() {
         let glyph = glyphs::glyph(GlyphSet::Solid, WHITE_KING);
-        // (terminal, font the squares are shaped for, square size)
+        // (terminal, font the squares are shaped for, square size, protocol)
         let cases = [
-            ((31, 11), CellSize::DEFAULT, (3, 1)),
-            ((41, 9), CellSize::new(5, 20), (5, 1)),
-            ((25, 17), CellSize::new(12, 12), (3, 2)),
+            ((31, 11), CellSize::DEFAULT, (3, 1), ProtocolType::Sixel),
+            ((41, 9), CellSize::new(5, 20), (5, 1), ProtocolType::Kitty),
+            (
+                (25, 17),
+                CellSize::new(12, 12),
+                (3, 2),
+                ProtocolType::Iterm2,
+            ),
+            // Half blocks: 7×3 and 9×4 squares, and one short of 11×5 either way.
+            (
+                (57, 25),
+                CellSize::DEFAULT,
+                (7, 3),
+                ProtocolType::Halfblocks,
+            ),
+            (
+                (73, 33),
+                CellSize::DEFAULT,
+                (9, 4),
+                ProtocolType::Halfblocks,
+            ),
+            (
+                (89, 33),
+                CellSize::new(8, 21),
+                (11, 4),
+                ProtocolType::Halfblocks,
+            ),
+            (
+                (73, 41),
+                CellSize::new(10, 17),
+                (9, 5),
+                ProtocolType::Halfblocks,
+            ),
         ];
-        for ((width, height), cell, size) in cases {
+        for ((width, height), cell, size, protocol) in cases {
             let mut scene = Scene::new(width, height, KINGS_AND_KNIGHT);
             scene.cell = cell;
+            scene.picker = Some(picker_for(protocol, cell));
             let mut images = PieceImages::new();
             let (terminal, g) = scene.draw(&mut images);
             assert_eq!((g.square_w, g.square_h), size);
@@ -1714,16 +1805,19 @@ mod tests {
             assert_eq!(
                 (e1.symbol(), e1.fg),
                 (glyph, scene.palette.white_piece),
-                "{size:?}"
+                "{size:?} {protocol:?}"
+            );
+            assert!(
+                images.is_empty(),
+                "{size:?} {protocol:?}: a picture was built"
             );
-            assert!(images.is_empty(), "{size:?}: a picture was built");
         }
 
-        // From 5×2 on, pictures.
-        let mut scene = Scene::new(41, 17, KINGS_AND_KNIGHT);
+        // Half blocks from 11×5 on, pictures.
+        let mut scene = Scene::new(89, 41, KINGS_AND_KNIGHT);
         let mut images = PieceImages::new();
         let (terminal, g) = scene.draw(&mut images);
-        assert_eq!((g.square_w, g.square_h), MIN_IMAGE_SQUARE);
+        assert_eq!((g.square_w, g.square_h), MIN_HALFBLOCK_SQUARE);
         assert_eq!(images.len(), 3);
         let e1 = glyph_cell(square_rect(&g, Square::E1));
         assert_ne!(terminal.backend().buffer()[e1].symbol(), glyph);
@@ -1734,17 +1828,42 @@ mod tests {
         let (terminal, _) = scene.draw(&mut images);
         assert_eq!(terminal.backend().buffer()[e1].symbol(), glyph);
         assert!(images.is_empty());
+
+        // The pixel protocols from 5×2 on.
+        let mut scene = Scene::new(41, 17, KINGS_AND_KNIGHT);
+        scene.picker = Some(picker_for(ProtocolType::Sixel, CellSize::DEFAULT));
+        let mut images = PieceImages::new();
+        let (terminal, g) = scene.draw(&mut images);
+        assert_eq!((g.square_w, g.square_h), MIN_IMAGE_SQUARE);
+        assert_eq!(images.len(), 3);
+        let e1 = glyph_cell(square_rect(&g, Square::E1));
+        assert_ne!(terminal.backend().buffer()[e1].symbol(), glyph);
+    }
+
+    #[test]
+    fn a_picture_too_large_to_build_is_drawn_as_the_glyph() {
+        // No real font gives an image area over 4096 pixels on a side; building one
+        // would take seconds and hundreds of megabytes on the UI thread.
+        let picker = halfblocks(CellSize::new(256, 256));
+        let mut images = PieceImages::new();
+        let too_wide = images.picture(&picker, WHITE_KING, [0, 0, 0], Size::new(17, 1));
+        assert!(too_wide.is_none(), "17 cells of 256 pixels");
+        let too_tall = images.picture(&picker, WHITE_KING, [0, 0, 0], Size::new(1, 17));
+        assert!(too_tall.is_none(), "17 rows of 256 pixels");
+        assert!(images.is_empty(), "nothing was built");
+        let widest = images.picture(&picker, WHITE_KING, [0, 0, 0], Size::new(16, 1));
+        assert!(widest.is_some(), "4096 pixels are built");
     }
 
     #[test]
     fn another_square_size_or_font_replaces_the_pictures() {
         let mut images = PieceImages::new();
-        let mut scene = Scene::new(41, 17, KINGS_AND_KNIGHT);
+        let mut scene = Scene::new(89, 41, KINGS_AND_KNIGHT);
         scene.draw(&mut images);
         assert_eq!(images.len(), 3);
 
         // Bigger squares: the small pictures are dropped, not kept beside the new ones.
-        scene.size = (57, 25);
+        scene.size = (105, 49);
         scene.draw(&mut images);
         assert_eq!(images.len(), 3);
 
@@ -1762,7 +1881,7 @@ mod tests {
 
     #[test]
     fn the_cursor_and_the_no_colour_marks_stay_beside_a_picture() {
-        let mut scene = Scene::new(57, 25, ROOK_CHECK);
+        let mut scene = Scene::new(89, 41, ROOK_CHECK);
         let picker = scene.picker.clone().expect("a picker");
         let pal = scene.palette;
         let rook = Piece::new(Side::Black, PieceKind::Rook);
@@ -1874,17 +1993,17 @@ mod tests {
 
     #[test]
     fn a_picture_the_area_cuts_off_is_drawn_as_the_glyph() {
-        let scene = Scene::new(57, 25, KINGS_AND_KNIGHT);
-        let full = Rect::new(0, 0, 57, 25);
+        let scene = Scene::new(89, 41, KINGS_AND_KNIGHT);
+        let full = Rect::new(0, 0, 89, 41);
         let g = layout_board(full, false, CellSize::DEFAULT).expect("fits");
-        // Rank 1 takes rows 21 to 23; the area stops after row 22.
+        // Rank 1 takes rows 35 to 39; the area stops after row 38.
         let e1 = square_rect(&g, Square::E1);
-        assert_eq!((e1.y, e1.height), (21, 3));
+        assert_eq!((e1.y, e1.height), (35, 5));
         let mut buf = Buffer::empty(full);
         let mut images = PieceImages::new();
         ratatui::widgets::StatefulWidget::render(
             scene.view(g),
-            Rect::new(0, 0, 57, 23),
+            Rect::new(0, 0, 89, 39),
             &mut buf,
             &mut images,
         );
@@ -1896,16 +2015,16 @@ mod tests {
                 scene.palette.white_piece
             )
         );
-        assert_eq!(buf[(e1.x, 23)], Cell::EMPTY);
+        assert_eq!(buf[(e1.x, 39)], Cell::EMPTY);
         // Only Black's king, on rank 8, has its whole image area inside.
         assert_eq!(images.len(), 1);
     }
 
     #[test]
     fn a_piece_under_a_box_shows_its_glyph_until_the_box_closes() {
-        let mut scene = Scene::new(57, 25, KINGS_AND_KNIGHT);
+        let mut scene = Scene::new(89, 41, KINGS_AND_KNIGHT);
         let picker = scene.picker.clone().expect("a picker");
-        let full = Rect::new(0, 0, 57, 25);
+        let full = Rect::new(0, 0, 89, 41);
         let g = layout_board(full, false, CellSize::DEFAULT).expect("fits");
         let (e1, g1) = (square_rect(&g, Square::E1), square_rect(&g, sq("g1")));
         // A box over one cell of g1's picture, its last one, and nothing of e1's.
diff --git a/src/tui/debug.rs b/src/tui/debug.rs
index 943c12922240ce39f9a84599f9dadf81fdd9c9ad..74a3d020eff25e8843d31e26dab6a3456e3f78b8 100644
--- a/src/tui/debug.rs
+++ b/src/tui/debug.rs
@@ -28,7 +28,7 @@ use std::time::{Duration, SystemTime, UNIX_EPOCH};
 use serde::{Serialize, Serializer};
 use serde_json::Value;
 
-use super::files::{civil_date, describe};
+use super::files::{civil_date, describe, expand_tilde};
 use super::glyphs::char_width;
 use crate::core::Game;
 use crate::engine::{ComputerMove, JevExchange};
@@ -48,8 +48,8 @@ const LOG_THREAD: &str = "debug-log";
 const LOG_DIR: &str = "rchess";
 /// The log's file name.
 const LOG_FILE: &str = "jev-debug.jsonl";
-/// Why there is no log when no path could be worked out.
-const NO_LOG_PATH: &str = "no log file (set RCHESS_DEBUG_LOG, XDG_STATE_HOME or HOME)";
+/// Why there is no log when none of the variables that give its path is set.
+pub const NO_LOG_PATH: &str = "no log file (set RCHESS_DEBUG_LOG, XDG_STATE_HOME or HOME)";
 
 /// True when debug mode is on: `--debug` was given (`flag`), or `RCHESS_DEBUG` is set
 /// to anything but empty or `0` (surrounding whitespace ignored).
@@ -62,20 +62,29 @@ pub fn enabled(flag: bool, get: impl Fn(&str) -> Option<String>) -> bool {
 /// The debug log file: `RCHESS_DEBUG_LOG` when set, else
 /// `$XDG_STATE_HOME/rchess/jev-debug.jsonl`, else `~/.local/state/rchess/jev-debug.jsonl`
 /// (on macOS too). Empty variables count as unset, and so does a relative
-/// `XDG_STATE_HOME` (the XDG base directory rules say to ignore one). `None` when none
-/// of the three is set.
+/// `XDG_STATE_HOME` (the XDG base directory rules say to ignore one). A leading `~/` in
+/// `RCHESS_DEBUG_LOG` is `HOME`, as in save paths ([`expand_tilde`]); a relative path
+/// stays relative to the working directory.
+///
+/// # Errors
+///
+/// Why there is no path, for [`DebugLog::open`] to report: [`NO_LOG_PATH`] when none of
+/// the three is set, or the reason `~` cannot be expanded when `RCHESS_DEBUG_LOG`
+/// starts with `~/` and `HOME` is not set.
 ///
 /// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
-pub fn log_path(get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
+pub fn log_path(get: impl Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
     let set = |name: &str| get(name).filter(|value| !value.is_empty());
     if let Some(path) = set(DEBUG_LOG_ENV) {
-        return Some(PathBuf::from(path));
+        return expand_tilde(&path, set("HOME").as_deref().map(Path::new))
+            .map_err(|error| format!("{DEBUG_LOG_ENV}: {error}"));
     }
     let state = set("XDG_STATE_HOME")
         .map(PathBuf::from)
         .filter(|dir| dir.is_absolute())
-        .or_else(|| set("HOME").map(|home| Path::new(&home).join(".local").join("state")))?;
-    Some(state.join(LOG_DIR).join(LOG_FILE))
+        .or_else(|| set("HOME").map(|home| Path::new(&home).join(".local").join("state")))
+        .ok_or_else(|| NO_LOG_PATH.to_string())?;
+    Ok(state.join(LOG_DIR).join(LOG_FILE))
 }
 
 /// `time` in UTC as RFC 3339 with milliseconds: `2026-09-27T14:03:05.120Z`. Times before
@@ -416,11 +425,15 @@ impl ExchangeView {
             .or_else(|| (!history.is_empty()).then_some(0))
     }
 
-    /// A new exchange numbered `number` has arrived: shown when nothing is, otherwise
-    /// the view stays where it is.
-    pub fn follow(&mut self, number: u64) {
-        if self.shown.is_none() {
-            self.show(Some(number));
+    /// A new exchange has arrived in `history`: the newest is shown when nothing is.
+    /// Otherwise the view stays on its exchange, scroll and all, while that is kept; once
+    /// it has been dropped, the view moves to the oldest one kept, at its top.
+    pub fn follow(&mut self, history: &History) {
+        let kept = self.shown.and_then(|number| history.index_of(number));
+        match (self.shown, kept) {
+            (None, _) => self.show(history.last().map(|record| record.number)),
+            (Some(_), Some(_)) => {}
+            (Some(_), None) => self.show(history.get(0).map(|record| record.number)),
         }
     }
 
@@ -574,7 +587,7 @@ pub struct LogFailure {
 /// The debug log: records go over a channel to the `debug-log` thread, which appends
 /// each as a line ([`log_line`]) and flushes it. The file and any missing folders are
 /// created with the first record: folders with mode 0700, the file with mode 0600 (an
-/// existing file is appended to and keeps its mode).
+/// existing file is appended to and made 0600 too; an existing folder keeps its mode).
 ///
 /// The first error ends the thread; [`DebugLog::failure`] then reports it once and
 /// nothing more is written. Records never wait: [`DebugLog::write`] only sends.
@@ -598,17 +611,14 @@ enum LogState {
 }
 
 impl DebugLog {
-    /// The log at `path` ([`log_path`]); without one, a log that reports the missing
-    /// path at the first record.
+    /// The log at `path` ([`log_path`]); without one, a log that reports why there is
+    /// no path (the error) at the first record.
     #[must_use]
-    pub fn open(path: Option<PathBuf>) -> DebugLog {
+    pub fn open(path: Result<PathBuf, String>) -> DebugLog {
         match path {
-            Some(path) => DebugLog::start(path),
-            None => DebugLog {
-                state: LogState::Unavailable(LogFailure {
-                    reason: NO_LOG_PATH.to_string(),
-                    path: None,
-                }),
+            Ok(path) => DebugLog::start(path),
+            Err(reason) => DebugLog {
+                state: LogState::Unavailable(LogFailure { reason, path: None }),
             },
         }
     }
@@ -715,8 +725,11 @@ fn write_records(path: &Path, queued: &Receiver<Record>) -> Result<(), LogFailur
     Ok(())
 }
 
-/// Opens `path` for appending, creating it (mode 0600) and its missing folders (mode
-/// 0700) as needed.
+/// Opens `path` for appending, creating it and its missing folders (mode 0700) as
+/// needed. The file gets mode 0600, also when it was already there with a looser one;
+/// folders that were already there keep theirs (one may be the user's
+/// `~/.local/state`). Only a regular file is changed: a device such as `/dev/null`
+/// is written as it is.
 fn open_log(path: &Path) -> io::Result<File> {
     if let Some(folder) = path
         .parent()
@@ -732,7 +745,16 @@ fn open_log(path: &Path) -> io::Result<File> {
     options.append(true).create(true);
     #[cfg(unix)]
     std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
-    options.open(path)
+    let file = options.open(path)?;
+    #[cfg(unix)]
+    {
+        use std::os::unix::fs::PermissionsExt;
+        let metadata = file.metadata()?;
+        if metadata.is_file() && metadata.permissions().mode() & 0o777 != 0o600 {
+            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
+        }
+    }
+    Ok(file)
 }
 
 /// Writes `record` as one line and flushes it.
@@ -790,7 +812,6 @@ impl DebugSession {
 
 #[cfg(test)]
 mod tests {
-    use std::collections::HashMap;
     use std::fs;
     use std::time::Instant;
 
@@ -798,16 +819,8 @@ mod tests {
 
     use super::*;
     use crate::engine::{JevAttempt, recorded_exchange};
-    use crate::tui::test_support::TempDir;
     use crate::tui::test_support::engine::{SENTINEL_KEY, jev_exchange, jev_move};
-
-    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
-        let map: HashMap<String, String> = pairs
-            .iter()
-            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
-            .collect();
-        move |key| map.get(key).cloned()
-    }
+    use crate::tui::test_support::{TempDir, env};
 
     /// `ms` milliseconds after the epoch.
     fn at_ms(ms: u64) -> SystemTime {
@@ -871,7 +884,7 @@ mod tests {
 
     #[test]
     fn the_log_path_follows_the_variables_in_order() {
-        let log = |pairs: &[(&str, &str)]| log_path(env(pairs));
+        let log = |pairs: &[(&str, &str)]| log_path(env(pairs)).ok();
         let all = [
             (DEBUG_LOG_ENV, "/tmp/mine.jsonl"),
             ("XDG_STATE_HOME", "/xdg/state"),
@@ -907,8 +920,13 @@ mod tests {
                 "XDG_STATE_HOME {xdg:?} is ignored"
             );
         }
-        assert_eq!(log(&[]), None);
-        assert_eq!(log(&[("HOME", ""), ("XDG_STATE_HOME", "state")]), None);
+        for pairs in [&[][..], &[("HOME", ""), ("XDG_STATE_HOME", "state")]] {
+            assert_eq!(
+                log_path(env(pairs)),
+                Err(NO_LOG_PATH.to_string()),
+                "{pairs:?}"
+            );
+        }
     }
 
     #[test]
@@ -1104,12 +1122,12 @@ mod tests {
         view.newer(&history);
         assert_eq!(view.shown, None);
 
-        let first = history.push(exchange(), false, at_ms(1)).number;
-        view.follow(first);
+        history.push(exchange(), false, at_ms(1));
+        view.follow(&history);
         assert_eq!(view.shown, Some(1), "the first exchange is shown at once");
         for n in 2..=3 {
-            let number = history.push(exchange(), false, at_ms(n)).number;
-            view.follow(number);
+            history.push(exchange(), false, at_ms(n));
+            view.follow(&history);
         }
         assert_eq!(view.shown, Some(1), "later ones do not move the view");
         view.newer(&history);
@@ -1127,14 +1145,39 @@ mod tests {
     fn a_dropped_exchange_leaves_the_view_on_the_oldest() {
         let mut history = History::new();
         history.push(exchange(), false, at_ms(1));
+        history.push(exchange(), false, at_ms(2));
         let mut view = ExchangeView::open(&history);
-        for n in 2..=51 {
+        view.older(&history);
+        (view.page, view.max_scroll) = (10, 40);
+        view.down(7);
+        assert_eq!((view.shown, view.scroll), (Some(1), 7));
+        // While the exchange shown is kept, new ones leave the view and its scroll alone.
+        for n in 3..=50 {
             history.push(exchange(), false, at_ms(n));
+            view.follow(&history);
         }
+        assert_eq!((view.shown, view.scroll), (Some(1), 7));
+        // Once it is dropped, the view moves to the oldest kept, at its top.
+        history.push(exchange(), false, at_ms(51));
+        view.follow(&history);
         assert_eq!(history.index_of(1), None);
+        assert_eq!((view.shown, view.scroll), (Some(2), 0));
+        assert_eq!(view.index(&history), Some(0));
+        view.down(3);
+        history.push(exchange(), false, at_ms(52));
+        view.follow(&history);
+        assert_eq!((view.shown, view.scroll), (Some(3), 0), "and again");
+        view.newer(&history);
+        assert_eq!(view.shown, Some(4));
+
+        // A view that missed the drop (no follow) still shows the oldest kept.
+        let mut view = ExchangeView {
+            shown: Some(1),
+            ..ExchangeView::default()
+        };
         assert_eq!(view.index(&history), Some(0));
         view.newer(&history);
-        assert_eq!(view.shown, Some(3));
+        assert_eq!(view.shown, Some(4));
     }
 
     #[test]
@@ -1291,6 +1334,54 @@ mod tests {
 
     // ----- the log thread -----
 
+    #[test]
+    fn a_leading_tilde_in_the_log_path_is_the_home_folder() {
+        let dir = TempDir::new("debug-log");
+        let home = dir.path().to_str().expect("a UTF-8 temp dir");
+        let path = log_path(env(&[(DEBUG_LOG_ENV, "~/x.jsonl"), ("HOME", home)]));
+        assert_eq!(path, Ok(dir.join("x.jsonl")));
+        let mut log = DebugLog::open(path);
+        log.write(&record(false));
+        log.close(Duration::from_secs(10));
+        assert_eq!(
+            fs::read_to_string(dir.join("x.jsonl"))
+                .unwrap()
+                .lines()
+                .count(),
+            1
+        );
+        assert!(!dir.join("~").exists());
+
+        // Only a leading `~/` is the home folder.
+        let log = |raw: &str| log_path(env(&[(DEBUG_LOG_ENV, raw), ("HOME", "/home/ana")]));
+        assert_eq!(log("~/a/b.jsonl"), Ok(PathBuf::from("/home/ana/a/b.jsonl")));
+        assert_eq!(log("logs/~/b.jsonl"), Ok(PathBuf::from("logs/~/b.jsonl")));
+        assert_eq!(log("~ana/b.jsonl"), Ok(PathBuf::from("~ana/b.jsonl")));
+    }
+
+    #[test]
+    fn a_tilde_without_a_home_is_reported_as_such() {
+        // Without a home there is nothing to expand `~` to, so there is no log, and the
+        // reason says so rather than asking for RCHESS_DEBUG_LOG, which is set.
+        for pairs in [
+            &[(DEBUG_LOG_ENV, "~/x.jsonl")][..],
+            &[(DEBUG_LOG_ENV, "~/x.jsonl"), ("HOME", "")],
+            &[
+                (DEBUG_LOG_ENV, "~/x.jsonl"),
+                ("XDG_STATE_HOME", "/xdg/state"),
+            ],
+        ] {
+            let mut log = DebugLog::open(log_path(env(pairs)));
+            log.write(&record(false));
+            let failure = log.failure().expect("reported");
+            assert_eq!(
+                failure.reason, "RCHESS_DEBUG_LOG: cannot expand ~ in ~/x.jsonl: HOME is not set",
+                "{pairs:?}"
+            );
+            assert_eq!(failure.path, None);
+        }
+    }
+
     #[test]
     fn the_log_is_created_private_and_gets_one_line_per_record() {
         let dir = TempDir::new("debug-log");
@@ -1332,6 +1423,27 @@ mod tests {
         assert!(text.starts_with("{\"earlier\":true}\n{\"time\""));
     }
 
+    #[test]
+    #[cfg(unix)]
+    fn an_existing_log_is_made_private_and_its_folder_is_left_alone() {
+        use std::os::unix::fs::PermissionsExt;
+        let dir = TempDir::new("debug-log");
+        // A folder the user already had, such as ~/.local/state, and a log another
+        // program left readable by everyone.
+        let folder = dir.join("state");
+        fs::create_dir(&folder).unwrap();
+        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
+        let path = folder.join(LOG_FILE);
+        fs::write(&path, "{\"earlier\":true}\n").unwrap();
+        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
+        let mut log = DebugLog::start(path.clone());
+        log.write(&record(false));
+        log.close(Duration::from_secs(10));
+        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
+        assert_eq!(mode(&path), 0o600);
+        assert_eq!(mode(&folder), 0o755);
+    }
+
     #[test]
     fn a_write_error_is_reported_once_and_ends_the_log() {
         let dir = TempDir::new("debug-log");
@@ -1355,7 +1467,7 @@ mod tests {
 
     #[test]
     fn a_missing_path_is_reported_at_the_first_record() {
-        let mut log = DebugLog::open(None);
+        let mut log = DebugLog::open(Err(NO_LOG_PATH.to_string()));
         assert_eq!(log.failure(), None);
         log.write(&record(false));
         let failure = log.failure().expect("reported");
diff --git a/src/tui/files.rs b/src/tui/files.rs
index a8901295f6b2faa9d981bed9a809da16219643c4..da0d6034550c0e4b985e96988fd5e02bcc140fc9 100644
--- a/src/tui/files.rs
+++ b/src/tui/files.rs
@@ -49,15 +49,7 @@ pub fn resolve_path(raw: &str, ext: &str, home: Option<&Path>) -> Result<PathBuf
         return Err(format!("{raw} is a folder; add a file name"));
     }
 
-    let mut path = match raw.strip_prefix('~') {
-        Some(rest) if rest.starts_with(is_separator) => {
-            let home = home
-                .filter(|home| !home.as_os_str().is_empty())
-                .ok_or_else(|| format!("cannot expand ~ in {raw}: HOME is not set"))?;
-            home.join(rest.trim_start_matches(is_separator))
-        }
-        _ => PathBuf::from(raw),
-    };
+    let mut path = expand_tilde(raw, home)?;
 
     let ext = ext.trim_start_matches('.');
     let has_ext = Path::new(name)
@@ -74,6 +66,26 @@ pub fn resolve_path(raw: &str, ext: &str, home: Option<&Path>) -> Result<PathBuf
     Ok(path)
 }
 
+/// `raw` with a leading `~` followed by a path separator replaced by `home`, as
+/// [`resolve_path`] does; `~name` and a `~` later in the path are left alone. Relative
+/// paths stay relative to the working directory.
+///
+/// # Errors
+///
+/// A message for the status line when `raw` starts with `~/` while `home` is `None` or
+/// empty.
+pub fn expand_tilde(raw: &str, home: Option<&Path>) -> Result<PathBuf, String> {
+    match raw.strip_prefix('~') {
+        Some(rest) if rest.starts_with(is_separator) => {
+            let home = home
+                .filter(|home| !home.as_os_str().is_empty())
+                .ok_or_else(|| format!("cannot expand ~ in {raw}: HOME is not set"))?;
+            Ok(home.join(rest.trim_start_matches(is_separator)))
+        }
+        _ => Ok(PathBuf::from(raw)),
+    }
+}
+
 /// Why [`write_file`] did not write.
 #[derive(Clone, Debug, PartialEq, Eq)]
 pub enum SaveError {
diff --git a/src/tui/glyphs.rs b/src/tui/glyphs.rs
index 15c7ed80c434b8a6eeae645a6c076fea7f8f02b9..a7b0599dc7d6abc79a8cf63d4400b1497589359d 100644
--- a/src/tui/glyphs.rs
+++ b/src/tui/glyphs.rs
@@ -398,11 +398,13 @@ pub fn initial_glyphs(
         }
     });
 
-    // `Off` with `image` named first means one of the two switches is on.
+    // Name the switch that turned images off, if one did.
     let off_because = if no_color(&get) {
-        "NO_COLOR is set"
+        " (NO_COLOR is set)"
+    } else if images_off(&get) {
+        " (RCHESS_IMAGES=off)"
     } else {
-        "RCHESS_IMAGES=off"
+        ""
     };
     let warnings = refused
         .into_iter()
@@ -413,7 +415,7 @@ pub fn initial_glyphs(
                 shorten(&value)
             ),
             Refused::ImagesOff => {
-                format!("{source}: images are off ({off_because}); using {set}")
+                format!("{source}: images are off{off_because}; using {set}")
             }
         })
         .collect();
@@ -474,11 +476,10 @@ pub fn shorten(text: &str) -> String {
 
 #[cfg(test)]
 mod tests {
-    use std::collections::HashMap;
-
     use ratatui::{buffer::Buffer, layout::Rect, style::Style, text::Span};
 
     use super::*;
+    use crate::tui::test_support::env;
 
     fn all_pieces() -> impl Iterator<Item = Piece> {
         Side::ALL
@@ -489,14 +490,6 @@ mod tests {
     /// No graphics query ran: the text styles only.
     const OFF: ImageSupport = ImageSupport::Off;
 
-    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
-        let map: HashMap<String, String> = pairs
-            .iter()
-            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
-            .collect();
-        move |key| map.get(key).cloned()
-    }
-
     /// Asserts `s` is one cell wide by ratatui's measure and in a real buffer.
     fn assert_one_cell(s: &str) {
         assert_eq!(Span::raw(s).width(), 1, "{s:?} is not one cell wide");
@@ -958,6 +951,14 @@ mod tests {
             warnings,
             ["--glyphs: images are off (RCHESS_IMAGES=off); using ascii"]
         );
+        // Off for another reason (no terminal was asked): no switch is blamed.
+        assert_eq!(
+            initial_glyphs(Some("image"), env(&[]), OFF),
+            (
+                GlyphSet::Solid,
+                vec!["--glyphs: images are off; using solid".to_string()]
+            )
+        );
     }
 
     #[test]
diff --git a/src/tui/graphics.rs b/src/tui/graphics.rs
index 3882b496f978471f8260f9aaaeed2a2b95b9f3ad..f58340dc9f0930942bc75b36e5fbdbf11233bdac 100644
--- a/src/tui/graphics.rs
+++ b/src/tui/graphics.rs
@@ -1,5 +1,8 @@
 //! Graphics detection (spec 9.3): which image protocol the terminal speaks and how
-//! large its font is, found out once at start-up.
+//! large its font is, found out once at start-up. After that only the font size can
+//! change (a font zoom), and it matters only to Sixel and iTerm2 pictures, which are
+//! encoded at a pixel size: after a resize the run loop asks the terminal for it again
+//! ([`FontMeter::measure`]), the same way.
 //!
 //! [`detect`] writes ratatui-image's capability query ([`Parser::query`]) and reads the
 //! answers itself, on the UI thread: it polls stdin with a deadline of
@@ -43,6 +46,10 @@ const POLL_SLICE: Duration = Duration::from_millis(50);
 /// kept out of the input ([`LateAnswers`]).
 const LATE_ANSWER_WINDOW: Duration = Duration::from_secs(10);
 
+/// The largest font size believed, in pixels per side of a cell. A bigger one is a
+/// bogus answer, and would make every picture huge.
+const MAX_CELL_PX: u16 = 256;
+
 /// The most key presses a late kitty answer is taken to have after its Alt+`_`;
 /// Kitty's answer has 8 (`Gi=31;OK`), an error answer a few dozen.
 const MAX_ANSWER_KEYS: usize = 128;
@@ -185,13 +192,137 @@ fn query_text(is_tmux: bool, get: impl Fn(&str) -> Option<String>) -> String {
     Parser::query(is_tmux, options)
 }
 
+/// Asks the terminal for its font size again after resizes ([`FontMeter::measure`]),
+/// and remembers what the measurements that gave up waiting still owe.
+///
+/// A terminal answers in order. On a slow link the answers of a measurement that gave
+/// up may still be on their way when the next one is asked, so that one reads them
+/// first: it skips one status report (and the cell size before it) for each
+/// measurement that gave up in the last 10 s, and takes the answer after them, its own.
+/// When they do not come (crossterm read them in between), it takes the last answer it
+/// read once the deadline has passed.
+#[derive(Clone, Debug, Default)]
+pub struct FontMeter {
+    /// How many measurements gave up since the last one that got an answer: the
+    /// status reports the terminal may still send ahead of the next one's.
+    owed: usize,
+    /// Until when those may still come (10 s after the last one gave up); later they
+    /// are taken to be read by crossterm or lost.
+    until: Option<Instant>,
+}
+
+impl FontMeter {
+    /// Asks the terminal for its font size again, after a resize: a font zoom changes
+    /// the cells, not the window, and Sixel and iTerm2 pictures are encoded at the
+    /// pixel size of the cells they fill. Call it on the UI thread between batches of
+    /// events.
+    ///
+    /// It writes the cell-size query, the status request and the device attributes
+    /// request (`ESC [ 16 t`, `ESC [ 5 n`, `ESC [ c`, wrapped for tmux like the
+    /// start-up query) and reads the answers as [`detect`] does, up to its own status
+    /// report (see [`FontMeter`]), taking at most [`QUERY_TIMEOUT`], less once `stop`
+    /// returns true. The size is the cell-size answer, else the window's pixels per
+    /// cell, else `None`: keep the current one ([`measured_font`]).
+    ///
+    /// The answers need no [`LateAnswers`]: all three are CSI sequences that crossterm
+    /// cannot take for a key. It drops the cell size and status reports (ending in `t`
+    /// and `n`) and keeps the device attributes to itself. But crossterm's reader
+    /// reads stdin until its bytes make an event, so dropped answers alone would leave
+    /// it blocked until the next key, click or paste, and the screen would not be
+    /// redrawn until then. The device attributes answer comes after them and makes
+    /// that event, so whatever answers reach crossterm (this measurement's own device
+    /// attributes, or late answers), they end with one. Bytes that arrive while the
+    /// measurement reads (keys typed in those milliseconds) are fed to the answer
+    /// parser and lost.
+    pub fn measure(&mut self, is_tmux: bool, stop: impl Fn() -> bool) -> Option<CellSize> {
+        let answers = match write_query(&font_query_text(is_tmux)) {
+            Ok(()) => self.read(read_stdin_byte, QUERY_TIMEOUT, stop),
+            Err(error) => Err(error.into()),
+        };
+        measured_font(answers, window_cell_size())
+    }
+
+    /// Reads the answers to a measurement with [`read_answers_after`], skipping those
+    /// still owed, and keeps count.
+    fn read(
+        &mut self,
+        read_byte: impl FnMut(Duration) -> io::Result<Option<u8>>,
+        timeout: Duration,
+        stop: impl Fn() -> bool,
+    ) -> Result<Vec<Response>, QueryError> {
+        let owed = self.owed_at(Instant::now());
+        let answers = read_answers_after(read_byte, timeout, stop, owed);
+        self.settle(owed, &answers, Instant::now());
+        answers
+    }
+
+    /// The status reports still owed at `now`.
+    fn owed_at(&self, now: Instant) -> usize {
+        match self.until {
+            Some(until) if now <= until => self.owed,
+            _ => 0,
+        }
+    }
+
+    /// Counts a measurement that was owed `owed` status reports and read `answers`,
+    /// at `now`: one that got an answer settles every debt, one that gave up adds its
+    /// own.
+    fn settle(&mut self, owed: usize, answers: &Result<Vec<Response>, QueryError>, now: Instant) {
+        if answers.is_ok() {
+            self.owed = 0;
+            self.until = None;
+        } else {
+            self.owed = owed + 1;
+            self.until = Some(now + LATE_ANSWER_WINDOW);
+        }
+    }
+}
+
+/// The query [`FontMeter::measure`] writes: the cell-size query, the status request
+/// and the device attributes request, wrapped for tmux like the start-up query.
+fn font_query_text(is_tmux: bool) -> String {
+    let (start, escape, end) = Parser::tmux_start_escape_end(is_tmux);
+    format!("{start}{escape}[16t{escape}[5n{escape}[c{end}")
+}
+
+/// The font size the answers to [`FontMeter::measure`]'s query give: the cell-size answer
+/// when it is plausible ([`cell_size_answer`]), else `window` (the window's pixel
+/// size per cell); `None` when neither says, or the query failed and the window
+/// does not say either, so the current size stays.
+fn measured_font(
+    answers: Result<Vec<Response>, QueryError>,
+    window: Option<CellSize>,
+) -> Option<CellSize> {
+    answers
+        .ok()
+        .and_then(|responses| cell_size_answer(&responses))
+        .or(window)
+}
+
+/// The font size in the last cell-size answer among `responses`, when it is
+/// plausible ([`font_size`]).
+fn cell_size_answer(responses: &[Response]) -> Option<CellSize> {
+    responses
+        .iter()
+        .rev()
+        .find_map(|response| match response {
+            Response::CellSize(Some((width, height))) => Some(font_size(*width, *height)),
+            _ => None,
+        })
+        .flatten()
+}
+
 /// Writes `query` to stdout and reads the answers from stdin.
 fn ask(query: &str, stop: impl Fn() -> bool) -> Result<Vec<Response>, QueryError> {
+    write_query(query)?;
+    read_answers(read_stdin_byte, QUERY_TIMEOUT, stop)
+}
+
+/// Writes `query` to stdout at once.
+fn write_query(query: &str) -> io::Result<()> {
     let mut stdout = io::stdout().lock();
     stdout.write_all(query.as_bytes())?;
-    stdout.flush()?;
-    drop(stdout);
-    read_answers(read_stdin_byte, QUERY_TIMEOUT, stop)
+    stdout.flush()
 }
 
 /// Feeds the bytes from `read_byte` to the answer parser until the status report
@@ -202,20 +333,36 @@ fn ask(query: &str, stop: impl Fn() -> bool) -> Result<Vec<Response>, QueryError
 /// `wait`; it is never asked to wait past the deadline `timeout` from now, nor
 /// longer than 50 ms at a time. `stop` is checked before every read.
 fn read_answers(
+    read_byte: impl FnMut(Duration) -> io::Result<Option<u8>>,
+    timeout: Duration,
+    stop: impl Fn() -> bool,
+) -> Result<Vec<Response>, QueryError> {
+    read_answers_after(read_byte, timeout, stop, 0)
+}
+
+/// [`read_answers`] after skipping `owed` status reports and the answers before each:
+/// it returns the answers before the status report numbered `owed + 1`. When the
+/// deadline passes after at least one status report, the answers before the last one
+/// read are the result (the owed ones did not come; that one was the query's own).
+fn read_answers_after(
     mut read_byte: impl FnMut(Duration) -> io::Result<Option<u8>>,
     timeout: Duration,
     stop: impl Fn() -> bool,
+    owed: usize,
 ) -> Result<Vec<Response>, QueryError> {
     let deadline = Instant::now() + timeout;
     let mut parser = Parser::new();
     let mut responses = Vec::new();
+    // The answers before the last status report skipped, and how many were.
+    let mut last = None;
+    let mut skipped = 0;
     loop {
         if stop() {
             return Err(QueryError::Interrupted);
         }
         let left = deadline.saturating_duration_since(Instant::now());
         if left.is_zero() {
-            return Err(QueryError::Timeout);
+            return last.ok_or(QueryError::Timeout);
         }
         let Some(byte) = read_byte(left.min(POLL_SLICE))? else {
             continue;
@@ -223,7 +370,11 @@ fn read_answers(
         // The answers are ASCII; ratatui-image feeds its parser the same way.
         for response in parser.push(char::from(byte)) {
             match response {
-                Response::Status => return Ok(responses),
+                Response::Status if skipped == owed => return Ok(responses),
+                Response::Status => {
+                    skipped += 1;
+                    last = Some(std::mem::take(&mut responses));
+                }
                 other => responses.push(other),
             }
         }
@@ -233,7 +384,8 @@ fn read_answers(
 /// Maps the answers to a protocol and font size as ratatui-image does
 /// (`Picker::from_query_stdio`): the protocol the answers name (Kitty over Sixel),
 /// else one the environment names ([`protocol_from_env`]), else half-blocks; the font
-/// size from the cell-size answer, else `window` (the window's pixel size per cell).
+/// size from the cell-size answer when it is plausible ([`font_size`]), else `window`
+/// (the window's pixel size per cell).
 /// Without any font size the protocol is half-blocks at 10×20, since the other
 /// protocols draw at the pixel size they are given. A failed query is half-blocks
 /// with a warning.
@@ -255,19 +407,16 @@ fn interpret(
         }
     };
     let mut answered = None;
-    let mut cell_size = None;
-    for response in responses {
+    for response in &responses {
         match response {
             Response::Kitty => answered = Some(ProtocolType::Kitty),
             Response::Sixel => {
                 answered.get_or_insert(ProtocolType::Sixel);
             }
-            Response::CellSize(Some((width, height))) => {
-                cell_size = Some(CellSize::new(width, height));
-            }
             _ => {}
         }
     }
+    let cell_size = cell_size_answer(&responses);
     let (protocol, cell_size) = match cell_size.or(window) {
         Some(cell_size) => {
             let protocol = answered
@@ -290,7 +439,9 @@ fn interpret(
 /// turns the kitty answer `ESC _ G i=31;OK ESC \` into the key presses Alt+`_`, `G`,
 /// `i`, `=`, `3`, `1`, `;`, `O`, `K`, Alt+`\`; on the menu the `3` would start a
 /// game. The other answers never become key presses: crossterm keeps the device
-/// attributes to itself and drops the cell size and status reports.
+/// attributes to itself and drops the cell size and status reports (CSI sequences
+/// ending in `t` and `n`, which it cannot parse), so a font measurement that gave up
+/// waiting needs none of this (see [`FontMeter::measure`] for what it does instead).
 ///
 /// After a query that gave up waiting ([`Graphics::answers_pending`]) and for the
 /// next 10 s, it drops one such run of key presses: an Alt+`_`, then printable
@@ -399,14 +550,23 @@ fn protocol_from_env(is_tmux: bool, get: impl Fn(&str) -> Option<String>) -> Opt
 }
 
 /// The font size from the terminal's pixel and cell counts, as ratatui-image
-/// computes it (rounded down); `None` when the terminal reports no pixel size.
+/// computes it (rounded down); `None` when the terminal reports no pixel size, or
+/// one that gives no plausible font ([`font_size`]).
 fn cell_size_from_window(window: WindowSize) -> Option<CellSize> {
     let width = window.width.checked_div(window.columns)?;
     let height = window.height.checked_div(window.rows)?;
-    (width > 0 && height > 0).then(|| CellSize::new(width, height))
+    font_size(width, height)
 }
 
-/// [`cell_size_from_window`] for this terminal.
+/// A font `width` × `height` pixels, or `None` when that cannot be one: a side of
+/// zero or over [`MAX_CELL_PX`].
+fn font_size(width: u16, height: u16) -> Option<CellSize> {
+    let plausible = |side: u16| (1..=MAX_CELL_PX).contains(&side);
+    (plausible(width) && plausible(height)).then(|| CellSize::new(width, height))
+}
+
+/// The font size from this terminal's pixel and cell counts
+/// ([`cell_size_from_window`]); `None` when it reports no pixel size.
 fn window_cell_size() -> Option<CellSize> {
     window_size().ok().and_then(cell_size_from_window)
 }
@@ -448,7 +608,7 @@ fn read_stdin_byte(_wait: Duration) -> io::Result<Option<u8>> {
 #[cfg(test)]
 mod tests {
     use std::cell::Cell;
-    use std::collections::{HashMap, VecDeque};
+    use std::collections::VecDeque;
     use std::io;
     use std::time::{Duration, Instant};
 
@@ -460,7 +620,7 @@ mod tests {
     use super::*;
     use crate::tui::board::CellSize;
     use crate::tui::glyphs::ImageSupport;
-    use crate::tui::test_support::{chord_event, key_event, late_kitty_answer};
+    use crate::tui::test_support::{chord_event, env, key_event, late_kitty_answer};
 
     /// Kitty 0.39: the graphics probe is accepted, no sixel, 9×18 cells.
     const KITTY: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c\x1b[6;18;9t\x1b[0n";
@@ -475,17 +635,12 @@ mod tests {
     /// Both kitty graphics and sixel: Kitty wins, as in ratatui-image.
     const KITTY_AND_SIXEL: &[u8] = b"\x1b[?62;4c\x1b_Gi=31;OK\x1b\\\x1b[6;20;10t\x1b[0n";
 
-    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
-        let map: HashMap<String, String> = pairs
-            .iter()
-            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
-            .collect();
-        move |key| map.get(key).cloned()
-    }
+    /// What the query reader returns.
+    type Answers = Result<Vec<Response>, QueryError>;
 
     /// The answers in `bytes` as the query reader collects them; bytes that end before
     /// the status report are a terminal that never finished answering.
-    fn answers(bytes: &[u8]) -> Result<Vec<Response>, QueryError> {
+    fn answers(bytes: &[u8]) -> Answers {
         let mut source = VecDeque::from(bytes.to_vec());
         read_answers(
             |_| Ok(source.pop_front()),
@@ -764,6 +919,33 @@ mod tests {
         );
     }
 
+    #[test]
+    fn implausible_font_sizes_are_not_believed() {
+        // Over 256 pixels per cell is no font: every picture would be huge. Such a
+        // cell-size answer counts as missing, so the window's size is used, else 10×20.
+        let huge: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[6;4000;300t\x1b[0n";
+        assert_eq!(
+            interpret(answers(huge), false, env(&[]), Some(CellSize::new(9, 18))),
+            detection(ProtocolType::Kitty, (9, 18))
+        );
+        assert_eq!(
+            interpret(answers(huge), false, env(&[]), None),
+            detection(ProtocolType::Halfblocks, (10, 20))
+        );
+        let window = |columns, rows, width, height| WindowSize {
+            rows,
+            columns,
+            width,
+            height,
+        };
+        assert_eq!(
+            cell_size_from_window(window(80, 24, 80 * 256, 24 * 256)),
+            Some(CellSize::new(256, 256))
+        );
+        assert_eq!(cell_size_from_window(window(80, 24, 80 * 257, 480)), None);
+        assert_eq!(cell_size_from_window(window(1, 1, 800, 65535)), None);
+    }
+
     #[test]
     fn no_answer_falls_back_to_half_blocks_with_a_warning() {
         let silent = interpret(
@@ -824,6 +1006,220 @@ mod tests {
         );
     }
 
+    // ----- measuring the font again -----
+
+    /// A font measurement's answers: a `width` × `height` cell, the status report and
+    /// a Sixel terminal's device attributes.
+    fn font_answer(width: u16, height: u16) -> Vec<u8> {
+        format!("\x1b[6;{height};{width}t\x1b[0n\x1b[?62;4c").into_bytes()
+    }
+
+    /// What a measurement reads from `bytes` when `owed` status reports are owed,
+    /// and the bytes it leaves unread.
+    fn owed_answers(bytes: &[u8], owed: usize) -> (Answers, Vec<u8>) {
+        let mut source = VecDeque::from(bytes.to_vec());
+        let answers = read_answers_after(
+            |_| Ok(source.pop_front()),
+            Duration::from_millis(50),
+            || false,
+            owed,
+        );
+        (answers, source.into())
+    }
+
+    #[test]
+    fn the_font_query_asks_for_the_cell_size_the_status_and_the_device_attributes() {
+        assert_eq!(font_query_text(false), "\x1b[16t\x1b[5n\x1b[c");
+        assert_eq!(
+            font_query_text(true),
+            "\x1bPtmux;\x1b\x1b[16t\x1b\x1b[5n\x1b\x1b[c\x1b\\"
+        );
+    }
+
+    #[test]
+    fn a_measurement_leaves_only_the_device_attributes_to_crossterm() {
+        // crossterm drops the cell size and status reports without making an event, and
+        // its reader then waits in `read` for more input: the device attributes after
+        // them make the event that ends that wait.
+        let (answers, left) = owed_answers(&font_answer(8, 16), 0);
+        assert_eq!(
+            answers.expect("complete"),
+            [Response::CellSize(Some((8, 16)))]
+        );
+        assert_eq!(left, b"\x1b[?62;4c");
+    }
+
+    #[test]
+    fn a_measurement_skips_the_answers_owed_by_one_that_gave_up() {
+        // The late answers of a measurement that gave up (8×16) come first, then this
+        // one's own (11×22).
+        let bytes = [font_answer(8, 16), font_answer(11, 22)].concat();
+        let (answers, left) = owed_answers(&bytes, 1);
+        assert_eq!(
+            measured_font(answers, None),
+            Some(CellSize::new(11, 22)),
+            "its own answer, not the late one"
+        );
+        assert_eq!(
+            left, b"\x1b[?62;4c",
+            "only its own device attributes are left"
+        );
+        // Two owed, and keys typed in between do not count as answers.
+        let bytes = [
+            font_answer(8, 16),
+            b"q".to_vec(),
+            font_answer(9, 18),
+            font_answer(11, 22),
+        ]
+        .concat();
+        let (answers, _) = owed_answers(&bytes, 2);
+        assert_eq!(measured_font(answers, None), Some(CellSize::new(11, 22)));
+    }
+
+    #[test]
+    fn owed_answers_that_never_come_leave_the_last_answer_after_the_deadline() {
+        // crossterm read the late answers before this measurement: its own answer is the
+        // only one, taken once the deadline has passed.
+        let (answers, left) = owed_answers(&font_answer(11, 22), 1);
+        assert_eq!(measured_font(answers, None), Some(CellSize::new(11, 22)));
+        assert_eq!(left, b"", "the device attributes were read while waiting");
+        // Nothing at all is a timeout, whatever is owed.
+        for owed in [0, 1, 3] {
+            let (answers, _) = owed_answers(b"", owed);
+            assert!(
+                matches!(answers, Err(QueryError::Timeout)),
+                "{owed}: {answers:?}"
+            );
+        }
+        // An answer cut off before its status report is none.
+        let (answers, _) = owed_answers(b"\x1b[6;22;11t", 1);
+        assert!(matches!(answers, Err(QueryError::Timeout)), "{answers:?}");
+    }
+
+    #[test]
+    fn a_measurement_that_gave_up_is_owed_by_the_next_for_a_while() {
+        let now = Instant::now();
+        let mut meter = FontMeter::default();
+        assert_eq!(meter.owed_at(now), 0);
+        meter.settle(0, &Err(QueryError::Timeout), now);
+        assert_eq!(meter.owed_at(now), 1);
+        meter.settle(1, &Err(QueryError::Timeout), now);
+        assert_eq!(
+            meter.owed_at(now),
+            2,
+            "each one that gave up owes its answers"
+        );
+        assert_eq!(
+            meter.owed_at(now + LATE_ANSWER_WINDOW + Duration::from_millis(1)),
+            0,
+            "answers that late are taken to be read by crossterm or lost"
+        );
+        meter.settle(2, &Ok(vec![Response::CellSize(Some((8, 16)))]), now);
+        assert_eq!(
+            meter.owed_at(now),
+            0,
+            "a measurement that got an answer owes nothing"
+        );
+        meter.settle(0, &Err(QueryError::Interrupted), now);
+        assert_eq!(meter.owed_at(now), 1);
+    }
+
+    #[test]
+    fn a_resize_after_a_measurement_that_gave_up_gets_its_own_answer() {
+        // The pty experiment: a measurement gives up; the next one is asked before the
+        // late answers arrive, then gets them and its own.
+        let mut meter = FontMeter::default();
+        let mut measure = |bytes: &[u8]| {
+            let mut source = VecDeque::from(bytes.to_vec());
+            let answers = meter.read(
+                |_| Ok(source.pop_front()),
+                Duration::from_millis(50),
+                || false,
+            );
+            (measured_font(answers, None), Vec::from(source))
+        };
+        assert_eq!(measure(b""), (None, vec![]), "the first gives up");
+        let bytes = [font_answer(8, 16), font_answer(11, 22)].concat();
+        assert_eq!(
+            measure(&bytes),
+            (Some(CellSize::new(11, 22)), b"\x1b[?62;4c".to_vec())
+        );
+        // Nothing is owed any more: the next takes the first answer.
+        assert_eq!(
+            measure(&font_answer(9, 18)),
+            (Some(CellSize::new(9, 18)), b"\x1b[?62;4c".to_vec())
+        );
+    }
+
+    #[test]
+    fn a_measurement_takes_the_cell_size_answer() {
+        const ZOOMED: &[u8] = b"\x1b[6;16;8t\x1b[0n";
+        assert_eq!(
+            measured_font(answers(ZOOMED), Some(CellSize::DEFAULT)),
+            Some(CellSize::new(8, 16)),
+            "the answer wins over the window's pixels per cell"
+        );
+        assert_eq!(
+            measured_font(answers(ZOOMED), None),
+            Some(CellSize::new(8, 16))
+        );
+        // Recorded answers from real terminals carry it the same way.
+        assert_eq!(
+            measured_font(answers(WEZTERM), None),
+            Some(CellSize::new(8, 16))
+        );
+        assert_eq!(
+            measured_font(answers(GHOSTTY), None),
+            Some(CellSize::new(17, 38))
+        );
+        assert_eq!(measured_font(answers(SIXEL), None), Some(CellSize::DEFAULT));
+    }
+
+    #[test]
+    fn a_measurement_without_an_answer_uses_the_window_else_keeps_the_font() {
+        let window = Some(CellSize::new(8, 16));
+        // A terminal that never answers, one that answers only the status request, and
+        // a query that failed or was interrupted.
+        let cases: [fn() -> Answers; 5] = [
+            || answers(b""),
+            || answers(PLAIN),
+            || answers(b"\x1b[0n"),
+            || Err(QueryError::Interrupted),
+            || Err(QueryError::Io(io::Error::other("input/output error"))),
+        ];
+        for failed in cases {
+            assert_eq!(measured_font(failed(), window), window);
+            assert_eq!(measured_font(failed(), None), None, "the font stays");
+        }
+    }
+
+    #[test]
+    fn garbage_around_a_measurement_is_not_a_font_size() {
+        let window = Some(CellSize::new(8, 16));
+        // Keys typed while the query waits do not hide the answer after them.
+        assert_eq!(
+            measured_font(answers(b"qe4\x1b[A\x1b[6;20;10t\x1b[0n"), window),
+            Some(CellSize::DEFAULT)
+        );
+        // A mangled or implausible answer counts as none.
+        for garbage in [
+            &b"\x1b[6;x;yt\x1b[0n"[..],
+            b"\x1b[6;4000;300t\x1b[0n",
+            b"\x1b[6;0;0t\x1b[0n",
+            b"\x1b[6t\x1b[0n",
+            b"\x01\x7f\xff\x1b\x1b[0n",
+        ] {
+            assert_eq!(
+                measured_font(answers(garbage), window),
+                window,
+                "{garbage:?}"
+            );
+            assert_eq!(measured_font(answers(garbage), None), None, "{garbage:?}");
+        }
+        // Without the status report the answers never end: a timeout.
+        assert_eq!(measured_font(answers(b"zz\x1b[6;16;8t"), None), None);
+    }
+
     // ----- the picker -----
 
     #[test]
diff --git a/src/tui/mod.rs b/src/tui/mod.rs
index fe27341f8243be29a66453f4393e99b9d02dbf0e..a73825139c524854ae0f7a143a30d1c56072693c 100644
--- a/src/tui/mod.rs
+++ b/src/tui/mod.rs
@@ -34,15 +34,17 @@ use std::time::{Duration, Instant};
 
 use ratatui::backend::Backend;
 use ratatui::crossterm::event::Event;
+use ratatui_image::picker::ProtocolType;
 
 use crate::core::Game;
 use crate::engine::{ComputerMove, ComputerPlayer, EngineConfig};
 
 use self::app::{Action, App};
+use self::board::CellSize;
 use self::debug::DebugLog;
 use self::event::AppEvent;
-use self::graphics::{Graphics, LateAnswers};
-use self::worker::{Engine, EngineOutcome, EngineReply};
+use self::graphics::{FontMeter, Graphics, LateAnswers};
+use self::worker::{Engine, EngineOutcome, EngineReply, EngineRequest};
 
 /// How long one batch waits for terminal input before its `Tick` (spec 6.5).
 const TICK: Duration = Duration::from_millis(50);
@@ -227,14 +229,21 @@ fn play(
     // for the cursor position, another answer to wait for.)
     screen.backend_mut().clear()?;
     let mut late = LateAnswers::after(&graphics, Instant::now());
+    let mut meter = FontMeter::default();
     let mut app = build(graphics);
     let result = run_loop(
         &mut app,
         quit,
         |app| {
-            screen
-                .draw(|frame| app.render(frame, Instant::now()))
-                .map(drop)
+            screen.draw(|frame| app.render(frame, Instant::now()))?;
+            // Kitty keeps pictures after the program ends unless they are deleted.
+            if let Some(picker) = app.picker()
+                && picker.protocol_type() == ProtocolType::Kitty
+                && !app.piece_images().is_empty()
+            {
+                terminal::note_kitty_images(picker.tmux_detected());
+            }
+            Ok(())
         },
         |replies| {
             let mut batch = event::collect(replies, TICK)?;
@@ -244,6 +253,10 @@ fn play(
             }
             Ok(batch)
         },
+        |app| {
+            let is_tmux = app.picker().is_some_and(|picker| picker.tmux_detected());
+            meter.measure(is_tmux, || quit.load(Ordering::SeqCst) != 0)
+        },
     );
     app.close_debug_log(LOG_GRACE);
     result
@@ -392,15 +405,23 @@ fn parse_args(args: impl IntoIterator<Item = String>) -> Cli {
 /// (the number of a quit signal, 0 until one arrives) is set. Both are checked
 /// before every draw, so a signal is seen within one batch timeout.
 ///
+/// Engine requests start at once. A font measurement ([`Action::MeasureFont`]) waits
+/// for the end of the batch, so that it sees the terminal after every resize in it,
+/// and runs once however many resizes asked: `measure_font` asks the terminal
+/// ([`FontMeter::measure`] in production, up to [`graphics::QUERY_TIMEOUT`] when it
+/// does not answer) and its result goes to
+/// [`App::font_measured`] before the next draw.
+///
 /// Drawing comes before each batch so that mouse events are hit-tested against
 /// what is on screen. Every batch ends with a `Tick`, so the screen is redrawn at
 /// least once per batch (the spinner and the Jev vs Jev delay depend on it);
 /// ratatui writes only the cells that changed. Once the app quits, the rest of
 /// the batch is dropped.
 ///
-/// `draw` and `next_batch` are the terminal in production and a `TestBackend`
-/// with scripted batches in tests. Engine replies travel over a channel created
-/// here: `next_batch` receives its end, the worker threads send on the other.
+/// `draw`, `next_batch` and `measure_font` are the terminal in production and a
+/// `TestBackend` with scripted batches and font sizes in tests. Engine replies
+/// travel over a channel created here: `next_batch` receives its end, the worker
+/// threads send on the other.
 ///
 /// # Errors
 ///
@@ -410,45 +431,49 @@ fn run_loop(
     quit: &AtomicI32,
     mut draw: impl FnMut(&mut App) -> io::Result<()>,
     mut next_batch: impl FnMut(&Receiver<EngineReply>) -> io::Result<Vec<AppEvent>>,
+    mut measure_font: impl FnMut(&App) -> Option<CellSize>,
 ) -> io::Result<()> {
     let (replies_tx, replies) = mpsc::channel();
     while quit.load(Ordering::SeqCst) == 0 && !app.should_quit() {
         draw(app)?;
         let batch = next_batch(&replies)?;
         let now = Instant::now();
+        let mut measure = false;
         for event in batch {
             for action in app.handle(event, now) {
-                perform(action, app.engine(), &replies_tx);
+                match action {
+                    Action::RequestEngine(request) => {
+                        request_engine(request, app.engine(), &replies_tx);
+                    }
+                    Action::MeasureFont => measure = true,
+                }
             }
             if app.should_quit() {
                 break;
             }
         }
+        if measure && !app.should_quit() {
+            let measured = measure_font(app);
+            app.font_measured(measured);
+        }
     }
     Ok(())
 }
 
-/// Carries out one [`Action`] for the app.
-fn perform(action: Action, engine: &Arc<dyn Engine>, replies: &Sender<EngineReply>) {
-    match action {
-        Action::RequestEngine(request) => {
-            let (generation, hash) = (request.generation, request.hash);
-            if let Err(error) = worker::spawn_request(Arc::clone(engine), request, replies.clone())
-            {
-                // No thread means no reply. Answer for it, so the app stops
-                // waiting: it shows the failure and lets the person retry.
-                let failed = EngineReply {
-                    generation,
-                    hash,
-                    outcome: EngineOutcome::Failed(format!(
-                        "cannot start the engine thread: {error}"
-                    )),
-                    exchange: None,
-                };
-                // The receiver lives in `run_loop`, which is still running.
-                let _ = replies.send(failed);
-            }
-        }
+/// Runs `request` on an engine thread, whose reply arrives on `replies`.
+fn request_engine(request: EngineRequest, engine: &Arc<dyn Engine>, replies: &Sender<EngineReply>) {
+    let (generation, hash) = (request.generation, request.hash);
+    if let Err(error) = worker::spawn_request(Arc::clone(engine), request, replies.clone()) {
+        // No thread means no reply. Answer for it, so the app stops
+        // waiting: it shows the failure and lets the person retry.
+        let failed = EngineReply {
+            generation,
+            hash,
+            outcome: EngineOutcome::Failed(format!("cannot start the engine thread: {error}")),
+            exchange: None,
+        };
+        // The receiver lives in `run_loop`, which is still running.
+        let _ = replies.send(failed);
     }
 }
 
@@ -807,21 +832,34 @@ mod tests {
         unused_steps: usize,
         /// [`App::in_flight`] at every draw, so before every batch.
         in_flight: Vec<usize>,
+        /// The draw each font measurement came after (1 for the first draw).
+        measured_after: Vec<usize>,
     }
 
     /// Runs [`run_loop`] on an 80×24 `TestBackend`, serving `steps` as batches. Running
     /// out of steps before the loop ends is an error, so a loop that fails to quit fails
-    /// the test instead of hanging it.
+    /// the test instead of hanging it. Font measurements find nothing.
     fn drive(app: &mut App, quit: &AtomicI32, steps: Vec<Step>) -> Run {
+        drive_with_font(app, quit, steps, None)
+    }
+
+    /// [`drive`], with every font measurement giving `font`.
+    fn drive_with_font(
+        app: &mut App,
+        quit: &AtomicI32,
+        steps: Vec<Step>,
+        font: Option<CellSize>,
+    ) -> Run {
         let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
         let mut steps = VecDeque::from(steps);
-        let mut draws = 0;
+        let draws = std::cell::Cell::new(0);
         let mut in_flight = Vec::new();
+        let mut measured_after = Vec::new();
         let result = run_loop(
             app,
             quit,
             |app| {
-                draws += 1;
+                draws.set(draws.get() + 1);
                 in_flight.push(app.in_flight());
                 terminal
                     .draw(|frame| app.render(frame, Instant::now()))
@@ -844,12 +882,82 @@ mod tests {
                 batch.push(AppEvent::Tick);
                 Ok(batch)
             },
+            |_| {
+                measured_after.push(draws.get());
+                font
+            },
         );
         Run {
             result,
-            draws,
+            draws: draws.get(),
             unused_steps: steps.len(),
             in_flight,
+            measured_after,
+        }
+    }
+
+    /// A game on an app whose picker draws with `protocol` at 10×20.
+    fn picture_game(protocol: ProtocolType) -> App {
+        let mut app = new_app().with_picker(Some(picker_for(protocol, CellSize::DEFAULT)));
+        app.set_cell_size(CellSize::DEFAULT);
+        let _ = app.handle(key(KeyCode::Char('1')), Instant::now());
+        app
+    }
+
+    fn resizes(sizes: &[(u16, u16)]) -> Step {
+        Step::Events(
+            sizes
+                .iter()
+                .map(|&(columns, rows)| AppEvent::Term(Event::Resize(columns, rows)))
+                .collect(),
+        )
+    }
+
+    #[test]
+    fn a_batch_of_resizes_measures_the_font_once_after_the_batch() {
+        let mut app = picture_game(ProtocolType::Sixel);
+        let run = drive_with_font(
+            &mut app,
+            &AtomicI32::new(0),
+            vec![
+                resizes(&[(100, 30), (90, 26), (100, 30)]),
+                Step::Events(Vec::new()),
+                resizes(&[(80, 24)]),
+                Step::Signal,
+            ],
+            Some(CellSize::new(8, 16)),
+        );
+        run.result.expect("loop ends cleanly");
+        assert_eq!(run.measured_after, [1, 3], "once per batch of resizes");
+        assert_eq!(app.cell_size(), CellSize::new(8, 16));
+        let font = app.picker().map(|picker| picker.font_size());
+        assert_eq!(font.map(|font| (font.width, font.height)), Some((8, 16)));
+
+        // Nothing measured keeps the font.
+        let mut app = picture_game(ProtocolType::Iterm2);
+        let run = drive(
+            &mut app,
+            &AtomicI32::new(0),
+            vec![resizes(&[(100, 30)]), Step::Signal],
+        );
+        run.result.expect("loop ends cleanly");
+        assert_eq!(run.measured_after, [1]);
+        assert_eq!(app.cell_size(), CellSize::DEFAULT);
+    }
+
+    #[test]
+    fn kitty_and_half_block_pictures_are_never_measured() {
+        for protocol in [ProtocolType::Kitty, ProtocolType::Halfblocks] {
+            let mut app = picture_game(protocol);
+            let run = drive_with_font(
+                &mut app,
+                &AtomicI32::new(0),
+                vec![resizes(&[(100, 30), (80, 24)]), Step::Signal],
+                Some(CellSize::new(8, 16)),
+            );
+            run.result.expect("loop ends cleanly");
+            assert!(run.measured_after.is_empty(), "{protocol:?}");
+            assert_eq!(app.cell_size(), CellSize::DEFAULT, "{protocol:?}");
         }
     }
 
@@ -1145,6 +1253,7 @@ mod tests {
             &quit,
             |_| Err(io::Error::other("tty gone")),
             |_| panic!("no batch after a failed draw"),
+            |_| panic!("no font measurement"),
         );
         assert_eq!(result.expect_err("draw failed").to_string(), "tty gone");
     }
@@ -1162,6 +1271,7 @@ mod tests {
                 Ok(())
             },
             |_| Err(io::Error::other("read failed")),
+            |_| panic!("no font measurement"),
         );
         assert_eq!(result.expect_err("read failed").to_string(), "read failed");
         assert_eq!(draws, 1);
@@ -1224,11 +1334,7 @@ mod tests {
         let engine: Arc<dyn Engine> = Arc::new(PanickingEngine(local_engine()));
         assert_eq!(engine.status(), "No JEV_API_KEY — local search");
         let (tx, rx) = mpsc::channel();
-        perform(
-            Action::RequestEngine(worker::EngineRequest::new(1, Game::new())),
-            &engine,
-            &tx,
-        );
+        request_engine(worker::EngineRequest::new(1, Game::new()), &engine, &tx);
         let reply = rx.recv_timeout(REPLY_TIMEOUT).expect("a reply arrives");
         let EngineOutcome::Move(computer) = reply.outcome else {
             panic!("expected the fallback move, got {:?}", reply.outcome);
@@ -1243,7 +1349,7 @@ mod tests {
         let request = worker::EngineRequest::new(7, crate::core::Game::new());
         let hash = request.hash;
 
-        perform(Action::RequestEngine(request), &engine, &tx);
+        request_engine(request, &engine, &tx);
 
         let reply = rx.recv_timeout(REPLY_TIMEOUT).expect("a reply arrives");
         assert!(worker::is_current(&reply, 7, hash));
diff --git a/src/tui/panels.rs b/src/tui/panels.rs
index f91df4a246a18f957e63da353848cbccbb779710..82bd0d20bbc4f1cd2ceb861e7dcbf2b0c9b8cdf0 100644
--- a/src/tui/panels.rs
+++ b/src/tui/panels.rs
@@ -21,7 +21,8 @@
 //! latency, model and the note) and takes the rows from Moves, which gets the rest and
 //! keeps at least [`MOVES_MIN_ROWS`]; when even that is not enough, only the note is cut
 //! short, ending in `…`. Menu, dialogs, help and the game-over overlay stay centred boxes.
-//! In debug mode the Status panel's border says `DEBUG`.
+//! In debug mode the Status panel says `DEBUG`: on the top border beside the mode when
+//! both fit, else at the start of its first line.
 //!
 //! The exchange view takes the whole screen instead of the playing screen (nothing of the
 //! board is drawn under it, so no piece picture is lost under it), with dialogs still on
@@ -524,32 +525,35 @@ fn command_panel(frame: &mut Frame, area: Rect, editor: &LineEditor, focused: bo
     }
 }
 
-/// Status: `DEBUG` in debug mode and the mode on the top border (the first of
-/// [`App::mode_labels`] that fits, so a narrow panel shows `You (W) vs Local` rather than
-/// nothing), then whose turn it is, the Jev vs Jev pace, the thinking spinner and the
-/// latest message.
+/// Status: the mode on the top border (the first of [`App::mode_labels`] that fits, so a
+/// narrow panel shows `You (W) vs Local` rather than nothing) and `DEBUG` in debug mode,
+/// then whose turn it is, the Jev vs Jev pace, the thinking spinner and the latest
+/// message. The mode keeps priority: `DEBUG` joins it on the top border only when both
+/// fit, else it starts the first status line (the bottom border is the next panel's
+/// top one).
 fn status_panel(frame: &mut Frame, area: Rect, app: &App, now: Instant) {
     const TITLE: &str = " Status ";
-    let mut block = side_block("Status");
-    let mut titles = TITLE.len();
-    if app.debug_mode() {
-        block = block.title_top(Line::from(DEBUG_TAG).yellow().bold());
-        // One border cell between the two titles.
-        titles += DEBUG_TAG.len() + 1;
-    }
+    let block = side_block("Status");
     let inner = block.inner(area);
     // Two corners and at least two border cells between the titles.
-    let room = usize::from(area.width).saturating_sub(titles + 4);
+    let room = usize::from(area.width).saturating_sub(TITLE.len() + 4);
     let mode = app
         .mode_labels()
         .into_iter()
         .map(|label| format!(" {label} "))
         .find(|mode| Span::raw(mode.as_str()).width() <= room);
-    let block = match mode {
+    let mode_width = mode.as_deref().map_or(0, |mode| Span::raw(mode).width());
+    let mut block = match mode {
         Some(mode) => block.title_top(Line::from(mode).right_aligned()),
         None => block,
     };
-    let lines = status_lines(app, now, inner.width, inner.height);
+    // The tag and one border cell between it and the Status title.
+    let tag_on_border = app.debug_mode() && mode_width + DEBUG_TAG.len() < room;
+    if tag_on_border {
+        block = block.title_top(Line::from(DEBUG_TAG).yellow().bold());
+    }
+    let tag_in_text = app.debug_mode() && !tag_on_border;
+    let lines = status_lines(app, now, inner.width, inner.height, tag_in_text);
     side_panel(frame, area, block, lines);
 }
 
@@ -568,12 +572,31 @@ fn fitted(full: String, brief: String, width: u16) -> String {
 /// for an earlier request), and the latest message in the rows left ([`fit_message`]). The
 /// turn and thinking lines drop words rather than wrap (the computer's name "Local search"
 /// is long), so they keep one row each; a game's outcome has no brief form and may wrap,
-/// and the message gets the rows its wrapped lines leave.
-fn status_lines(app: &App, now: Instant, width: u16, rows: u16) -> Vec<Line<'static>> {
+/// and the message gets the rows its wrapped lines leave. With `debug_tag` the turn line
+/// starts with `DEBUG`.
+fn status_lines(
+    app: &App,
+    now: Instant,
+    width: u16,
+    rows: u16,
+    debug_tag: bool,
+) -> Vec<Line<'static>> {
     let game = app.game();
     let in_check = game.outcome().is_none() && game.position().is_check();
-    let turn = Line::from(fitted(app.turn_text(), app.turn_text_brief(), width)).bold();
-    let mut lines = vec![if in_check { turn.red() } else { turn }];
+    let tag = if debug_tag {
+        DEBUG_TAG.trim_start()
+    } else {
+        ""
+    };
+    let turn_width = width.saturating_sub(u16::try_from(tag.len()).unwrap_or(u16::MAX));
+    let mut turn = Line::from(fitted(app.turn_text(), app.turn_text_brief(), turn_width)).bold();
+    if in_check {
+        turn = turn.red();
+    }
+    if debug_tag {
+        turn.spans.insert(0, Span::raw(tag).yellow());
+    }
+    let mut lines = vec![turn];
     if app.mode() == Mode::JevVsJev && game.outcome().is_none() {
         let pace = if app.paused() {
             "paused · space resumes".to_string()
@@ -1515,7 +1538,7 @@ mod tests {
     use super::*;
     use crate::core::{START_FEN, Square};
     use crate::tui::board::{image_area, square_at, square_rect};
-    use crate::tui::debug::DebugLog;
+    use crate::tui::debug::{DebugLog, NO_LOG_PATH};
     use crate::tui::event::AppEvent;
     use crate::tui::glyphs::{ImageSupport, initial_glyphs};
     use crate::tui::graphics::picker_for;
@@ -1938,6 +1961,11 @@ mod tests {
 
     /// The Status panel's top border, where the mode title goes.
     fn status_title(h: &Harness) -> String {
+        status_row(h, 0)
+    }
+
+    /// Row `row` of the Status panel, counted from its top border.
+    fn status_row(h: &Harness, row: u16) -> String {
         let buffer = h.buffer();
         let area = buffer.area;
         let status = playing_layout(
@@ -1948,10 +1976,58 @@ mod tests {
         )
         .status;
         (status.x..status.right())
-            .map(|x| buffer[(x, status.y)].symbol())
+            .map(|x| buffer[(x, status.y + row)].symbol())
             .collect()
     }
 
+    #[test]
+    fn the_mode_title_keeps_its_room_in_debug_mode() {
+        for (width, height) in [(60, 20), (80, 24), (120, 40)] {
+            for jev in [false, true] {
+                for key in ['1', '2', '3', '5'] {
+                    let engine = || {
+                        if jev {
+                            FakeEngine::jev()
+                        } else {
+                            FakeEngine::local()
+                        }
+                    };
+                    let mut plain = Harness::sized(engine(), width, height);
+                    plain.char(key);
+                    let mut h = Harness::build(engine(), (width, height), Vec::new(), |app| {
+                        app.with_debug(DebugLog::open(Err(NO_LOG_PATH.to_string())))
+                    });
+                    h.char(key);
+                    let case = format!("{width}x{height} jev={jev} {key}");
+                    let top = status_title(&h);
+                    let first = status_row(&h, 1);
+                    // The same mode title as without debug mode, which always has one.
+                    let title = h
+                        .app
+                        .mode_labels()
+                        .into_iter()
+                        .find(|label| status_title(&plain).contains(&format!(" {label} ┐")))
+                        .unwrap_or_else(|| panic!("{case}: no mode title"));
+                    assert!(top.contains(&format!(" {title} ┐")), "{case}: {top}");
+                    // DEBUG beside it when both fit, else at the start of the first line.
+                    let on_top = top.contains("┌ Status ─ DEBUG ─");
+                    let in_text = first.starts_with("│ DEBUG White to move");
+                    assert!(on_top != in_text, "{case}: {top} / {first}");
+                    if width >= 120 && jev {
+                        assert!(on_top, "{case}: {top}");
+                    }
+                }
+            }
+        }
+        // At the smallest size no mode leaves room for both.
+        let mut h = Harness::build(FakeEngine::jev(), (60, 20), Vec::new(), |app| {
+            app.with_debug(DebugLog::open(Err(NO_LOG_PATH.to_string())))
+        });
+        h.char('5');
+        assert!(status_title(&h).contains(" Jev vs Jev ┐"));
+        insta::assert_snapshot!("jev_vs_jev_debug_60x20", h.terminal.backend());
+    }
+
     #[test]
     fn the_mode_title_shortens_until_it_fits() {
         let engine = |jev: bool| {
@@ -2634,7 +2710,7 @@ mod tests {
     /// answer to 1. e4 came after an undo (stale), then its answer to 1. e4 played again.
     fn two_exchanges(width: u16, height: u16) -> Harness {
         let mut h = Harness::build(FakeEngine::jev(), (width, height), Vec::new(), |app| {
-            app.with_debug(DebugLog::open(None))
+            app.with_debug(DebugLog::open(Err(NO_LOG_PATH.to_string())))
         });
         h.char('2');
         h.moves(&["e4"]);
diff --git a/src/tui/pieces.rs b/src/tui/pieces.rs
index f5b200e26a5928cf4866ae26f05dd97384d2a75e..c99315edb3e918cd28d7bab20dbcd1188b34e20b 100644
--- a/src/tui/pieces.rs
+++ b/src/tui/pieces.rs
@@ -62,6 +62,10 @@ fn source(piece: Piece) -> &'static RgbaImage {
 /// anti-aliased edges are scaled against the colour they are shown on, and nothing
 /// depends on how a terminal treats transparency. A zero width or height gives an
 /// empty image.
+///
+/// The image is allocated in full (4 bytes a pixel), so the caller keeps the size
+/// to what a board square needs; the board builds nothing over 4096 pixels on a
+/// side, and a size whose byte count overflows `usize` panics.
 #[must_use]
 pub fn composite(piece: Piece, background: [u8; 3], width_px: u32, height_px: u32) -> RgbaImage {
     let [r, g, b] = background;
@@ -265,6 +269,27 @@ mod tests {
         }
     }
 
+    #[test]
+    fn every_piece_is_embedded_from_its_own_file() {
+        // The file name comes from the piece itself: `w` or `b`, then the kind's letter.
+        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/pieces");
+        for piece in all_pieces() {
+            let side = match piece.color {
+                Side::White => 'w',
+                Side::Black => 'b',
+            };
+            let name = format!("{side}{}.png", piece.kind.to_char().to_ascii_uppercase());
+            let bytes = std::fs::read(dir.join(&name)).expect("the asset file");
+            let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
+                .expect("a PNG")
+                .into_rgba8();
+            assert!(
+                image == *source(piece),
+                "{piece:?} is not drawn from {name}"
+            );
+        }
+    }
+
     #[test]
     fn every_piece_has_its_own_image() {
         let pieces: Vec<Piece> = all_pieces().collect();
@@ -298,7 +323,17 @@ mod tests {
     fn composite_is_opaque_with_the_background_in_the_corners() {
         for piece in all_pieces() {
             for background in [LIGHT, DARK, SELECTED] {
-                for (width, height) in [(48, 48), (50, 60), (60, 20), (20, 60), (160, 160)] {
+                // (24, 32) and (30, 40) are the smallest image areas the board draws:
+                // 3×2 cells at 8×16 and at 10×20.
+                for (width, height) in [
+                    (24, 32),
+                    (30, 40),
+                    (48, 48),
+                    (50, 60),
+                    (60, 20),
+                    (20, 60),
+                    (160, 160),
+                ] {
                     let image = composite(piece, background, width, height);
                     assert!(
                         image.pixels().all(|p| p[3] == 255),
diff --git a/src/tui/snapshots/chess__tui__panels__tests__jev_vs_jev_debug_60x20.snap b/src/tui/snapshots/chess__tui__panels__tests__jev_vs_jev_debug_60x20.snap
new file mode 100644
index 0000000000000000000000000000000000000000..9ace797a2298ff1d8bb774e147b9d8d2b347ffd3
--- /dev/null
+++ b/src/tui/snapshots/chess__tui__panels__tests__jev_vs_jev_debug_60x20.snap
@@ -0,0 +1,24 @@
+---
+source: src/tui/panels.rs
+expression: h.terminal.backend()
+---
+"┌ Board ────────────────────┐┌ Status ───────── Jev vs Jev ┐"
+"│                           ││ DEBUG White to move (Jev)   │"
+"│                           ││ step 1.0 s · space pauses   │"
+"│                           ││ | Jev thinking... 0.0s      │"
+"│ 8 ♜  ♞  ♝  ♛  ♚  ♝  ♞  ♜  ││                             │"
+"│ 7 ♟︎  ♟︎  ♟︎  ♟︎  ♟︎  ♟︎  ♟︎  ♟︎  │├ Jev ────────────────────────┤"
+"│ 6                         ││ Jev ready (jev-test)        │"
+"│ 5                         ││ no move yet                 │"
+"│ 4                         ││                             │"
+"│ 3                         │├ Moves ──────────────────────┤"
+"│ 2 ♟︎  ♟︎  ♟︎  ♟︎  ♟︎  ♟︎  ♟︎  ♟︎  ││ no moves yet                │"
+"│ 1 ♜  ♞  ♝  ♛  ♚  ♝  ♞  ♜  ││                             │"
+"│   a  b  c  d  e  f  g  h  ││                             │"
+"│                           ││                             │"
+"│                           ││                             │"
+"│                           ││                             │"
+"└───────────────────────────┘├ Captured ───────────────────┤"
+"┌ Command ──────────────────┐│ White                       │"
+"│ / move  : command  ? help ││ Black                       │"
+"└───────────────────────────┘└─────────────────────────────┘"
diff --git a/src/tui/terminal.rs b/src/tui/terminal.rs
index ac366f4dccd3ffc6321b472e742946c3c5eba4c6..d57c1fc2fd11c88860ff9aa5adebc166bf92f469 100644
--- a/src/tui/terminal.rs
+++ b/src/tui/terminal.rs
@@ -2,11 +2,11 @@
 //!
 //! [`enter`] puts the terminal in raw mode on the alternate screen, runs the
 //! caller's start-up step (the graphics query), then turns on click-and-drag
-//! mouse reporting and bracketed paste. [`leave`] undoes all of it
-//! and is reached on every exit path: a normal return or `?` error (through
-//! [`Guard`]), a panic on the UI thread (through the panic hook) and SIGINT,
-//! SIGTERM or SIGHUP (through the flag from [`register_signals`], which the main
-//! loop checks every tick; the program then ends by that signal with
+//! mouse reporting and bracketed paste. [`leave`] undoes all of it (deleting any
+//! Kitty pictures first) and is reached on every exit path: a normal return or `?`
+//! error (through [`Guard`]), a panic on the UI thread (through the panic hook) and
+//! SIGINT, SIGTERM or SIGHUP (through the flag from [`register_signals`], which the
+//! main loop checks every tick; the program then ends by that signal with
 //! [`exit_by_signal`]). If the loop does not react within [`STUCK_GRACE`], the
 //! signal thread restores the terminal itself and ends the process. That is also
 //! how a hangup ends when the UI is idle: crossterm keeps polling a hung-up tty
@@ -22,7 +22,7 @@ use std::fmt;
 use std::io::{self, stdout};
 use std::panic;
 use std::sync::Arc;
-use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
+use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
 use std::thread;
 use std::time::Duration;
 
@@ -33,11 +33,22 @@ use ratatui::crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
 use ratatui::crossterm::terminal::disable_raw_mode;
 use ratatui::crossterm::terminal::is_raw_mode_enabled;
 use ratatui::crossterm::{Command, execute};
+use ratatui_image::picker::cap_parser::Parser;
 
 /// True between a successful [`enter`] and the first [`leave`], which makes
 /// `leave` idempotent and a no-op when the terminal was never set up.
 static ACTIVE: AtomicBool = AtomicBool::new(false);
 
+/// Whether this session drew Kitty pictures, and how they reached the terminal:
+/// [`NO_KITTY`], [`KITTY`] or [`KITTY_IN_TMUX`]. [`leave`] deletes them.
+static KITTY_IMAGES: AtomicU8 = AtomicU8::new(NO_KITTY);
+/// No Kitty picture was drawn.
+const NO_KITTY: u8 = 0;
+/// Kitty pictures were sent straight to the terminal.
+const KITTY: u8 = 1;
+/// Kitty pictures went through tmux's passthrough.
+const KITTY_IN_TMUX: u8 = 2;
+
 /// The only thread whose panic may touch the terminal: the one running the UI.
 const UI_THREAD: &str = "main";
 
@@ -95,6 +106,48 @@ impl Command for DisableClickMouse {
     }
 }
 
+/// Deletes every Kitty picture and frees its data (`a=d,d=A`), so Kitty and Ghostty
+/// do not keep them after the program ends. With `tmux` the command is wrapped for
+/// tmux's passthrough, as ratatui-image wraps the pictures. It prints nothing.
+#[derive(Clone, Copy, Debug, PartialEq, Eq)]
+pub struct DeleteKittyImages {
+    /// The pictures went through tmux.
+    pub tmux: bool,
+}
+
+impl Command for DeleteKittyImages {
+    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
+        let (start, escape, end) = Parser::tmux_start_escape_end(self.tmux);
+        write!(f, "{start}{escape}_Ga=d,d=A{escape}\\{end}")
+    }
+
+    /// The Windows console draws no Kitty pictures.
+    #[cfg(windows)]
+    fn execute_winapi(&self) -> io::Result<()> {
+        Ok(())
+    }
+}
+
+/// Notes that Kitty pictures were drawn (through tmux when `tmux`), so [`leave`]
+/// deletes them before it leaves the alternate screen.
+pub fn note_kitty_images(tmux: bool) {
+    note_kitty(&KITTY_IMAGES, tmux);
+}
+
+fn note_kitty(state: &AtomicU8, tmux: bool) {
+    state.store(if tmux { KITTY_IN_TMUX } else { KITTY }, Ordering::SeqCst);
+}
+
+/// The command that deletes the Kitty pictures noted in `state`, once; `None` when
+/// none were drawn.
+fn kitty_cleanup(state: &AtomicU8) -> Option<DeleteKittyImages> {
+    match state.swap(NO_KITTY, Ordering::SeqCst) {
+        KITTY => Some(DeleteKittyImages { tmux: false }),
+        KITTY_IN_TMUX => Some(DeleteKittyImages { tmux: true }),
+        _ => None,
+    }
+}
+
 /// Sets up the terminal: raw mode and the alternate screen (`ratatui::try_init`),
 /// then `before_input`, then click-and-drag mouse reporting and bracketed paste,
 /// then a thread-aware panic hook. Returns the terminal with what `before_input`
@@ -134,15 +187,28 @@ pub fn enter<T>(before_input: impl FnOnce() -> T) -> io::Result<(DefaultTerminal
         }
     };
     ACTIVE.store(true, Ordering::SeqCst);
-    let value = before_input();
-    finish_enter(
-        original,
-        || execute!(stdout(), EnableClickMouse, EnableBracketedPaste),
-        leave,
-    )?;
+    let value = query_then_enable(before_input, || {
+        finish_enter(
+            original,
+            || execute!(stdout(), EnableClickMouse, EnableBracketedPaste),
+            leave,
+        )
+    })?;
     Ok((terminal, value))
 }
 
+/// The order [`enter`] keeps once raw mode and the alternate screen are on: first
+/// `before_input` (the graphics query), then `enable` (mouse reporting and bracketed
+/// paste), so no mouse or paste report can mix into the query's answers.
+fn query_then_enable<T>(
+    before_input: impl FnOnce() -> T,
+    enable: impl FnOnce() -> io::Result<()>,
+) -> io::Result<T> {
+    let value = before_input();
+    enable()?;
+    Ok(value)
+}
+
 /// The rest of [`enter`] once `try_init` succeeded: `enable` turns on mouse
 /// reporting and bracketed paste. If it fails, `undo` restores the terminal and
 /// `original` becomes the panic hook again (dropping ratatui's); otherwise the
@@ -163,19 +229,29 @@ fn finish_enter(
     Ok(())
 }
 
-/// Restores the terminal: turns off mouse reporting and bracketed paste, then
+/// Restores the terminal: deletes the Kitty pictures if any were drawn
+/// ([`note_kitty_images`]), turns off mouse reporting and bracketed paste, then
 /// raw mode and the alternate screen (`ratatui::try_restore`), then shows the
 /// cursor. Every error is ignored and nothing is printed, so this never panics,
 /// even on a hung-up tty. Only the first call after [`enter`] does anything, so
 /// every exit path may call it.
 pub fn leave() {
     leave_once(&ACTIVE, || {
-        let _ = execute!(stdout(), DisableClickMouse, DisableBracketedPaste);
+        write_teardown(&mut stdout(), kitty_cleanup(&KITTY_IMAGES));
         let _ = ratatui::try_restore();
         let _ = execute!(stdout(), Show);
     });
 }
 
+/// What [`leave`] writes while still on the alternate screen: `kitty` (the pictures'
+/// deletion) first, then mouse reporting and bracketed paste off. Errors are ignored.
+fn write_teardown(out: &mut impl io::Write, kitty: Option<DeleteKittyImages>) {
+    if let Some(delete) = kitty {
+        let _ = execute!(out, delete);
+    }
+    let _ = execute!(out, DisableClickMouse, DisableBracketedPaste);
+}
+
 /// Calls [`leave`] when dropped. Bind it to a named variable for the lifetime of
 /// the UI: `let _guard = Guard;` (`let _ = Guard;` would drop it immediately).
 #[derive(Debug)]
@@ -339,7 +415,7 @@ fn is_ui_thread(name: Option<&str>) -> bool {
 #[cfg(test)]
 mod tests {
     use super::*;
-    use std::cell::Cell;
+    use std::cell::{Cell, RefCell};
 
     fn ansi(command: impl Command) -> String {
         let mut out = String::new();
@@ -383,6 +459,43 @@ mod tests {
         );
     }
 
+    #[cfg(unix)]
+    #[test]
+    fn kitty_pictures_are_deleted_first_and_only_after_kitty_drew() {
+        assert_eq!(
+            ansi(DeleteKittyImages { tmux: false }),
+            "\x1b_Ga=d,d=A\x1b\\"
+        );
+        assert_eq!(
+            ansi(DeleteKittyImages { tmux: true }),
+            "\x1bPtmux;\x1b\x1b_Ga=d,d=A\x1b\x1b\\\x1b\\"
+        );
+        let teardown = |kitty| {
+            let mut out = Vec::new();
+            write_teardown(&mut out, kitty);
+            String::from_utf8(out).unwrap()
+        };
+        assert_eq!(
+            teardown(Some(DeleteKittyImages { tmux: false })),
+            "\x1b_Ga=d,d=A\x1b\\\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?2004l"
+        );
+        assert_eq!(
+            teardown(None),
+            "\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?2004l"
+        );
+
+        let used = AtomicU8::new(NO_KITTY);
+        assert_eq!(kitty_cleanup(&used), None, "no kitty picture was drawn");
+        note_kitty(&used, true);
+        note_kitty(&used, false);
+        assert_eq!(
+            kitty_cleanup(&used),
+            Some(DeleteKittyImages { tmux: false }),
+            "the last note wins"
+        );
+        assert_eq!(kitty_cleanup(&used), None, "cleaned up once");
+    }
+
     #[test]
     fn leave_once_restores_only_once() {
         let active = AtomicBool::new(true);
@@ -450,6 +563,28 @@ mod tests {
         assert!(success_installs_thread_hook, "thread-aware hook installed");
     }
 
+    #[test]
+    fn the_query_runs_before_mouse_and_paste_are_enabled() {
+        // `enter` calls this once raw mode and the alternate screen are on (the pty
+        // smoke test checks the query comes after ?1049h on the wire).
+        let steps = RefCell::new(Vec::new());
+        let value = query_then_enable(
+            || {
+                steps.borrow_mut().push("query");
+                7
+            },
+            || {
+                steps.borrow_mut().push("enable");
+                Ok(())
+            },
+        );
+        assert_eq!(value.unwrap(), 7);
+        assert_eq!(*steps.borrow(), ["query", "enable"]);
+
+        let failed = query_then_enable(|| 7, || Err(io::Error::other("no mouse")));
+        assert_eq!(failed.unwrap_err().to_string(), "no mouse");
+    }
+
     #[test]
     fn only_the_main_thread_restores_on_panic() {
         assert!(is_ui_thread(Some("main")));
diff --git a/src/tui/test_support/harness.rs b/src/tui/test_support/harness.rs
index d13daa9584663f8384b9f6561d1364ebea22a5ad..006902bd89a803d8db9aae7acb7ac830bab2f793 100644
--- a/src/tui/test_support/harness.rs
+++ b/src/tui/test_support/harness.rs
@@ -96,8 +96,9 @@ impl Harness {
     pub(crate) fn send(&mut self, event: AppEvent) -> Vec<Action> {
         let actions = self.app.handle(event, self.now);
         for action in &actions {
-            let Action::RequestEngine(request) = action;
-            self.request = Some(request.clone());
+            if let Action::RequestEngine(request) = action {
+                self.request = Some(request.clone());
+            }
         }
         self.draw();
         actions
diff --git a/src/tui/test_support/mod.rs b/src/tui/test_support/mod.rs
index b3ba580d0e279d5700715a0092e228d2a8278fac..0a717d4d3df7c9c1e466976228bd9e136a0a5cf0 100644
--- a/src/tui/test_support/mod.rs
+++ b/src/tui/test_support/mod.rs
@@ -17,6 +17,7 @@
 pub(crate) mod engine;
 pub(crate) mod harness;
 
+use std::collections::HashMap;
 use std::fs;
 use std::path::{Path, PathBuf};
 use std::sync::atomic::{AtomicUsize, Ordering};
@@ -118,6 +119,18 @@ pub(crate) fn buffer_text(buffer: &Buffer) -> String {
     text
 }
 
+// ----- environment -----
+
+/// An environment variable reader over `pairs` only, for the functions that take
+/// `get` in place of `std::env::var`.
+pub(crate) fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
+    let map: HashMap<String, String> = pairs
+        .iter()
+        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
+        .collect();
+    move |key| map.get(key).cloned()
+}
+
 // ----- files -----
 
 /// A fresh, uniquely named folder under the system temp dir, removed on drop.
diff --git a/tests/pty_smoke.py b/tests/pty_smoke.py
index 6813c139260bbe57d62edf5a63f0f6582a739db3..ed4a8735ff50125700c13f022864f315ddc325f1 100644
--- a/tests/pty_smoke.py
+++ b/tests/pty_smoke.py
@@ -23,16 +23,29 @@ nothing touches the network), drives it with keystrokes or signals, and checks:
 * the graphics query (spec 9.3), on a pty that
   - never answers: the query is written once, in raw mode, between ?1049h and
     ?1000h; start-up goes on after about 1 s with a menu warning, keys typed
-    afterwards work, half-block pictures are in the `g` cycle, and the terminal
-    is restored;
+    afterwards work, the Image style (half blocks) is in the `g` cycle, showing
+    solid glyphs on 80x24's small squares and pictures at 200x60, and the
+    terminal is restored;
   - answers late, while the menu is up: the answer does not act as key presses
     (its `3` would start a game);
   - gets SIGTERM or hangs up while the query waits: the process ends by that
     signal at once, SIGTERM with the terminal restored;
   - answers like Kitty: start-up goes on at once, the answer is not echoed,
     Image is the starting style and pieces are kitty pictures (unicode
-    placeholders);
-* exit writes ?1006l ?1002l ?1000l ?2004l and then ?1049l, then only shows the
+    placeholders), and a resize asks nothing (placeholders scale with the cells);
+  - answers like a Sixel terminal with a 10x20 font, then zooms out to 8x16
+    (more cells, and SIGWINCH): the resize asks for the cell size again (CSI 16 t,
+    the status request and the device attributes, nothing else), and with the
+    answer the pictures are re-encoded for 8x16 cells, so none spills out of its
+    square; the window reports no pixels, so only the answer can give the new
+    size; a resize whose query is not answered keeps 8x16 and the game goes on
+    after about 1 s, and its late answers, which end with the device attributes,
+    do not leave the UI waiting for a key (a resize right after them is handled
+    at once); a resize that comes after a query gave up but before its late
+    answers arrive takes its own answer (11x22), not the late one, and the UI
+    still reacts to the next resize without a key;
+* exit writes ?1006l ?1002l ?1000l ?2004l and then ?1049l (after kitty pictures,
+  Kitty's delete-all-images command comes first, and only then), then only shows the
   cursor again (?25h) and draws nothing on the main screen, exits with status 0,
   and leaves the pty's termios exactly as it was before the program started
   (ICANON and ECHO back on);
@@ -87,7 +100,25 @@ QUERY_PARTS = [QUERY_START, b"\x1b[c", b"\x1b[16t", QUERY_END]
 # What Kitty answers: graphics OK, device attributes without sixel, a 9x18 pixel
 # cell and the status report.
 KITTY_ANSWER = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c\x1b[6;18;9t\x1b[0n"
+# What a Sixel terminal answers: no kitty graphics, device attributes with sixel
+# (4), a 10x20 pixel cell and the status report.
+SIXEL_ANSWER = b"\x1b[?62;4c\x1b[6;20;10t\x1b[0n"
+# A Sixel picture's raster attributes: its width and height in pixels.
+SIXEL_RASTER = re.compile(rb'\x1bP[0-9;]*q"1;1;(\d+);(\d+)')
 QUERY_WARNING = "graphics query: no answer within 1 s"
+# What the app asks after a resize while its pictures are Sixel or iTerm2: the cell
+# size and the status report, which ends the answers the app reads, then the device
+# attributes, whose answer comes last and is left to crossterm.
+FONT_QUERY = b"\x1b[16t\x1b[5n"
+ATTRIBUTES_REQUEST = b"\x1b[c"
+FULL_FONT_QUERY = FONT_QUERY + ATTRIBUTES_REQUEST
+# A Sixel terminal's answer to the device attributes request.
+ATTRIBUTES_ANSWER = b"\x1b[?62;4c"
+# A Sixel terminal's answer to the font query after a zoom to an 8x16 pixel cell.
+ZOOMED_ANSWER = b"\x1b[6;16;8t\x1b[0n" + ATTRIBUTES_ANSWER
+# Kitty's command to delete every image and free its data, written at exit after
+# kitty pictures were drawn.
+KITTY_DELETE = b"\x1b_Ga=d,d=A\x1b\\"
 # Kitty's unicode placeholder: every cell of a kitty picture holds one.
 PLACEHOLDER = "\U0010EEEE"
 SECRET_VARS = ("JEV_API_KEY", "TYPESAFE_API_KEY")
@@ -327,13 +358,14 @@ class App:
                 return True
         return False
 
-    def wait_bytes(self, needle, timeout=5.0):
+    def wait_bytes(self, needle, timeout=5.0, start=0):
+        """Waits until `needle` is in the stream from offset `start` on."""
         end = time.monotonic() + timeout
         while time.monotonic() < end:
-            if needle in self.stream:
+            if needle in self.stream[start:]:
                 return True
             self.pump(0.02)
-        return needle in self.stream
+        return needle in self.stream[start:]
 
     def wait_exit(self, timeout=5.0):
         end = time.monotonic() + timeout
@@ -378,7 +410,8 @@ class App:
 
 def check_setup(app, query=False):
     """Checks the setup sequences; with `query`, that the graphics query is written
-    once, between ?1049h and ?1000h, and otherwise that none of it is written."""
+    once, after ?1049h and before the first mouse or paste sequence (?1000h to
+    ?2004h), and otherwise that none of it is written."""
     check(app.wait_bytes(SETUP[-1]), "setup sequences arrive")
     offsets = ordered(app.stream, SETUP)
     check(
@@ -394,18 +427,30 @@ def check_setup(app, query=False):
             f"offsets {offsets}",
         )
         check(app.stream.count(QUERY_START) == 1, "the query is written once")
+        query_end = app.stream.find(QUERY_END)
+        early = [seq for seq in SETUP[1:] if 0 <= app.stream.find(seq) < query_end]
+        check(not early, "no mouse or paste sequence before the query", f"found {early}")
     else:
         sent = [part for part in QUERY_PARTS if part in app.stream]
         check(not sent, "images off: no graphics query", f"found {sent}")
 
 
-def check_teardown(app, after, label):
+def check_teardown(app, after, label, kitty=False):
     offsets = ordered(app.stream, TEARDOWN, after)
     check(
         offsets is not None,
         f"{label}: teardown order ?1006l ?1002l ?1000l ?2004l then ?1049l",
         f"offsets {offsets}",
     )
+    if kitty:
+        deleted = ordered(app.stream, [KITTY_DELETE] + TEARDOWN, after)
+        check(
+            deleted is not None and app.stream.count(KITTY_DELETE) == 1,
+            f"{label}: the kitty pictures are deleted once, before ?1006l and ?1049l",
+            f"offsets {deleted}",
+        )
+    else:
+        check(KITTY_DELETE not in app.stream, f"{label}: no kitty delete command")
     if offsets is not None:
         tail = app.stream[offsets[-1] + len(TEARDOWN[-1]):]
         # The cursor comes back (it is hidden while the UI runs) and nothing else follows.
@@ -775,8 +820,24 @@ def scenario_query_unanswered(binary):
         app.send(b"gg")
         check(app.wait_screen("glyphs: image"), "Image is still in the g cycle")
         text = app.screen().text()
-        check("▀" in text or "▄" in text, "pieces are drawn with half-blocks")
-        app.screen().show("half-block pictures")
+        # 80x24 gives 5x2 squares, too small for half-block pictures (11x5 at least).
+        check(
+            "♜" in text and "▀" not in text and "▄" not in text,
+            "at 80x24 the squares are too small for half-block pictures: solid glyphs",
+        )
+        app.screen().show("image style on small squares")
+        resized_at = len(app.stream)
+        set_window(app, 60, 200, 0, 0)
+        end = time.monotonic() + 5
+        while "▀".encode() not in app.stream[resized_at:] and time.monotonic() < end:
+            app.pump(0.05)
+        drawn = app.stream[resized_at:]
+        check(
+            "▀".encode() in drawn or "▄".encode() in drawn,
+            "at 200x60 the pieces are drawn with half-blocks",
+        )
+        set_window(app, ROWS, COLS, 0, 0)
+        app.idle(0.3)
         app.send(b"q")
         check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
         quit_at = len(app.stream)
@@ -891,6 +952,17 @@ def scenario_query_kitty(binary):
         check(board.count(PLACEHOLDER) > 0, "the board shows the pictures' placeholder cells")
         check("♜" not in board, "no solid glyphs while the pictures are shown")
         app.screen().show("kitty pictures (placeholders show as their character)")
+        resized_at = len(app.stream)
+        set_window(app, 30, 100, 800, 480)
+        end = time.monotonic() + 5
+        while b"a=T,U=1" not in app.stream[resized_at:] and time.monotonic() < end:
+            app.pump(0.05)
+        app.idle(0.3)
+        check(b"a=T,U=1" in app.stream[resized_at:], "a resize redraws the kitty pictures")
+        check(b"[16t" not in app.stream[resized_at:], "a resize asks nothing with kitty pictures")
+        set_window(app, ROWS, COLS, 0, 0)
+        app.idle(0.3)
+        check(b"[16t" not in app.stream[resized_at:], "nor does a second one")
         app.send(b"g")
         check(app.wait_screen("glyphs: solid"), "the style was Image (g goes on to Solid)")
         check("♜" in app.screen().text(), "then the pieces are solid glyphs")
@@ -900,7 +972,166 @@ def scenario_query_kitty(binary):
         app.send(b"y", settle=0)
         check(app.wait_exit(), "process exits after y")
         check(app.status == 0, "exit status 0", f"status {app.status}")
-        check_teardown(app, quit_at, "quit after kitty pictures")
+        check_teardown(app, quit_at, "quit after kitty pictures", kitty=True)
+    finally:
+        app.close()
+
+
+def set_window(app, rows, cols, width_px, height_px):
+    """Gives the pty a new size in cells and pixels and tells the child, as a
+    terminal window does when it is resized or its font zoomed."""
+    fcntl.ioctl(app.slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, width_px, height_px))
+    os.kill(app.pid, signal.SIGWINCH)
+
+
+def sixel_sizes(stream):
+    """The pixel size of every Sixel picture in `stream`, in order."""
+    return [(int(w), int(h)) for w, h in SIXEL_RASTER.findall(stream)]
+
+
+def wait_sixels(app, start, count, timeout=5.0):
+    """Waits for `count` Sixel pictures after `start` in the stream; their sizes."""
+    end = time.monotonic() + timeout
+    while len(sixel_sizes(app.stream[start:])) < count and time.monotonic() < end:
+        app.pump(0.05)
+    app.idle(0.3)
+    return sixel_sizes(app.stream[start:])
+
+
+def answer_font(app, asked_at, width, height, upto=None):
+    """Answers the font measurement written after `asked_at` (and before `upto`) as a
+    terminal does: a cell of `width` x `height` pixels and the status report, then the
+    device attributes if they were asked for too."""
+    asked = app.stream[asked_at:upto]
+    answer = b"\x1b[6;%d;%dt\x1b[0n" % (height, width)
+    if ATTRIBUTES_REQUEST in asked:
+        answer += ATTRIBUTES_ANSWER
+    os.write(app.master, answer)
+
+
+def scenario_query_sixel_zoom(binary):
+    print("scenario: Sixel pictures follow a font zoom (10x20 to 8x16), measured again")
+    app = App(binary, env=QUERY_ENV)
+    try:
+        fcntl.ioctl(app.slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 800, 480))
+        check(wait_for_query(app) is not None, "the graphics query is written")
+        os.write(app.master, SIXEL_ANSWER)
+        check(app.wait_screen("1. Human vs Human"), "menu renders")
+        check(QUERY_WARNING not in app.screen().text(), "no query warning")
+        app.send(b"1")
+        check(app.wait_screen("White to move"), "Human vs Human starts")
+        app.idle(0.3)
+        before = sixel_sizes(app.stream)
+        # 80x24 gives 5x2 squares, image areas of 3x2 cells: 30x40 pixels at 10x20.
+        check(len(before) == 32, "every piece is a Sixel picture", f"{len(before)} pictures")
+        check(set(before) == {(30, 40)}, "pictures fit 3x2 cells of 10x20", f"sizes {set(before)}")
+        zoomed_at = len(app.stream)
+        # The zoom: more cells, and a window that reports no pixels, so the size can
+        # come only from the answer.
+        set_window(app, 30, 100, 0, 0)
+        check(
+            app.wait_bytes(FONT_QUERY, start=zoomed_at), "the resize asks for the cell size again"
+        )
+        app.idle(0.1)
+        asked = app.stream[zoomed_at:]
+        check(
+            asked.count(b"[16t") == 1
+            and asked.count(FULL_FONT_QUERY) == 1
+            and asked.count(ATTRIBUTES_REQUEST) == 1
+            and b"Gi=31" not in asked,
+            "once, and only for the cell size, the status and the device attributes",
+        )
+        answered_at = len(app.stream)
+        answered = time.monotonic()
+        os.write(app.master, ZOOMED_ANSWER)
+        after = wait_sixels(app, answered_at, 32)
+        # 100x30 gives 7x3 squares, image areas of 5x3 cells: 40x48 pixels at 8x16. At the
+        # old font they would be 50x60, 1.25 columns and 0.75 rows too big.
+        check(len(after) == 32, "the zoom redraws every picture", f"{len(after)} pictures")
+        check(set(after) == {(40, 48)}, "pictures fit 5x3 cells of 8x16", f"sizes {set(after)}")
+        check(time.monotonic() - answered < 2.0, "right after the answer")
+        check("White to move" in app.screen().text(), "the game is still shown")
+        check_no_query_text(app, "board after the zoom")
+
+        # A resize the terminal does not answer keeps the font, after about 1 s. The wait is
+        # timed from the resize: the query is written after it, so the measurement cannot give
+        # up sooner, and noticing the query late cannot shorten the time measured.
+        resized_at = len(app.stream)
+        resized = time.monotonic()
+        set_window(app, ROWS, COLS, 0, 0)
+        check(
+            app.wait_bytes(FONT_QUERY, timeout=2.0, start=resized_at), "the next resize asks again"
+        )
+        kept = wait_sixels(app, resized_at, 32)
+        waited = time.monotonic() - resized
+        check(len(kept) == 32, "the pictures are redrawn", f"{len(kept)} pictures")
+        check(
+            kept and all(w % 8 == 0 and h % 16 == 0 for w, h in kept),
+            "still for 8x16 cells",
+            f"sizes {set(kept)}",
+        )
+        check(waited >= 0.9, "once the query gave up", f"{waited:.2f}s")
+        # Its answers come late, as a terminal sends them: in order, so the device
+        # attributes come last and make crossterm an event. Alone, the dropped cell size
+        # and status reports would leave crossterm's reader waiting for the next input.
+        os.write(app.master, ZOOMED_ANSWER)
+        app.idle(0.3)
+        late_at = len(app.stream)
+        set_window(app, 30, 100, 0, 0)
+        check(
+            app.wait_bytes(FONT_QUERY, timeout=1.0, start=late_at),
+            "the UI still reacts without a key after the late answers (a resize asks at once)",
+        )
+        answer_font(app, late_at, 8, 16)
+        # This measurement still counts the late answers as owed (crossterm read them), so
+        # it waits out the deadline for them before it takes its own.
+        check(len(wait_sixels(app, late_at, 32)) == 32, "and redraws the pictures")
+
+        # A slow link: a resize comes after a measurement gave up but before its answers
+        # arrive. The next measurement skips those and takes its own answer, and leaves
+        # nothing to crossterm that would make it wait for a key.
+        gave_up_at = len(app.stream)
+        set_window(app, ROWS, COLS, 0, 0)
+        check(
+            app.wait_bytes(FONT_QUERY, timeout=2.0, start=gave_up_at),
+            "a resize asks, and gets no answer in time",
+        )
+        wait_sixels(app, gave_up_at, 32)
+        second_at = len(app.stream)
+        set_window(app, 26, 90, 0, 0)
+        check(
+            app.wait_bytes(FONT_QUERY, timeout=2.0, start=second_at),
+            "a resize right after the one that gave up asks again",
+        )
+        # The late answers to the first measurement (still 8x16), then 0.2 s later the
+        # second one's own answer (a zoom to 11x22).
+        answer_font(app, gave_up_at, 8, 16, upto=second_at)
+        app.idle(0.2)
+        own_at = len(app.stream)
+        answer_font(app, second_at, 11, 22)
+        fresh = wait_sixels(app, own_at, 32)
+        check(
+            len(fresh) == 32 and all(w % 11 == 0 and h % 22 == 0 for w, h in fresh),
+            "the pictures follow the measurement's own answer (11x22), not the late one",
+            f"{len(fresh)} pictures, sizes {set(fresh)}",
+        )
+        after_at = len(app.stream)
+        set_window(app, ROWS, COLS, 0, 0)
+        check(
+            app.wait_bytes(FONT_QUERY, timeout=2.0, start=after_at),
+            "the UI reacts to a resize without a key after the late answers",
+        )
+        answer_font(app, after_at, 8, 16)
+        check(len(wait_sixels(app, after_at, 32)) == 32, "and redraws the pictures")
+        app.send(b"g")
+        check(app.wait_screen("glyphs: solid"), "keys work after the unanswered query")
+        app.send(b"q")
+        check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
+        quit_at = len(app.stream)
+        app.send(b"y", settle=0)
+        check(app.wait_exit(), "process exits after y")
+        check(app.status == 0, "exit status 0", f"status {app.status}")
+        check_teardown(app, quit_at, "quit after the zoom")
     finally:
         app.close()
 
@@ -992,6 +1223,7 @@ def main():
     scenario_query_signal(binary)
     scenario_query_hangup(binary)
     scenario_query_kitty(binary)
+    scenario_query_sixel_zoom(binary)
     for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
         scenario_signal(binary, signum)
     scenario_signal(binary, signal.SIGTERM, repeat=2)
````

- [ ] **Step: Run the tests to verify they pass**

Run: `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::` and
`env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test --lib engine::`
Expected: tui:: 443 passed, engine:: 112 passed; `find src -name '*.snap.new'` prints nothing.

Run: `cargo fmt --check` and `cargo clippy --all-targets 2>&1 | grep -A5 -E "src/tui|src/engine|src/main.rs"`
Expected: no output from either.

- [ ] **Step: Run the pseudo-terminal smoke test**

Run: `cargo build && env -u JEV_API_KEY -u TYPESAFE_API_KEY python3 tests/pty_smoke.py --no-build`
Expected: every scenario passes and the script prints `ALL CHECKS PASSED`.

- [ ] **Step: Record the known limits in the handoff**

Add a subsection `### tui-polish (known limits)` under `## Known open issues` in `docs/handoff/HANDOFF.md` with these lines:

```
- src/tui/graphics.rs: iTerm2 gets the Sixel protocol when it answers the sixel probe (the iTerm2 environment hint only decides when no probe answers).
- src/tui/graphics.rs: on terminals that answer more than 1 s late, keys typed while a font measurement waits are lost, and a stale font can stay until the next resize; a late start-up answer from WezTerm or Konsole (no device-attributes probe) can hold crossterm's read until the next input.
- src/tui/graphics.rs: a quit signal during the start-up query leaves the terminal's answers in the tty input queue for the shell.
- src/tui/terminal.rs: the Kitty delete-all command (d=A) may not free unicode-placeholder images on every Kitty version; deleting by image id would be exact.
- src/tui/board.rs: pictures are composited and encoded on the UI thread on the first Image frame and after every square-size change; debug builds may pause briefly.
- src/tui/board.rs: half-block pictures are written in 24-bit colour even on the 256-colour palette (ratatui-image behaviour).
- src/tui/app.rs: quitting while a Jev answer is held (watch mode paused) drops that exchange from the history and the log.
- src/tui/debug.rs: non-UTF-8 values of RCHESS_DEBUG_LOG, XDG_STATE_HOME or HOME are skipped without a warning, and a relative HOME is accepted.
- src/tui/glyphs.rs: RCHESS_IMAGES turns images off only for the value `off`; other values are ignored without a warning.
- src/tui/panels.rs: menu warnings that do not fit at 60x20 are dropped without notice (pre-existing).
```

Also update the stale deferred-minor line about `src/tui/board.rs` and the 7×3 board: after this plan the 120×40 snapshot renders a full board with labels, so replace that line with one saying full-board rendering is covered by the 120×40 snapshot.

- [ ] **Step: Final checks**

Run and quote the outputs in your report:
- `cargo fmt --check`
- `cargo clippy --all-targets 2>&1 | grep -A5 -E "src/tui|src/engine|src/main.rs"` — expect no output
- `env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui::` and `cargo test --lib engine::` — the counts above
- `cargo test --lib core:: && cargo test --test core_properties` — still green
- `cargo test --no-fail-fast --lib 2>&1 | grep -E "test result|^    [a-z_:]+$"` — only the 3 known old-code failures

- [ ] **Step: Ask the user for a manual test in a real terminal**

The agent cannot look at a real terminal. Report to the controller that the user should run `cargo run` in Ghostty and in one other terminal (Kitty, WezTerm or Alacritty) and check (spec 9.6): the TUI fills the terminal and the board grows with the window; pieces are pictures and easy to tell apart (text pieces in Alacritty, pictures after `g` on a large window); `g` cycles Image → Solid → Outline → Ascii; `cargo run -- --debug` then a game against Jev (with `JEV_API_KEY` set) and `d` shows the request and response with the key redacted; the log file `~/.local/state/rchess/jev-debug.jsonl` (or `$XDG_STATE_HOME/rchess/...`) gets one line per request and has mode 0600; quitting leaves the shell normal.

- [ ] **Step: Update the handoff file**

Edit `docs/handoff/HANDOFF.md`. In `## Current`, set:

```
Sub-project: tui-polish | Plan: docs/superpowers/plans/2026-09-27-tui-polish.md
Branch: feat/tui-polish
Last completed task: tui-polish task 8 (Whole-branch review fixes)
Next task: the user runs the manual test in a real terminal (spec 9.6) and merges feat/tui-polish; then plan the cleanup sub-project (spec section 7 step 5)
State: green (except 3 pre-existing old-code test failures)
```

Set `## Verify before continuing` to:

```
env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui:: && cargo test --lib engine::
```

Add at the top of `## Log (newest first)`: `- <today's date> tui-polish task 8 done: Whole-branch review fixes`. Keep every other section. Record under `## Notes / decisions made during work` any decision this plan did not specify.

- [ ] **Step: Refresh the knowledge graph**

Run: `graphify update .`
Expected: completes without error. Never commit `graphify-out/`, `.claude/` or `.superpowers/`.

- [ ] **Step: Commit**

```bash
git add docs/handoff/HANDOFF.md src/engine/jev.rs src/tui/app.rs src/tui/board.rs src/tui/debug.rs src/tui/files.rs src/tui/glyphs.rs src/tui/graphics.rs src/tui/mod.rs src/tui/panels.rs src/tui/pieces.rs src/tui/snapshots/chess__tui__panels__tests__jev_vs_jev_debug_60x20.snap src/tui/terminal.rs src/tui/test_support/harness.rs src/tui/test_support/mod.rs tests/pty_smoke.py
git commit -m "fix(tui): apply the whole-branch review fixes

Co-Authored-By: <the model you are> <noreply@anthropic.com>"
```

Replace `<the model you are>` with your own model name.

---
