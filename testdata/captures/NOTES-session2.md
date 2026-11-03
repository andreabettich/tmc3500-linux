# TM-C3500 session 2 (colour) capture notes

One line per capture: id | image | exact UI path and value | verification. Everything not named is at the driver default.
Each `<id>` has `<id>.prn`, `<id>.devmode.dat` (per-user DEVMODE before printing), `<id>.printticket.xml` and
`<id>.drvlog.txt` (this job's lines from the driver's own log; the `DSTD` line lists the effective settings).

How the captures were made (same method as session 1):
- The dialog was driven by `scripts\run_matrix.ps1` with `scripts\colour_specs.ps1`, using Win32 window messages
  instead of manual clicks.
- **Defaults before every capture.** The driver has no "Restore Defaults" button in the main dialog. The
  per-user DEVMODE was reset from the untouched global DEVMODE, then checked byte-identical to the session-1
  baseline (`_baseline_user.devmode.dat`, copied here). Driver-wide preferences (vacuum enable, Skip Blank Page)
  were undone after their capture.
- **Verification.** After OK, the DEVMODE was diffed against the baseline (must differ, except for the defaults
  captures) and against the reference capture. The printer data was diffed too. The driver log's settings line
  was diffed against the reference.
- **Custom media.** Coating, Print Quality and the Advanced colour settings are greyed out for the predefined
  media "108 x 174.8 mm - Die-cut Label". Those captures therefore select General › Media Name = Custom, whose
  values are the same 4.25 x 6.88 in die-cut / Matte / Speed. `713_chart_custom_matte_speed` captures Custom
  alone as their reference. Custom is **not** byte-identical to the predefined media: the page-length command `ESC ( C` is 2519 dots
  instead of 2520 (log: PapSizeID 2050→256, PapSizeW 1531→1530, LblGap 43→42), besides the TI/JH bytes.
  The same holds in session 1 (000 vs 210), where NOTES.md wrongly said only TI/JH differ. So compare
  Custom-based charts with 713, not 701.
- **Vacuum "manual N"** = slider position N on Options › Change Vacuum Setting (positions 0..9, default 9).
  Session 1 captured positions 9, 4 and 0 (ids 610-612).
- **Extra references** (not in the brief): 713 (above), 790/791/792 (defaults with t1, t3, t4 x 3 pages).

## Skipped
- `723_chart_black_only`: the driver has no "black ink only" / greyscale option (see session-1
  SETTINGS_INVENTORY.md; the colour options are only Color Controls / Disable Color Correction, Mode, Gamma,
  Ink Profile, Brightness).

## Captures
| id | image | setting changed | verification |
|---|---|---|---|
| 713_chart_custom_matte_speed | c1_colour_chart.png | General › Media Name = Custom (values unchanged: 4.25 x 6.88 in die-cut, Matte, Speed). Extra reference for 700-724 | DEVMODE diff: 18 B vs baseline, 18 B vs 701_chart_matte_speed; printer data diff 0 B. Driver log vs 701_chart_matte_speed: PapSizeID: 2050->256; PapSizeW: 1531->1530; LblGap: 43->42 (filled in after 701 was captured). .prn differs from 701 at 0x7F, 0x80, 0x8F (TI timestamp / JH job id) and 0x10131 (`ESC ( C` page length 0x09D8=2520 → 0x09D7=2519 dots) |
| 700_chart_plain_speed | c1_colour_chart.png | General › Media Name = Custom › Media Coating Type = Plain (Print Quality = Speed, the only choice) | DEVMODE diff: 24 B vs baseline, 8 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: MedID: 2003->2001; BandingReduction: 1->0 |
| 702_chart_matte_q1 | c1_colour_chart.png | General › Media Name = Custom › Print Quality = Quality (Mode1) (coating Matte) | DEVMODE diff: 24 B vs baseline, 6 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: BandingReduction: 1->0 |
| 703_chart_matte_q2 | c1_colour_chart.png | General › Media Name = Custom › Print Quality = Quality (Mode2) (coating Matte) | DEVMODE diff: 24 B vs baseline, 6 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: no change **[OK+LOG_UNCHANGED]** |
| 704_chart_synthetic_speed | c1_colour_chart.png | General › Media Name = Custom › Media Coating Type = Synthetic, Print Quality = Speed | DEVMODE diff: 21 B vs baseline, 5 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: MedID: 2003->2007 |
| 705_chart_synthetic_q1 | c1_colour_chart.png | General › Media Name = Custom › Media Coating Type = Synthetic, Print Quality = Quality (Mode1) | DEVMODE diff: 27 B vs baseline, 8 B vs 704_chart_synthetic_speed; printer data diff 0 B. Driver log vs 704_chart_synthetic_speed: BandingReduction: 1->0 |
| 706_chart_synthetic_q2 | c1_colour_chart.png | General › Media Name = Custom › Media Coating Type = Synthetic, Print Quality = Quality (Mode2) | DEVMODE diff: 27 B vs baseline, 8 B vs 704_chart_synthetic_speed; printer data diff 0 B. Driver log vs 704_chart_synthetic_speed: no change **[OK+LOG_UNCHANGED]** |
| 707_chart_glossy_speed | c1_colour_chart.png | General › Media Name = Custom › Media Coating Type = Glossy, Print Quality = Speed | DEVMODE diff: 21 B vs baseline, 5 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: MedID: 2003->2012 |
| 708_chart_glossy_q1 | c1_colour_chart.png | General › Media Name = Custom › Media Coating Type = Glossy, Print Quality = Quality (Mode1) | DEVMODE diff: 27 B vs baseline, 8 B vs 707_chart_glossy_speed; printer data diff 0 B. Driver log vs 707_chart_glossy_speed: BandingReduction: 1->0 |
| 709_chart_glossy_q2 | c1_colour_chart.png | General › Media Name = Custom › Media Coating Type = Glossy, Print Quality = Quality (Mode2) | DEVMODE diff: 27 B vs baseline, 8 B vs 707_chart_glossy_speed; printer data diff 0 B. Driver log vs 707_chart_glossy_speed: no change **[OK+LOG_UNCHANGED]** |
| 710_chart_wristband_speed | c1_colour_chart.png | General › Media Name = Custom › Media Coating Type = Wristband, Print Quality = Speed | DEVMODE diff: 21 B vs baseline, 5 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: MedID: 2003->2011 |
| 711_chart_wristband_q1 | c1_colour_chart.png | General › Media Name = Custom › Media Coating Type = Wristband, Print Quality = Quality (Mode1) | DEVMODE diff: 27 B vs baseline, 8 B vs 710_chart_wristband_speed; printer data diff 0 B. Driver log vs 710_chart_wristband_speed: BandingReduction: 1->0 |
| 712_chart_matte_speed_banding_off | c1_colour_chart.png | General › Media Name = Custom › Advanced... › Banding Reduction = off (Matte, Speed) | DEVMODE diff: 20 B vs baseline, 2 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: BandingReduction: 1->0 |
| 701_chart_matte_speed | c1_colour_chart.png | all driver defaults (Media Name = 108 x 174.8 mm - Die-cut Label, Matte, Speed) | DEVMODE diff: 0 B vs baseline, n/a B vs 701_chart_matte_speed; printer data diff 0 B. Driver log vs 701_chart_matte_speed: no change |
| 790_default_t1 | t1_black_rect.png | all driver defaults (reference for 730-742, 748) | DEVMODE diff: 0 B vs baseline, n/a B vs 790_default_t1; printer data diff 0 B. Driver log vs 790_default_t1: no change |
| 791_default_t3 | t3_full_test.png | all driver defaults (reference for 743, 746) | DEVMODE diff: 0 B vs baseline, n/a B vs 791_default_t3; printer data diff 0 B. Driver log vs 791_default_t3: no change |
| 792_default_t4_pages3 | t4_blank.png | all driver defaults, 3-page document (reference for 745) | DEVMODE diff: 0 B vs baseline, n/a B vs 792_default_t4_pages3; printer data diff 0 B. Driver log vs 792_default_t4_pages3: no change |
| 720_chart_color_standard | c1_colour_chart.png | General › Media Name = Custom › Advanced... › Color Controls › Mode = EPSON Standard | DEVMODE diff: 19 B vs baseline, 1 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: ColAdj2(14:Biz,1:Std,3:No): 14->1 |
| 721_chart_gamma_18 | c1_colour_chart.png | General › Media Name = Custom › Advanced... › Color Controls › Gamma = 1.8 | DEVMODE diff: 19 B vs baseline, 1 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: Gamma: 220->180 |
| 722_chart_color_off | c1_colour_chart.png | General › Media Name = Custom › Advanced... › Color Preset = Disable Color Correction | DEVMODE diff: 20 B vs baseline, 2 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: ColAdj1(0:Drv,4:No): 0->4; ColAdj2(14:Biz,1:Std,3:No): 14->3 |
| 724_chart_inkadj_bkcmy_-3_bk_-3_bright_+1 | c1_colour_chart.png | General › Media Name = Custom › Advanced... › Ink Profile and Brightness Adjustment = on; Ink Profile › BkCMY = -3 (slider pos 4 of 1..7), Bk = -3 (pos 4); Brightness = +1 | DEVMODE diff: 23 B vs baseline, 5 B vs 713_chart_custom_matte_speed; printer data diff 0 B. Driver log vs 713_chart_custom_matte_speed: no change **[OK+LOG_UNCHANGED]** |
| 730_vacuum_1 | t1_black_rect.png | Driver Utilities › Driver Preferences › Enable Platen Vacuum Manual Adjustment = on; Options › Platen Vacuum Manual Adjustment = on › Change Vacuum Setting... slider position 1 (0..9; default 9; session 1 had 9, 4, 0) | DEVMODE diff: 13 B vs baseline, 13 B vs 790_default_t1; printer data diff 0 B. Driver log vs 790_default_t1: Suction: 90->40; EnableSuction: 0->1 |
| 731_vacuum_2 | t1_black_rect.png | Driver Utilities › Driver Preferences › Enable Platen Vacuum Manual Adjustment = on; Options › Platen Vacuum Manual Adjustment = on › Change Vacuum Setting... slider position 2 (0..9; default 9; session 1 had 9, 4, 0) | DEVMODE diff: 13 B vs baseline, 13 B vs 790_default_t1; printer data diff 0 B. Driver log vs 790_default_t1: Suction: 90->50; EnableSuction: 0->1 |
| 732_vacuum_3 | t1_black_rect.png | Driver Utilities › Driver Preferences › Enable Platen Vacuum Manual Adjustment = on; Options › Platen Vacuum Manual Adjustment = on › Change Vacuum Setting... slider position 3 (0..9; default 9; session 1 had 9, 4, 0) | DEVMODE diff: 13 B vs baseline, 13 B vs 790_default_t1; printer data diff 0 B. Driver log vs 790_default_t1: Suction: 90->60; EnableSuction: 0->1 |
| 733_vacuum_5 | t1_black_rect.png | Driver Utilities › Driver Preferences › Enable Platen Vacuum Manual Adjustment = on; Options › Platen Vacuum Manual Adjustment = on › Change Vacuum Setting... slider position 5 (0..9; default 9; session 1 had 9, 4, 0) | DEVMODE diff: 13 B vs baseline, 13 B vs 790_default_t1; printer data diff 0 B. Driver log vs 790_default_t1: Suction: 90->70; EnableSuction: 0->1 |
| 734_vacuum_6 | t1_black_rect.png | Driver Utilities › Driver Preferences › Enable Platen Vacuum Manual Adjustment = on; Options › Platen Vacuum Manual Adjustment = on › Change Vacuum Setting... slider position 6 (0..9; default 9; session 1 had 9, 4, 0) | DEVMODE diff: 13 B vs baseline, 13 B vs 790_default_t1; printer data diff 0 B. Driver log vs 790_default_t1: Suction: 90->75; EnableSuction: 0->1 |
| 735_vacuum_7 | t1_black_rect.png | Driver Utilities › Driver Preferences › Enable Platen Vacuum Manual Adjustment = on; Options › Platen Vacuum Manual Adjustment = on › Change Vacuum Setting... slider position 7 (0..9; default 9; session 1 had 9, 4, 0) | DEVMODE diff: 13 B vs baseline, 13 B vs 790_default_t1; printer data diff 0 B. Driver log vs 790_default_t1: Suction: 90->80; EnableSuction: 0->1 |
| 736_vacuum_8 | t1_black_rect.png | Driver Utilities › Driver Preferences › Enable Platen Vacuum Manual Adjustment = on; Options › Platen Vacuum Manual Adjustment = on › Change Vacuum Setting... slider position 8 (0..9; default 9; session 1 had 9, 4, 0) | DEVMODE diff: 13 B vs baseline, 13 B vs 790_default_t1; printer data diff 0 B. Driver log vs 790_default_t1: Suction: 90->85; EnableSuction: 0->1 |
| 740_label_width_4.10 | t1_black_rect.png | General › Media Name = Custom › Media Width = 4.10 in (Length 6.88, Die-cut Label (Gap)) | DEVMODE diff: 28 B vs baseline, 28 B vs 790_default_t1; printer data diff 0 B. Driver log vs 790_default_t1: PapSizeID: 2050->256; PapSizeW: 1531->1475; LblSizeW: 1587->1532; LblGap: 43->42; SrcPapSizeW: 1531->1475; SrcPapMgnL: 28->0; SrcPapMgnR: 28->0 |
| 741_label_width_4.15 | t1_black_rect.png | General › Media Name = Custom › Media Width = 4.15 in (Length 6.88, Die-cut Label (Gap)) | DEVMODE diff: 27 B vs baseline, 11 B vs 740_label_width_4.10; printer data diff 0 B. Driver log vs 740_label_width_4.10: PapSizeW: 1475->1493; LblSizeW: 1532->1550; SrcPapSizeW: 1475->1494; SrcPapMgnL: 0->9; SrcPapMgnR: 0->10 |
| 742_paper_width_2.00 | t1_black_rect.png | General › Media Name = Custom › Media Form = Continuous Paper, Media Width = 2.00 in (Length 6.88) | DEVMODE diff: 39 B vs baseline, 19 B vs 740_label_width_4.10; printer data diff 0 B. Driver log vs 740_label_width_4.10: PapType(0,1,3,5,6,7,8): 0->5; PapSizeW: 1475->720; LblSizeW: 1532->720; LblGap: 42->0; EdgeL: 28->0; EdgeR: 28->0; SrcPapSizeW: 1475->720 |
| 743_borderless_off_q1 | t3_full_test.png | General › Media Name = Custom › Print Quality = Quality (Mode1); Options › Layout › Borderless Printing = off | DEVMODE diff: 20 B vs baseline, 20 B vs 791_default_t3; printer data diff 0 B. Driver log vs 791_default_t3: PapSizeID: 2050->256; PapSizeW: 1531->1530; LblGap: 43->42; SrcPapMgnT: 0->21; SrcPapMgnB: 0->21; BandingReduction: 1->0 |
| 744_borderless_off_q2 | t3_full_test.png | General › Media Name = Custom › Print Quality = Quality (Mode2); Options › Layout › Borderless Printing = off | DEVMODE diff: 20 B vs baseline, 3 B vs 743_borderless_off_q1; printer data diff 0 B. Driver log vs 743_borderless_off_q1: BandingReduction: 0->1 |
| 745_skip_blank_pages3 | t4_blank.png | Driver Utilities › Driver Preferences › Skip Blank Page = on; 3-page document | DEVMODE diff: 11 B vs baseline, 11 B vs 792_default_t4_pages3; printer data diff 0 B. Driver log vs 792_default_t4_pages3: SaveBlank: 0->1 |
| 746_save_lowerupper_t3 | t3_full_test.png | General › Media Name = Custom › Media Form = Continuous Label (4.25 x 6.88 in) › Media Saving = Eliminate Lower And Upper Margin | DEVMODE diff: 20 B vs baseline, 20 B vs 791_default_t3; printer data diff 0 B. Driver log vs 791_default_t3: PapSizeID: 2050->256; PapType(0,1,3,5,6,7,8): 0->3; PapSizeW: 1531->1530; LblGap: 43->0; Save(1:B,2:TB,3:Off): 3->2 |
| 747_save_lower_t3 | t3_full_test.png | General › Media Name = Custom › Media Form = Continuous Label (4.25 x 6.88 in) › Media Saving = Eliminate Lower Margin | DEVMODE diff: 19 B vs baseline, 1 B vs 746_save_lowerupper_t3; printer data diff 0 B. Driver log vs 746_save_lowerupper_t3: Save(1:B,2:TB,3:Off): 2->1 |
| 748_rotate180_posadj_v+0.50 | t1_black_rect.png | Options › Orientation › Rotate 180 = on; Options › Layout › Print Position Adjustment › Vertical = 0.50 | DEVMODE diff: 14 B vs baseline, 14 B vs 790_default_t1; printer data diff 0 B. Driver log vs 790_default_t1: FlipV: 0->1; PosAdjV: 0->127 |

## Observations
- All 37 captures (33 from the brief with 723 skipped, plus 713, 790, 791, 792 as references) have status OK. The
  defaults captures (701, 790-792) have a DEVMODE byte-identical to the baseline: the dialog is not opened
  for them, because pressing OK on an unchanged dialog still rewrites ~10 DEVMODE bytes. Every other capture
  changed the DEVMODE, which was checked before printing. The printer data never changed.
- **Not in the driver log (expected):** Quality (Mode2) (703/706/709) and the ink-adjustment sliders (724).
  Their DEVMODE diffs (6-8 / 5 bytes) and the output confirm them. Mode1 shows in the log only through
  BandingReduction 1→0, and Mode2 keeps it at 1 (744 vs 743).
- **Chart size by quality** (Matte via Custom): Speed 659 KB, Mode1 1.15 MB, Mode2 1.70 MB. Plain/Synthetic/
  Glossy/Wristband are in the same ranges. Coatings are logged as MedID: Plain 2001, Matte 2003, Synthetic
  2007, Wristband 2011, Glossy 2012.
- **Vacuum slider → logged Suction** (with session 1): pos 0=30, 1=40, 2=50, 3=60, 4=65, 5=70, 6=75, 7=80,
  8=85, 9=90 (default).
- **Widths:** 4.10 in → PapSizeW 1475, 4.15 in → 1493 (1/360 in). From 4.15 in the driver adds side margins
  SrcPapMgnL/R 9/10 (0 at 4.10). Continuous paper 2.00 in → PapSizeW 720, LblGap 0, EdgeL/R 0.
- **745 (Skip Blank Page, 3 blank pages)** is still a job, 3 bytes shorter than 792, with 1 form-feed byte
  instead of 4 (not analysed further). The log shows SaveBlank 0→1.
- **746/747 (continuous label + Media Saving)** give .prn files the same size as 791 (the t3 image is small,
  so no margin is removed from the raster). The settings are logged as Save=2 / Save=1, PapType=3.
- **748:** FlipV 0→1 and PosAdjV 0→127 (0.1 mm units), as in session 1 (101 + 106).
