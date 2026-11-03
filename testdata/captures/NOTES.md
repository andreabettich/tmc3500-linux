# TM-C3500 capture notes

One line per capture: id | image | exact setting (tab › group › control = value) | notes.
Everything not named is at the driver default (see SETTINGS_INVENTORY.md). Each capture `<id>` has:
`<id>.prn` (raw printer data), `<id>.devmode.dat` (per-user DEVMODE before printing, `printui /Ss … u`),
`<id>.printticket.xml`, and `<id>.drvlog.txt`. The `.drvlog.txt` file holds this job's lines from the
driver's own log (`C:\ProgramData\EPSON\EPSON TM-C3500 Ver2\EPSON TM-C3500 Ver2.log`, on by default). Its
`DSTD` line lists every effective setting (MedID, PapSizeID, LblGap, BiDi, AutoCut, Suction, ...) in
1/360-inch units.

## General observations
- Output is ESC/P raster: `ESC 01 @EJL 1284.4` → `ESC @` → `ESC ( R … REMOTE1` (EX, SV, TI, JS, JH …) → `ESC ( d` …
- Run-to-run variation (000 vs 001, identical settings): exactly 3 bytes differ.
  - 0x7F–0x80: the minute and second of the `REMOTE1 TI` timestamp (`TI 08 00 00 YY YY MM DD hh mm ss`, year
    big-endian 0x07EA = 2026, local time).
  - 0x8F: a byte in the `REMOTE1 JH` command that tracks the spooler job ID (job 2 → 02, job 3 → 03).
- Default driver log values: LblGap=43 (≈3 mm), EdgeL=EdgeR=28 (≈2 mm), PapSizeW=1531, PapSizeL=2477,
  LblSizeW=1587, Suction=90, SrcRes=360×360.

## Captures
| id | image | setting changed | notes |
|---|---|---|---|
| 000_default_t1 | t1_black_rect.png | none (all defaults) | 102382 B; JobID=2 |
| 001_default_t1_again | t1_black_rect.png | none (repeat of 000) | 102382 B; JobID=3; differs from 000 only in TI timestamp + JH job id |
| 002_default_t2 | t2_cmyk_bars.png | none | 160092 B |
| 003_default_t3 | t3_full_test.png | none | 236657 B |
| 004_default_t4_blank | t4_blank.png | none | 65954 B (blank page still produces a full job) |
| 005_default_t1_copies3 | t1_black_rect.png | application copies = 3 (PrintDocument.PrinterSettings.Copies; dialog untouched) | 175372 B; log Copies=3, HSCopy=1. User DEVMODE unchanged, as expected (copies are per job) |
| 006_default_t1_pages2 | t1_black_rect.png | 2-page document (same image on both pages) | 138877 B |

All 000–006 `.devmode.dat` files are byte-identical to `_baseline_user.devmode.dat`, as expected for
default runs.

## Step 4: one-setting-at-a-time matrix
Driven by `scripts\run_matrix.ps1` (specs in `scripts\matrix_specs.ps1`). Before each capture the per-user DEVMODE is reset to the default (copied from the untouched global DEVMODE) and checked byte-identical to `_baseline_user.devmode.dat`. The change is then made in the dialog and confirmed with OK. Verification: DEVMODE diff vs baseline and vs the reference capture, printer-data diff, and the driver log's `DSTD` settings line diffed against the reference capture. Media, quality and colour controls are locked for the predefined media, so those captures use Media Name = Custom (captured alone as 210/211) as their reference.

| id | image | setting changed | verification |
|---|---|---|---|
| 100_orient_landscape | t1_black_rect.png | Options › Orientation = Landscape | DEVMODE diff: 13 B vs baseline, 13 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: Orient(1:P,2:L): 1->2 |
| 101_rotate180_on | t1_black_rect.png | Options › Orientation › Rotate 180 = on | DEVMODE diff: 13 B vs baseline, 13 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: FlipV: 0->1 |
| 102_copies3_dialog | t1_black_rect.png | Options › Number of Prints › Copies = 3 | DEVMODE diff: 13 B vs baseline, 13 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: Copies: 1->3 |
| 103_collate_on | t1_black_rect.png | Options › Number of Prints › Collate = on (copies 1) | DEVMODE diff: 13 B vs baseline, 13 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: no change **[OK+LOG_UNCHANGED]** |
| 104_collate_on_copies3 | t1_black_rect.png | Options › Copies = 3 AND Collate = on (2 changes; ref 102) | DEVMODE diff: 14 B vs baseline, 1 B vs 102_copies3_dialog; printer data diff 0 B. Driver log vs 102_copies3_dialog: Collate: 0->1 |
| 105_reverse_order_pages2 | t1_black_rect.png | Options › Number of Prints › Reverse Order = on (2-page doc; ref 006) | DEVMODE diff: 13 B vs baseline, 13 B vs 006_default_t1_pages2; printer data diff 0 B. Driver log vs 006_default_t1_pages2: Reverse: 0->1 |
| 106_posadj_vert_+0.50 | t1_black_rect.png | Options › Layout › Print Position Adjustment › Vertical = 0.50 | DEVMODE diff: 14 B vs baseline, 14 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: PosAdjV: 0->127 |
| 107_posadj_vert_-0.25 | t1_black_rect.png | Options › Layout › Print Position Adjustment › Vertical = -0.25 | DEVMODE diff: 20 B vs baseline, 20 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: PosAdjV: 0->-64 |
| 108_posadj_horiz_+0.50 | t1_black_rect.png | Options › Layout › Print Position Adjustment › Horizontal = 0.50 | DEVMODE diff: 14 B vs baseline, 14 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: PosAdjH: 0->127 |
| 109_posadj_horiz_-0.25 | t1_black_rect.png | Options › Layout › Print Position Adjustment › Horizontal = -0.25 | DEVMODE diff: 20 B vs baseline, 20 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: PosAdjH: 0->-64 |
| 110_borderless_off | t1_black_rect.png | Options › Layout › Borderless Printing = off | DEVMODE diff: 8 B vs baseline, 8 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: SrcPapMgnT: 0->21; SrcPapMgnB: 0->21 |
| 111_borderless_off_t3 | t3_full_test.png | Options › Layout › Borderless Printing = off | DEVMODE diff: 8 B vs baseline, 8 B vs 003_default_t3; printer data diff 0 B. Driver log vs 003_default_t3: SrcPapMgnT: 0->21; SrcPapMgnB: 0->21 |
| 200_notify_beep_page | t1_black_rect.png | General › Notification = Beep at Page End | DEVMODE diff: 13 B vs baseline, 13 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: Buzzer: 0->2 |
| 201_notify_beep_job | t1_black_rect.png | General › Notification = Beep at Job End | DEVMODE diff: 13 B vs baseline, 13 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: Buzzer: 0->1 |
| 210_media_custom_t1 | t1_black_rect.png | General › Media Name = Custom (values unchanged: 4.25x6.88 in, Die-cut Gap, Matte, Speed, AutoCut every page) | DEVMODE diff: 19 B vs baseline, 19 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: PapSizeID: 2050->256; PapSizeW: 1531->1530; LblGap: 43->42 |
| 211_media_custom_t3 | t3_full_test.png | General › Media Name = Custom (values unchanged) | DEVMODE diff: 19 B vs baseline, 19 B vs 003_default_t3; printer data diff 0 B. Driver log vs 003_default_t3: PapSizeID: 2050->256; PapSizeW: 1531->1530; LblGap: 43->42 |
| 301_form_blackmark_label | t1_black_rect.png | General › Media Name = Custom › Media Form = Die-cut Label (BlackMark) | DEVMODE diff: 23 B vs baseline, 4 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: PapType(0,1,3,5,6,7,8): 0->1 |
| 302_form_continuous_label | t1_black_rect.png | General › Media Name = Custom › Media Form = Continuous Label | DEVMODE diff: 21 B vs baseline, 3 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: PapType(0,1,3,5,6,7,8): 0->3; LblGap: 42->0 |
| 303_form_continuous_paper | t1_black_rect.png | General › Media Name = Custom › Media Form = Continuous Paper | DEVMODE diff: 25 B vs baseline, 7 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: PapType(0,1,3,5,6,7,8): 0->5; LblSizeW: 1587->1530; LblGap: 42->0; EdgeL: 28->0; EdgeR: 28->0 |
| 304_form_continuous_paper_blackmark | t1_black_rect.png | General › Media Name = Custom › Media Form = Continuous Paper(BlackMark) | DEVMODE diff: 27 B vs baseline, 9 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: PapType(0,1,3,5,6,7,8): 0->6; LblSizeW: 1587->1530; LblGap: 42->0; EdgeL: 28->0; EdgeR: 28->0 |
| 305_save_lower_contlabel | t1_black_rect.png | Custom › Media Form = Continuous Label › Media Saving = Eliminate Lower Margin (ref 302) | DEVMODE diff: 20 B vs baseline, 1 B vs 302_form_continuous_label; printer data diff 0 B. Driver log vs 302_form_continuous_label: Save(1:B,2:TB,3:Off): 3->1 |
| 306_save_lowerupper_contlabel | t1_black_rect.png | Custom › Media Form = Continuous Label › Media Saving = Eliminate Lower And Upper Margin (ref 302) | DEVMODE diff: 21 B vs baseline, 2 B vs 302_form_continuous_label; printer data diff 0 B. Driver log vs 302_form_continuous_label: Save(1:B,2:TB,3:Off): 3->2 |
| 307_save_lower_contpaper | t1_black_rect.png | Custom › Media Form = Continuous Paper › Media Saving = Eliminate Lower Margin (ref 303) | DEVMODE diff: 24 B vs baseline, 1 B vs 303_form_continuous_paper; printer data diff 0 B. Driver log vs 303_form_continuous_paper: Save(1:B,2:TB,3:Off): 3->1 |
| 308_save_lowerupper_contpaper | t1_black_rect.png | Custom › Media Form = Continuous Paper › Media Saving = Eliminate Lower And Upper Margin (ref 303) | DEVMODE diff: 25 B vs baseline, 2 B vs 303_form_continuous_paper; printer data diff 0 B. Driver log vs 303_form_continuous_paper: Save(1:B,2:TB,3:Off): 3->2 |
| 310_coating_plain | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Plain | DEVMODE diff: 25 B vs baseline, 8 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: MedID: 2003->2001; BandingReduction: 1->0 |
| 311_coating_synthetic | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Synthetic | DEVMODE diff: 22 B vs baseline, 5 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: MedID: 2003->2007 |
| 312_coating_wristband | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Wristband | DEVMODE diff: 22 B vs baseline, 5 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: MedID: 2003->2011 |
| 313_coating_glossy | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Glossy | DEVMODE diff: 22 B vs baseline, 5 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: MedID: 2003->2012 |
| 314_quality_matte_mode1 | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Matte › Print Quality = Quality (Mode1) | DEVMODE diff: 25 B vs baseline, 6 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: BandingReduction: 1->0 |
| 315_quality_matte_mode2 | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Matte › Print Quality = Quality (Mode2) | DEVMODE diff: 25 B vs baseline, 6 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: no change **[OK+LOG_UNCHANGED]** |
| 316_quality_synthetic_mode1 | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Synthetic › Print Quality = Quality (Mode1) (coating is a 2nd change; ref = that coating) | DEVMODE diff: 28 B vs baseline, 8 B vs 311_coating_synthetic; printer data diff 0 B. Driver log vs 311_coating_synthetic: BandingReduction: 1->0 |
| 317_quality_synthetic_mode2 | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Synthetic › Print Quality = Quality (Mode2) (coating is a 2nd change; ref = that coating) | DEVMODE diff: 28 B vs baseline, 8 B vs 311_coating_synthetic; printer data diff 0 B. Driver log vs 311_coating_synthetic: no change **[OK+LOG_UNCHANGED]** |
| 318_quality_wristband_mode1 | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Wristband › Print Quality = Quality (Mode1) (coating is a 2nd change; ref = that coating) | DEVMODE diff: 28 B vs baseline, 8 B vs 312_coating_wristband; printer data diff 0 B. Driver log vs 312_coating_wristband: BandingReduction: 1->0 |
| 319_quality_glossy_mode1 | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Glossy › Print Quality = Quality (Mode1) (coating is a 2nd change; ref = that coating) | DEVMODE diff: 28 B vs baseline, 8 B vs 313_coating_glossy; printer data diff 0 B. Driver log vs 313_coating_glossy: BandingReduction: 1->0 |
| 320_quality_glossy_mode2 | t3_full_test.png | General › Media Name = Custom › Media Coating Type = Glossy › Print Quality = Quality (Mode2) (coating is a 2nd change; ref = that coating) | DEVMODE diff: 28 B vs baseline, 8 B vs 313_coating_glossy; printer data diff 0 B. Driver log vs 313_coating_glossy: no change **[OK+LOG_UNCHANGED]** |
| 321_autocut_last_page | t1_black_rect.png | General › Media Name = Custom › Settings For Paper Handling After Print = Auto Cut(Only After Last Page) | DEVMODE diff: 19 B vs baseline, 1 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: AutoCut: 1->2 |
| 322_autocut_every_n1 | t1_black_rect.png | General › Media Name = Custom › Settings For Paper Handling After Print = Auto Cut(After Specified Number Of Pages) | DEVMODE diff: 19 B vs baseline, 1 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: AutoCut: 1->3 |
| 325_nocut_peel_off | t1_black_rect.png | General › Media Name = Custom › Settings For Paper Handling After Print = No Auto Cut(Feed To Peel Off Position) | DEVMODE diff: 19 B vs baseline, 1 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: AC(0:Off,1:On): 1->0; AutoCut: 1->0; TearOff: 0->1 |
| 326_nocut_cut_position | t1_black_rect.png | General › Media Name = Custom › Settings For Paper Handling After Print = No Auto Cut(Feed To Cut Position) | DEVMODE diff: 20 B vs baseline, 2 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: AC(0:Off,1:On): 1->0; AutoCut: 1->0; TearOff: 0->2 |
| 327_nocut_print_end | t1_black_rect.png | General › Media Name = Custom › Settings For Paper Handling After Print = No Auto Cut(Stop at the Print End Position) | DEVMODE diff: 20 B vs baseline, 2 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: AC(0:Off,1:On): 1->0; AutoCut: 1->0; TearOff: 0->3 |
| 323_autocut_every_n2_pages6 | t1_black_rect.png | Custom › After Print = Auto Cut(After Specified Number Of Pages) › Cut Interval = 2 (6-page doc; ref 322) | DEVMODE diff: 20 B vs baseline, 1 B vs 322_autocut_every_n1; printer data diff 0 B. Driver log vs 322_autocut_every_n1: ACNum: 1->2 |
| 324_autocut_every_n5_pages6 | t1_black_rect.png | Custom › After Print = Auto Cut(After Specified Number Of Pages) › Cut Interval = 5 (6-page doc; ref 322) | DEVMODE diff: 20 B vs baseline, 1 B vs 322_autocut_every_n1; printer data diff 0 B. Driver log vs 322_autocut_every_n1: ACNum: 1->5 |
| 328_autocut_default_pages2 | t1_black_rect.png | General › Media Name = Custom only, 2-page doc (reference for 321-327) | DEVMODE diff: 19 B vs baseline, 0 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: no change **[OK+LOG_UNCHANGED]** |
| 330_size_width_2.00 | t1_black_rect.png | Custom › Media Width = 2.00 in | DEVMODE diff: 36 B vs baseline, 18 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: PapSizeW: 1530->720; LblSizeW: 1587->776; SrcPapSizeW: 1531->720; SrcPapMgnL: 28->0; SrcPapMgnR: 28->0 |
| 331_size_length_3.00 | t1_black_rect.png | Custom › Media Length = 3.00 in | DEVMODE diff: 32 B vs baseline, 14 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: PapSizeL: 2477->1080; LblSizeL: 2477->1080; SrcPapSizeL: 2477->1080 |
| 332_size_50x30mm | t1_black_rect.png | Custom › Media Width = 1.97 in, Length = 1.18 in (50x30 mm user size) | DEVMODE diff: 49 B vs baseline, 32 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: PapSizeW: 1530->708; PapSizeL: 2477->425; LblSizeW: 1587->765; LblSizeL: 2477->425; SrcPapSizeW: 1531->709; SrcPapSizeL: 2477->425; SrcPapMgnL: 28->0; SrcPapMgnR: 28->0 |
| 333_size_102x152mm | t1_black_rect.png | Custom › Media Width = 4.02 in, Length = 5.98 in (102x152 mm user size) | DEVMODE diff: 45 B vs baseline, 28 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: PapSizeW: 1530->1447; PapSizeL: 2477->2152; LblSizeW: 1587->1503; LblSizeL: 2477->2152; SrcPapSizeW: 1531->1447; SrcPapSizeL: 2477->2153; SrcPapMgnL: 28->0; SrcPapMgnR: 28->0 |
| 334_size_min_1.00x0.31 | t1_black_rect.png | Custom › Media Width = 1.00 in, Length = 0.31 in (minimum) | DEVMODE diff: 49 B vs baseline, 33 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: PapSizeW: 1530->360; PapSizeL: 2477->113; LblSizeW: 1587->416; LblSizeL: 2477->113; SrcPapSizeW: 1531->360; SrcPapSizeL: 2477->113; SrcPapMgnL: 28->0; SrcPapMgnR: 28->0; AC(0:Off,1:On): 1->0; AutoCut: 1->0; TearOff: 0->1 |
| 335_size_length_12.00 | t1_black_rect.png | Custom › Media Length = 12.00 in | DEVMODE diff: 32 B vs baseline, 14 B vs 210_media_custom_t1; printer data diff 0 B. Driver log vs 210_media_custom_t1: PapSizeL: 2477->4320; LblSizeL: 2477->4320; SrcPapSizeL: 2477->4320 |
| 400_color_disable_correction | t3_full_test.png | Custom › Advanced... › Color Preset = Disable Color Correction | DEVMODE diff: 21 B vs baseline, 2 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: ColAdj1(0:Drv,4:No): 0->4; ColAdj2(14:Biz,1:Std,3:No): 14->3 |
| 401_color_mode_epson_standard | t3_full_test.png | Custom › Advanced... › Color Controls › Mode = EPSON Standard | DEVMODE diff: 20 B vs baseline, 1 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: ColAdj2(14:Biz,1:Std,3:No): 14->1 |
| 402_gamma_1.8 | t3_full_test.png | Custom › Advanced... › Color Controls › Gamma = 1.8 | DEVMODE diff: 20 B vs baseline, 1 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: Gamma: 220->180 |
| 403_inkprofile_on | t3_full_test.png | Custom › Advanced... › Ink Profile and Brightness Adjustment = on (sliders at default) | DEVMODE diff: 20 B vs baseline, 1 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: no change **[OK+LOG_UNCHANGED]** |
| 404_inkprofile_bkcmy_-3 | t3_full_test.png | Custom › Advanced... › Ink Profile and Brightness Adjustment = on › Ink Profile › BkCMY = -3 (pos 4) (ref 403) | DEVMODE diff: 21 B vs baseline, 1 B vs 403_inkprofile_on; printer data diff 0 B. Driver log vs 403_inkprofile_on: no change **[OK+LOG_UNCHANGED]** |
| 405_inkprofile_bkcmy_-6 | t3_full_test.png | Custom › Advanced... › Ink Profile and Brightness Adjustment = on › Ink Profile › BkCMY = -6 (pos 1) (ref 403) | DEVMODE diff: 21 B vs baseline, 1 B vs 403_inkprofile_on; printer data diff 0 B. Driver log vs 403_inkprofile_on: no change **[OK+LOG_UNCHANGED]** |
| 406_inkprofile_bk_-3 | t3_full_test.png | Custom › Advanced... › Ink Profile and Brightness Adjustment = on › Ink Profile › Bk = -3 (pos 4) (ref 403) | DEVMODE diff: 21 B vs baseline, 1 B vs 403_inkprofile_on; printer data diff 0 B. Driver log vs 403_inkprofile_on: no change **[OK+LOG_UNCHANGED]** |
| 407_inkprofile_bk_-6 | t3_full_test.png | Custom › Advanced... › Ink Profile and Brightness Adjustment = on › Ink Profile › Bk = -6 (pos 1) (ref 403) | DEVMODE diff: 21 B vs baseline, 1 B vs 403_inkprofile_on; printer data diff 0 B. Driver log vs 403_inkprofile_on: no change **[OK+LOG_UNCHANGED]** |
| 408_brightness_-2 | t3_full_test.png | Custom › Advanced... › Ink Profile and Brightness Adjustment = on › Brightness Adjustment › Brightness = -2 (ref 403) | DEVMODE diff: 23 B vs baseline, 3 B vs 403_inkprofile_on; printer data diff 0 B. Driver log vs 403_inkprofile_on: no change **[OK+LOG_UNCHANGED]** |
| 409_brightness_+2 | t3_full_test.png | Custom › Advanced... › Ink Profile and Brightness Adjustment = on › Brightness Adjustment › Brightness = +2 (ref 403) | DEVMODE diff: 22 B vs baseline, 2 B vs 403_inkprofile_on; printer data diff 0 B. Driver log vs 403_inkprofile_on: no change **[OK+LOG_UNCHANGED]** |
| 410_bidirectional_off | t3_full_test.png | Custom › Advanced... › Bidirectional Printing = off | DEVMODE diff: 20 B vs baseline, 1 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: BiDi: 1->0 |
| 411_banding_reduction_off | t3_full_test.png | Custom › Advanced... › Banding Reduction = off (Speed) | DEVMODE diff: 21 B vs baseline, 2 B vs 211_media_custom_t3; printer data diff 0 B. Driver log vs 211_media_custom_t3: BandingReduction: 1->0 |
| 500_hscopy_off_copies3 | t1_black_rect.png | Driver Utilities › Driver Preferences › High Speed Copies = off (+ app copies 3; ref 005) | DEVMODE diff: 11 B vs baseline, 11 B vs 005_default_t1_copies3; printer data diff 0 B. Driver log vs 005_default_t1_copies3: HSCopy: 1->0 |
| 501_hscopy_off | t1_black_rect.png | Driver Utilities › Driver Preferences › High Speed Copies = off | DEVMODE diff: 11 B vs baseline, 11 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: HSCopy: 1->0 |
| 502_always_spool_raw | t1_black_rect.png | Driver Utilities › Driver Preferences › Always spool RAW data type = on | DEVMODE diff: 11 B vs baseline, 11 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: RAWSpl: 0->1 |
| 503_print_as_bitmap_t3 | t3_full_test.png | Driver Utilities › Driver Preferences › Print as Bitmap = on | DEVMODE diff: 12 B vs baseline, 12 B vs 003_default_t3; printer data diff 0 B. Driver log vs 003_default_t3: ConvLine: 0->1 |
| 504_skip_blank_page_t4 | t4_blank.png | Driver Utilities › Driver Preferences › Skip Blank Page = on | DEVMODE diff: 12 B vs baseline, 12 B vs 004_default_t4_blank; printer data diff 0 B. Driver log vs 004_default_t4_blank: SaveBlank: 0->1 |
| 505_skip_blank_page_t1 | t1_black_rect.png | Driver Utilities › Driver Preferences › Skip Blank Page = on | DEVMODE diff: 12 B vs baseline, 12 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: SaveBlank: 0->1 |
| 506_join_barcodes_off | t1_black_rect.png | Driver Utilities › Driver Preferences › Join Adjacent Barcodes or 2D Symbol Fonts = off | DEVMODE diff: 12 B vs baseline, 12 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: BarJoin: 1->0 |
| 507_units_mm | t1_black_rect.png | Driver Utilities › Driver Preferences › Units to use = mm | DEVMODE diff: 12 B vs baseline, 12 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: Unit(0:I,1:M): 0->1 |
| 508_status_monitor_on | t1_black_rect.png | Driver Utilities › Driver Preferences › Use EPSON Status Monitor 3 = on | DEVMODE diff: 11 B vs baseline, 11 B vs 000_default_t1; printer data diff 2 B. Driver log vs 000_default_t1: ProgMet: 0->1 |
| 600_reduction_fit | t1_black_rect.png | General › Media Name = Media Size Reduction › (Fit to Page, before 4.25x6.88, output = 108x174.8 Die-cut) | DEVMODE diff: 23 B vs baseline, 23 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: PapSizeID: 2050->2000; Zoom(0:Off,3:Fit,9:Cust): 0->3; ZoomRatio: 100->96 |
| 601_reduction_custom_50pct | t1_black_rect.png | General › Media Name = Media Size Reduction › Custom, Scale to = 50 % (ref 600) | DEVMODE diff: 30 B vs baseline, 14 B vs 600_reduction_fit; printer data diff 0 B. Driver log vs 600_reduction_fit: Zoom(0:Off,3:Fit,9:Cust): 3->9; ZoomRatio: 96->50 |
| 602_reduction_fit_before_8.27x11.69 | t1_black_rect.png | General › Media Name = Media Size Reduction › Fit, Media Size Before Reduction = 8.27 x 11.69 in (ref 600) | DEVMODE diff: 36 B vs baseline, 19 B vs 600_reduction_fit; printer data diff 0 B. Driver log vs 600_reduction_fit: ZoomRatio: 96->49 |
| 610_vacuum_manual_default9 | t1_black_rect.png | Driver Preferences › Enable Platen Vacuum Manual Adjustment = on; Options › Platen Vacuum Manual Adjustment = on (slider left at 9) | DEVMODE diff: 14 B vs baseline, 14 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: EnableSuction: 0->1 |
| 611_vacuum_manual_4 | t1_black_rect.png | Same as 610 + Change Vacuum Setting slider = 4 (ref 610) | DEVMODE diff: 14 B vs baseline, 1 B vs 610_vacuum_manual_default9; printer data diff 0 B. Driver log vs 610_vacuum_manual_default9: Suction: 90->65 |
| 612_vacuum_manual_0 | t1_black_rect.png | Same as 610 + Change Vacuum Setting slider = 0 (ref 610) | DEVMODE diff: 14 B vs baseline, 1 B vs 610_vacuum_manual_default9; printer data diff 0 B. Driver log vs 610_vacuum_manual_default9: Suction: 90->30 |
| 202_print_preview_on | t1_black_rect.png | General › Print Preview = on (the job waits in the EPSON Print Preview program E_PPRECTJ.EXE; its menu command "Print Document" (WM_COMMAND 57607) was sent by hand to release it; screenshot sub\preview_EPSON_Print_Preview.png) | DEVMODE diff: 14 B vs baseline, 14 B vs 000_default_t1; printer data diff 0 B. Driver log vs 000_default_t1: Preview: 0->1 |
| 340_named_media_50x30mm | t1_black_rect.png | General › Media Definition... › New: Name = UD_50x30, Width = 1.97, Length = 1.18 (rest default) › OK; Media Name = UD_50x30 (ref 332 = same size via Custom) | DEVMODE diff: 48 B vs baseline, 6 B vs 332_size_50x30mm; printer data diff 0 B. Driver log vs 332_size_50x30mm: PapSizeID: 256->3000 |

### Step 4 observations
- All 78 matrix captures have status OK. In every one, the per-user DEVMODE changed, and the change was
  checked before printing. The printer data never changed. Before every capture the DEVMODE was confirmed
  byte-identical to the baseline.
- **`+LOG_UNCHANGED` rows are not failures.** The driver log has no field for these settings: Collate with
  1 copy (103), Quality (Mode2) (315/317/320; Mode1 only shows up because it turns BandingReduction off),
  and the Ink Profile switch, sliders and Brightness (403–409). For each, the DEVMODE diff confirms the
  change. 315/317/320 roughly double the .prn size. 404–409 differ from 403 in 49k–154k raster bytes.
  403 itself (Ink Profile on, sliders at 0) is identical to 211 apart from the timestamp and job ID. 328 is
  only a 2-page reference.
- **Units in the log:** sizes are in 1/360 in. Print Position Adjustment is in 0.1 mm (0.50 in → 127,
  -0.25 in → -64). Suction 9 → 90, 4 → 65, 0 → 30 (percent-like).
- **Custom with unchanged values (210/211) is not quite neutral.** It logs PapSizeID 2050→256, PapSizeW
  1531→1530 and LblGap 43→42 (rounding), yet the output matches 000/003 apart from the timestamp and job ID.
  A Media Definition entry (340) is logged as PapSizeID=3000.
- **334 (1.00 x 0.31 in):** setting a length under 0.59 in shows "Set a value of less than 15 mm / 0.59 inch
  as the media length. Cannot perform auto cutting because the paper may jam." The driver then switches
  After Print to No Auto Cut (Feed To Peel Off) by itself (log: AutoCut 1→0, TearOff 0→1). So 334 differs
  from 210 in size *and* cut mode.
- **Bidirectional off (410)** changes one raster byte only: ESC/P `ESC U n` at 0x10125 (`1B 55 00` bidi →
  `1B 55 01` uni).
- **Vacuum (611/612)** changes one byte in a settings block (0x100F7: 0x5A=90 → 0x41=65 / 0x1E=30). The
  byte 6 positions later (0x100FD) drops by the same amount (0x8C → 0x73 / 0x50), so it is probably a
  checksum.
- **Driver Preferences (500–508)** are stored in the per-user DEVMODE (11–12 bytes). For an image job none
  of them changes the printer data: the only extra differing bytes are the timestamp's day/hour, because
  the run crossed midnight. 504 (Skip Blank Page on the blank image) still writes a job, 1 byte shorter
  with 44 other bytes different (not yet analysed). 508 (Use EPSON Status Monitor 3) is logged as ProgMet 0→1.
- **Date/hour bytes:** captures made after midnight (2026-11-03) also differ in TI bytes 0x7D–0x7E
  (day, hour), so the TI timestamp is the full Y/M/D h:m:s.
- **Not captured:** Platen Vacuum without the Driver Preferences enable (the control is greyed). Printer
  Utilities (needs a printer). Barcode/Font Replacement settings (they only affect text in the printer's
  barcode fonts, not image jobs). Favourites and Import/Export (UI only).
- The driver-wide changes (500–508, 610–612) were undone after each capture. At the end, all Driver
  Preferences were back to default and the user/global DEVMODE and printer data matched the baselines.

## Step 6: driver logging
- **No registry switch exists.** `HKCU`/`HKLM\SOFTWARE\EPSON` (incl. `WOW6432Node`) and the queue keys have
  no ImageLog values. The switch is `C:\ProgramData\EPSON\EPSON TM-C3500 Ver2\EPLOGENV.INI`, section
  `[IMAGE_LOG_INFO]`: `ENABLE`, `MAX_FILE_COUNT`, `MAX_FILE_SUM_MB`. These strings sit together in
  `E_PMAICTA.DLL` with the folder name `ImageLog`. Nothing else was changed.
- The text job log (`EPSON TM-C3500 Ver2.log`, the `MedID=…,PapSizeID=…,AutoCut=…` lines) is a separate
  feature and was already on by default. It is the source of every `<id>.drvlog.txt`.

| id | image | setting changed | verification |
|---|---|---|---|
| 900_default_t1_logged | t1_black_rect.png | none in the driver; `EPLOGENV.INI` `ENABLE=0` → `1` (backup `driverlog\EPLOGENV.INI.original`) | DEVMODE = baseline. The driver wrote `ImageLog\TMC3500-CAPTURE_20261103003007712.prn` (copied to `driverlog\ImageLog\`), **byte-identical to 900.prn**, so the image log is a copy of the printer data stream, not a rendered image. 900 vs 000 differ only in TI/JH bytes 0x7D–0x80, 0x8F. `ENABLE` was set back to 0 afterwards (hash-checked against the backup). |

- `EPSON TM-C3500 Ver2.JBI` is a 40-slot job index: `n=<date time>,<spooler job id>,1,6,7CED8D69EBC7`.
  The 6 bytes `7C ED 8D 69 EB C7` are the constant that follows the job id in every `REMOTE1 JH` command
  (e.g. 000.prn at 0x93). It is probably a host/driver-instance id, so a Linux driver may need to generate
  or fix it.
- Copies in `driverlog\`: the job log after the baselines, after 900 and at the end (`*.final.log`), the
  JBI index, EPLOGENV.INI (original and enabled), gapsetting.ini and EPDEFDM.BIN.

## Nozzle Check (approved by you during step 6)
- Printer Utilities › Nozzle Check opens a "Nozzle Check" window that immediately reports "A communication
  error has occurred. Make sure that the printer is properly connected to the computer and try again."
  (`screenshots\sub\Nozzle_Check.png`). No print job, no file-port output and no image-log file was
  produced. The utility needs two-way communication with a real printer before it sends anything, so it
  can't be captured here. Manual Head Cleaning was not tried.
