# Phase 2 (raster path, filter, PPD) implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans. Steps are test-first.
> The user authorised running all remaining phases without check-ins ("go until you are done"), so this
> plan carries the reverse-engineering results and test contracts rather than every line of code.

**Goal:** a complete job from CUPS raster to printer bytes: layout, provisional colour, dither, weave,
streaming job writer, CUPS raster reader, `rastertotmc3500` filter, PPD. Media saving becomes encodable,
so captures 305–308 join the golden tests.

**Spec:** `docs/superpowers/specs/2026-11-03-tmc3500-linux-driver-design.md` (sections 2, 3, 4, 7).

## Global constraints

Same as phase 1: std-only `tmc3500-core`, `#![forbid(unsafe_code)]`, no panics on external input (CUPS
raster stream, options, PPD), `cargo fmt` + `cargo clippy -D warnings` + full suite before each commit,
commit trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Dev-only crate added: `png`.

## Reverse-engineering results (captures, analysed 2026-11-03)

Units: rows = 1/360 in. Speed columns = 1/360 in; Quality columns = 1/720 in (`ESC(U 04 04 02 a0 05`
sets the *horizontal* unit to 1/720; vertical stays 1/360).

- **Image placement:** image pixel (col,row) = dot (col,row) relative to `ESC(l` and the page top.
  Printable width = min(media width, 1475) dots; printable height = media length (borderless). Rotate 180
  mirrors inside W×H (101: rows 90..179 → 2297..2386, cols 90..449 → 1025..1384 with W=1475, H=2477).
- **Position adjustment:** shift by `mm10_to_dots(|adj|)` (signed), clipping at the edges (107: −0.25 in =
  −91 rows, row −1 clipped; 109 likewise for columns; 106/108 +0.50 in = +180).
- **Borderless off:** pass grid and image start 21 rows lower (110: rows 111..200; 111: passes 21/351/681).
  Assumption: printable height L − 42.
- **Nozzles:** per ink two rows (`c`, `c|0x40`), each line of a block = 2 image rows. Absolute row of
  block line l in a pass at y: `R = y + 2·l − 20 + (c & 0x40 ? 1 : 0)`. Colours per pass in order C(2),
  C'(66), M(1), M'(65), Y(4), Y'(68), K(0), K'(64); a block whose nozzle row has no dots is omitted.
- **Horizontal block extent (per block):** start = first inked column rounded down to 16, end = last inked
  column + 1 rounded up to 8; bpl = (end − start)/4 (2 bits per dot). `ESC($ start)` is sent before the
  block only when the position ≠ 0; CR after every block resets to 0. Quality: start/end computed on the
  pass's 360-dot grid, `ESC($ 2·start + phase)` (phase 0/1).
- **Passes:**
  - *Speed, banding reduction on:* passes at y = 0, 330, 660, …; 175-line blocks (covers rows y−20..y+329).
    Rows y−10..y−1 are feathered between pass p−1 and p (Windows' share for the earlier pass falls
    0.97→0.06 over the 10 rows; we use a linear ramp); otherwise each row belongs to one pass.
  - *Speed, banding off (also Plain coating):* first `ESC(v 20`, then passes every 359; `c` blocks 180
    lines, `c|0x40` blocks 179 lines; no overlap.
  - *Quality Mode1:* two horizontal phases; phase 0 passes at 0, 330, 660…, phase 1 at 10, 340, 670…; each
    phase as Speed-banding with 175-line blocks and 1440-column images split by column parity.
  - *Quality Mode2:* 176-line blocks (352 rows); phase 0 at 0, 96, 268, 440, …(+172); phase 1 at 10, 182,
    354, …(+172). Every row is shared by the passes of its phase covering it (Windows ramps the shares;
    we use triangular weights over the nozzle span).
  - Passes are on a fixed grid from the page top; a pass with no dots is omitted and its `ESC(v` is merged
    into the next one. Each pass ends with `ESC(v` to the next pass only if one follows.
- **Media saving (305–308):** "Lower" keeps the content position, "Lower and upper" shifts content up to
  the first inked row; the page length (`ESC(C`, `ESC(S` height, `ESC(c` bottom) becomes
  max(212, last inked row + 1). Both captures give exactly 212, so the margin below content is an
  assumption to confirm on the printer.

## Tasks

1. **Streaming job writer** (`protocol::job`): `JobWriter<W: Write>` with `begin`, `page`, `end_set`/
   `finish`; `Page` gains `length: Option<u32>` (media saving). `encode_job` becomes a wrapper (golden
   tests unchanged). Tests: wrapper output identical; page length override changes `ESC(C/S/c` only;
   writing pages one by one to a sink never holds more than one page.
2. **`raster::InkPage` + `weave`**: `InkPage { width, height, planes[K,C,M,Y]: Vec<u8> (levels 0..3) }`;
   `weave(page, WeaveMode, row_offset) -> Vec<u8>`. Tests: block-header structure (y, x, colour, lines,
   bpl and `ESC(v` sequence) equals captures 000, 002, 003, 106, 110, 111, 411 (Speed), 314, 315
   (Quality) for synthetic ink pages with the same inked areas; every dot appears in exactly one pass;
   RLE decode of every block round-trips.
3. **Provisional colour + dither** (`color`, `dither`): RGB 8-bit → K,C,M,Y 16-bit (naive with rich black,
   gamma/brightness/colour-mode hooks; replaced by fitted tables in phase 3); ordered 2-bit dither with
   three drop sizes. Tests: white → no dots; 100 % solids → all level 3 with no holes; K-only option;
   monotonic coverage on ramps; a bar's edges land on exact columns with no stray dots outside it.
4. **Layout** (`layout`): printable area, position adjustment, rotate 180, borderless offset, media
   saving. Tests use the capture numbers above.
5. **CUPS raster reader** (`cupsraster`): sync words `RaSt`/`tSaR` (v1), `RaS2`/`2SaR` (v2, compressed),
   `RaS3`/`3SaR` (v3), 1796-byte header, CUPS line/pixel RLE, 8-bit RGB (and grey/K as fallback). Hard
   limits on width/height/bytes. Tests: hand-built streams in each variant; truncated/garbage input →
   error (proptest, never panics).
6. **Filter `rastertotmc3500`** (new crate): CUPS argv, defaults from the PPD (`PPD` env), options from
   argv[5] override, page size from options or raster header, resolution check (nearest-neighbour
   rescale if the raster's resolution differs), per-page pipeline, copies and collate (collate buffers one
   set of encoded pages), `PAGE:` accounting, `ERROR:` messages and exit 1 on bad input. Integration test
   runs the binary on synthetic raster and decodes the output.
7. **PPD** `packaging/ppd/tmc3500.ppd`: every `Tm*` option with the values of `options`, defaults equal
   to `JobConfig::default()`, custom page size, `TmQuality` code sets `HWResolution`, `cupsFilter2`,
   `cupsManualCopies False`. Test: parse the PPD and feed every choice through `JobConfig::from_cups`.
8. **Golden raster structure**: render the kit test images (`testdata/images/t*.png`) through layout →
   colour → dither → weave and compare block structure with every capture (including 305–308 now);
   remove the phase-1 skips.

## Review focus

1. Hostile CUPS raster headers (huge width × height, zero sizes, bit depths we don't support, truncated
   pages) → error, no panic, bounded memory.
2. Raster resolution not matching the selected quality (e.g. 300 dpi from a generic app) → rescaled,
   never misplaced.
3. Content beyond the printable area (negative position adjustment, oversize pages) → clipped.
4. Collated copies of long jobs → memory bounded by one set of compressed pages.
5. Empty pages and all-white pages with Skip Blank on and off.
