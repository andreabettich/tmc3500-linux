# Phase 3 (colour) implementation plan

> REQUIRED SUB-SKILL: superpowers:executing-plans. Test-first. User authorised continuous execution.

**Goal:** colour output fitted to the Windows driver. Code now; tables once the VM colour session
(`tmc3500_colour_kit/`) has produced chart captures.

**Spec:** §4 (colour and barcode quality). Constraint unchanged: `tmc3500-core` pure, no panics on input.

## Model
Windows' output is measured per ink and per drop size: for a patch, the fraction of dots that are small,
medium and large. The colour stage therefore produces, per pixel and ink, three fractions `(f1, f2, f3)`;
the dither places drop size 3 where the threshold `t < f3`, 2 where `t < f3+f2`, 1 where `t < f3+f2+f1`.
The provisional formula maps to the same form, so behaviour is unchanged until tables exist.

Tables: 9×9×9 RGB grid (the chart cube), tetrahedral interpolation, one per
`{coating}-{quality}-{mode}` (`mode` = preferred | standard | gamma18 | off; Quality1/Quality2 →
`q1`/`q2`), file format `TMCLUT1` + n + n³×4 inks×3 fractions (u16 LE). Loaded by the filter from
`/usr/share/tmc3500/colour/` (override `TMC3500_COLOUR_DIR`); missing table → provisional formula with an
`INFO:` line. Ink adjustment and brightness stay formula-based on top of the table.

## Tasks
1. Fraction-based dither (`dither::Drops`, `level_from_fractions`); provisional converter returns fractions.
   Existing dither tests keep passing.
2. `color::Lut` (parse, validate, tetrahedral lookup) and `Converter::with_lut`. Tests: exact at nodes,
   linear between nodes, corrupt files rejected.
3. `raster::reassemble(job) -> pages of InkPage` (inverse weave, Speed and Quality columns).
   Test: reassemble(weave(page)) == page through a full job; Windows capture 000 reassembles to the
   black rectangle's extent.
4. `xtask fit-colour <capture.prn> <layout.json> <out.lut>`: measure the cube patches, write the table,
   print the residuals of grey and ink ramps. Test: chart rendered by our own pipeline → fit → the table
   reproduces the provisional converter at every node (within dither sampling tolerance).
5. Filter loads the table for the job's coating/quality/mode.

Open until the captures arrive: the fitted tables themselves, the K-only behaviour, error diffusion for
photos (the content is barcodes, text and flat brand colours; ordered dither is kept).
