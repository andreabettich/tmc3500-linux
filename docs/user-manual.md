# TM-C3500 label printing: user manual

For people who design and print labels on the Epson TM-C3500 from a Linux computer. You don't need to
know Linux administration. Whoever set up the printer (your administrator) has installed the driver
and created the printer queue, usually called **TM-C3500**.

This driver is unofficial: it is not made or supported by Epson. It produces the same printer output
as Epson's Windows driver, so settings you know from Windows work the same way here. For loading
media, replacing ink and clearing jams, follow Epson's printer manual.

## 1. The one rule: tell the driver what is loaded

The printer prints what the driver tells it to, for the media the driver *thinks* is loaded. Most
problems (wrong position, wasted labels, dull colours, labels not cut) come from settings that don't
match the roll in the printer. Before a print run, check these three settings:

| setting (print dialog) | choose | if it's wrong |
|---|---|---|
| **Media Size** | the size of one label (width × height, without the liner) | content is shifted, cut off or spread over two labels |
| **Media Form** | how the roll is made (see below) | the printer can't find the label start; labels are fed or cut in the wrong place |
| **Media Coating** | the label surface: Plain, Matte, Synthetic, Glossy or Wristband | colours and ink amount are wrong; ink may smear on glossy or synthetic labels |

**Media Form:**

| your roll | choose |
|---|---|
| separate labels on a liner, with a gap between them | Die-cut Label (Gap) |
| separate labels with a black mark on the back | Die-cut Label (Black Mark) |
| one continuous label strip (cut to length by the printer) | Continuous Label |
| continuous paper or tag stock without a liner | Continuous Paper |
| continuous paper with black marks on the back | Continuous Paper (Black Mark) |

**Sizes the printer accepts:**
- **Width:** 1.00 to 4.25 inches (25.4–108 mm) for labels, and 1.18 to 4.25 inches (30–108 mm) for
  paper.
- **Length:** 0.31 to 44 inches (8 mm–1.1 m).
- **Standard size:** "108 × 174.8 mm" is in the list. Any other size is a custom size, which most
  print dialogs let you type in (for example 50 × 30 mm).

Labels shorter than 15 mm can't be cut: the printer feeds them to the peel-off position instead.

Your administrator can set the queue's defaults to match the roll you normally use. Then you only
change settings when you load something different.

## 2. Designing labels

- **Make the document the size of the label.** One page = one label. Set the page size in your
  design program (Word, LibreOffice, Inkscape, label software) to exactly the label size, with no
  margins.
- **Keep important content about 1.5 mm from the edges.** The printer prints to the edge (Borderless
  is on by default), but labels can shift slightly on the roll.
- **Save or export as PDF** where possible. PDFs keep text and barcodes as sharp shapes, so they
  print at the printer's full resolution.

### Barcodes and small text

Barcodes must scan reliably, so:

1. **Use vector barcodes:** a barcode font, or the barcode tool of your label or design program,
   exported to PDF. Avoid pasting a barcode as a picture.
2. **If it has to be a picture,** make it at exactly **360 dpi** and don't scale it in the document.
   The printer prints 360 dots per inch, so a picture at another resolution is resized and bar
   widths change slightly.
3. **Choose the black for barcodes.** The **Black** setting has two choices:
   - **Rich black (as Windows):** black is built from all four inks, exactly as on Windows. It's the
     deepest black and best for logos and photos.
   - **Black ink only for black and greys:** black and grey are printed with black ink only, so
     there's no colour fringe around bars and they have the crispest edges. Use it for barcode and
     text labels. Other colours on the label are unchanged.
4. **Keep the quiet zones** (the empty margins left and right of a barcode). Don't put text or
   frames in them.
5. **Use the barcode standard's minimum bar width or larger.** If a scanner struggles, make the bars
   a little wider before changing other settings.

### Colours

- Colours print the same as from Epson's Windows driver with the same settings. Labels designed and
  approved on Windows will match.
- Use **RGB colours** (sRGB) in your documents. Brand colours given as hex codes (`#E30613`) or RGB
  values can be entered directly.
- The colour also depends on the **Media Coating** setting. The same colour looks different on Matte
  and Glossy labels, and the driver adjusts the ink for each.
- Before a large run of a new design, print one label and compare it with a reference label.

## 3. Printing

### From a program

Choose **File → Print**, select the **TM-C3500** printer, then open the printer's options. The tab
is named "Printer Options", "Properties" or "Advanced" depending on the program. All settings
described here are there, with the same names as in the Windows driver.

### From a command line or script

If labels are printed by a script, or you're asked to print from a terminal:

```sh
lp -d TM-C3500 label.pdf                                   # with the default settings
lp -d TM-C3500 -n 10 label.pdf                             # 10 copies
lp -d TM-C3500 -o PageSize=Custom.50x30mm label.pdf        # a 50 × 30 mm label
lp -d TM-C3500 -o TmCoating=Glossy -o TmQuality=Quality1 label.pdf
lp -d TM-C3500 -o TmBlack=KOnly barcode-label.pdf          # black ink only for barcodes
```

The full list of `-o` settings is in [operations.md §2](operations.md#2-printing).

### Print quality

| Print Quality | use for |
|---|---|
| **Speed** (default) | most labels: text, barcodes, logos. Fastest. |
| **Quality Mode 1** | photos, gradients, large colour areas. Slower, smoother. Not on Plain labels. |
| **Quality Mode 2** | the smoothest result for demanding colour work. Slowest. Not on Plain or Wristband. |

The print dialog doesn't offer combinations the printer can't do. A script that asks for one gets an
error (see §5).

**Banding Reduction** (Speed only, on by default) hides fine horizontal stripes in colour areas.
**Bidirectional Printing** (on by default) is faster; turn it off only if vertical lines look
slightly jagged.

### Cutting

| After Printing | what happens |
|---|---|
| Auto Cut every page (default) | each label is cut off |
| Auto Cut every N pages | cut after every N labels (set **Cut Every N Pages**) |
| Auto Cut after last page | the whole job stays in one strip, cut at the end |
| No Cut, feed to peel-off position | for peeling labels off the liner by hand |
| No Cut, feed to cut position | labels stay attached; you tear or cut them yourself |
| No Cut, stop at print end | the printer stops right after printing |

### Other settings

| setting | what it does |
|---|---|
| Copies, Collate | several copies; Collate prints 1-2-3, 1-2-3 instead of 1-1, 2-2, 3-3 |
| Rotate 180 Degrees | prints upside down, e.g. when the roll unwinds the other way |
| Print Position Vertical / Horizontal | moves the print by the given amount (e.g. 0.5 mm) to correct a small offset |
| Skip Blank Pages | blank pages in the document produce no label |
| Media Saving (continuous forms) | shortens each label to the printed content, saving media (Continuous Label and Continuous Paper only) |
| Borderless | on by default; turn off to leave a white margin of about 1.5 mm at the top and bottom |
| Buzzer | the printer beeps after each label or at the end of the job |
| Platen Vacuum | how strongly the printer holds the media flat. Automatic is right for most rolls; use a stronger setting for curling or thick media |
| Colour Correction, Gamma, Brightness, Ink Adjustment | the same colour adjustments as on Windows. Lower the ink amount if ink smears or bleeds on a coating |

## 4. Checking the printer

- **Is it printing, waiting or in error?** Open the CUPS printer page (your administrator can give you
  the address, usually `http://localhost:631/printers/TM-C3500`) or your desktop's printer settings.
  The printer status shows messages such as "Media empty" or "Cover open".
- **Ink levels and the maintenance box** are shown on the same page. From a terminal,
  `tmc3500ctl ink` lists them, with LOW, EMPTY or NEAR FULL flags.
- Your administrator also receives automatic alerts (mail or chat) when ink runs low, paper runs out
  or a job is held.

Nozzle check and head cleaning aren't available from this driver yet. If printouts show missing
lines or stripes, use the printer's own controls as described in Epson's manual, or ask your
administrator.

## 5. When something goes wrong

| what you see | what to do |
|---|---|
| **Job waits; printer shows "Media empty", "Cover open" or similar** | Fix the problem at the printer (load media, close the cover). The job continues by itself. Don't print it again: you'd get it twice. |
| **Paper or ink ran out in the middle of a job** | Load media or replace the cartridge. The printer continues where it stopped. |
| **Job is "held"** | Follow "Held jobs" below. |
| **Job fails immediately** | A setting combination isn't allowed (for example Plain coating with Quality Mode 2). The job log names the setting. Change it and print again. The queue may also stop ("Paused" / "Stopped") so that later jobs don't fail too: ask your administrator to re-enable it (`cupsenable TM-C3500`). |
| **Content shifted or spread over two labels** | Check Media Size and Media Form against the loaded roll (§1). For a small constant offset, use Print Position. |
| **Labels not cut** | Check After Printing. Labels shorter than 15 mm are never cut. |
| **Colours dull, too dark, or smeared** | Check Media Coating. For smearing on glossy or synthetic labels, try Ink Adjustment with a lower amount. |
| **Barcode doesn't scan** | Use a vector barcode or a 360 dpi picture, Black "ink only", and wider bars (§2). |
| **Nothing happens at all** | Check that the printer is on and connected to the network. Then tell your administrator. |

### Held jobs: check before you release

If the network connection to the printer breaks while a job is printing, the job is **held**. The
driver never prints it again by itself, because it can't know how many labels already came out. This
protects you from duplicate labels, which matters for serial numbers, tickets and shipping labels.

1. **Look at the printer output.** Count which labels of the job printed correctly.
2. **Decide:**
   - All labels printed: cancel the held job.
   - None printed: release it.
   - Some printed: release it and discard the duplicates, or cancel it and print only the missing
     labels.
3. **Release or cancel.** Use the CUPS jobs page, ask your administrator, or run
   `tmc3500ctl release-held-jobs` to list held jobs and `tmc3500ctl release-held-jobs --job <number>`
   to release one. A released job prints again **from the beginning**.

## 6. Quick checklist for a print run

1. Roll loaded, and Media Size, Form and Coating match it.
2. Document page size = label size, saved as PDF.
3. Barcodes: vector or 360 dpi, Black "ink only".
4. Print one label, check it (scan the barcode, compare colours), then print the run.
5. If a job is held: check the output before releasing it.
