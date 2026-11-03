# Developer guide

How the code is organised, how a label travels through it, and how to make the usual changes safely.
Start with `AGENTS.md` (rules, commands, traps). See `docs/api.md` for the external interfaces.

## 1. Data flow

```
application ─PDF/image─▶ CUPS ─▶ cups-filters (Ghostscript/Poppler) ─▶ CUPS raster, RGB 8-bit
                                                                         │ (360×360 or 360×720 dpi)
rastertotmc3500  (filter, stdin → stdout)                                ▼
  options::JobConfig::from_cups   PPD defaults + job options → validated settings
  color::Converter                RGB → K/C/M/Y ink amounts (colour table) → drop-size split
  dither::dither_row              ink amounts → 2-bit drops (none/small/medium/large)
  layout                          rotate 180, position adjustment, media saving, printable area
  weave::weave                    page → head passes (175 nozzles, 2 rows per ink)
  protocol::job::JobWriter        EJL/REMOTE1 header, page setup, RLE raster, trailer
                                                                         │ printer job bytes
tmc3500 backend  (CUPS backend, runs as lp)                              ▼
  SNMP pre-check → wait while in error → TCP 9100 send → drain → exit code to CUPS
  on mid-job loss/stall: hold the job, write a partial-send marker, alert
                                                                         ▼
                                                                    TM-C3500
tmc3500-monitor  (systemd) ── SNMP poll every 30 s ──▶ alert hooks, Prometheus textfile
tmc3500ctl       (admin)   ── SNMP / lpadmin / lp  ──▶ status, ink, counters, setup, release held jobs
```

`pipeline::render_page` connects the filter steps for one page. The filter
(`crates/rastertotmc3500/src/lib.rs`) handles copies, collation, skip-blank and the job log.

## 2. Crates and modules

| crate | what it is | key files |
|---|---|---|
| `tmc3500-core` | Pure library, no I/O. All printer knowledge lives here. | see below |
| `rastertotmc3500` | CUPS filter binary. Reads the PPD defaults, picks the colour table, writes the job and logs `PAGE:`/`INFO:`/`ERROR:` lines. | `src/lib.rs` |
| `tmc3500-backend` | CUPS backend binary, installed as `tmc3500`. Implements the spec's error policy (section 5). | `src/lib.rs` (state machine), `reasons.rs` (CUPS `STATE:` lines), `alerts.rs` |
| `tmc3500-common` | Code shared by the backend, monitor and ctl, with I/O. | `config.rs` (TOML), `snmp.rs` (SNMP v1/v2c client), `alert.rs` (hooks, repeat suppression), `markers.rs` (partial-send markers), `uri.rs` |
| `tmc3500-monitor` | systemd service: idle polling, alert transitions, metrics. | `src/lib.rs` |
| `tmc3500ctl` | Admin CLI, every command supports `--json`. | `src/main.rs` |
| `tmc3500-fakeprinter` | Simulated printer (TCP 9100 + SNMP agent, scripted faults) for tests and end-to-end runs. | `src/lib.rs` |
| `xtask` | Developer tools: fits colour tables from Windows chart captures. Not shipped. | `src/lib.rs`, `tests/fit.rs` |
| `fuzz/` | `cargo fuzz` targets: `cupsraster`, `options`, `snmp`, `decode`, `colour_table`. | |

`tmc3500-core` modules:

| module | responsibility |
|---|---|
| `options` | `JobConfig`, and the Windows dialog's rules (allowed coating/quality pairs, cut modes, minimum sizes). The only place option names are parsed. |
| `cupsopts` | Tokenises the CUPS option string; parses lengths (`0.5mm`, `-0.02in`). |
| `cupsraster` | CUPS raster reader (`RaSt`/`RaS2`/`RaS3`, both byte orders) with size limits. |
| `units`, `geometry` | 0.1 mm ↔ 1/360 in, with the Windows rounding; the values for `ESC(C`, `ESC(S`, `ESC(l`, `ESC(c` and SN. |
| `layout` | Printable area, position adjustment, rotate 180, media saving. |
| `color` | Colour tables (`Lut`, TMCLUT2 format), tetrahedral interpolation, drop-size split curves, brightness tone curve, ink profile, K-only black (`Converter`). |
| `dither` | Ordered threshold dither from ink amounts to 2-bit drops. Solids get no holes and zero ink gets no dots, so barcode edges stay sharp. |
| `raster` | `InkPage`: four planes of 2-bit drops at device resolution. |
| `weave` | Pass planning (Speed, Quality1, Quality2, banding reduction, borderless off) and blank-area trimming. |
| `rle` | Epson RLE (byte-exact with every captured line). |
| `protocol::{job, sn, writer}` | Byte encoding of the job: REMOTE1 commands, the SN media block with checksum, page setup, raster bands, trailer. Clock and job ID are injected. |
| `decode`, `reassemble` | Strict job parser and the inverse of the weave (job → dithered pages). Used by tests, the fake printer and the colour fitter. |
| `status::{snmp, printer, epson}` | SNMP values → `PrinterStatus` → CUPS reasons, alert conditions and events. `epson` is a stub until the reply format is recorded (phase 5). |
| `pipeline` | One page from raster to `protocol::job::Page`. |

## 3. Tests and evidence

| test | proves |
|---|---|
| `tmc3500-core/tests/golden.rs` | For each of the 105 Windows captures in `testdata/captures/`, the options in `testdata/golden.toml` produce the same command skeleton byte for byte (timestamp and job ID masked). RLE also reproduces every captured raster line. |
| `tmc3500-core/tests/weave_structure.rs` | Pass structure (feeds, block placement, nozzle rows) matches the captures, including borderless off in Quality modes. |
| `rastertotmc3500/tests/golden_raster.rs` | The whole filter, run on the source PNGs, matches the captures' command and raster-block structure. |
| `rastertotmc3500/tests/ppd.rs` | The PPD is well formed, offers every option the core accepts, has the Windows defaults, and constrains the combinations the core rejects. |
| `xtask/tests/fit.rs` | Colour fitting round trips. The shipped Matte Speed table prints the chart with the same ink as Windows (median < 1 %, p90 < 2 %). |
| `tmc3500-backend/tests/backend.rs` | Error policy against the fake printer: hold on a lost connection, no duplicates, waiting on paper out, cancel, unreachable before start. |
| `tmc3500-monitor/tests/`, `tmc3500ctl/tests/`, `tmc3500-common/tests/` | Alert transitions, metrics, CLI JSON, config validation, hook security, packaging paths and modes. |

How the captures were made: the Windows driver ran in a VM and printed to a file port. Each capture
has the driver's settings log (`*.drvlog.txt`) and a row in `manifest.csv` (source image, copies,
pages, SHA-256). The source images are in `testdata/images/`, and the colour chart is in
`testdata/colour/`.

**End-to-end in a container:** run cupsd and start the fake printer:

```sh
cargo run -p tmc3500-fakeprinter -- --snmp-port 1161 --store-dir /tmp/jobs
```

Install the `.deb` (or copy the binaries to `/usr/lib/cups/{filter,backend}`), then create the queue
with the device URI `tmc3500://127.0.0.1:9100/?snmp-port=1161` and print with `lp`. Faults (paper
out, dropped connection, ink levels) can be set by typing commands into the fake printer's stdin;
`help` lists them.

## 4. Recipes

### Add or change a print option

1. Get a Windows capture of the setting if it changes any command (see "Add a Windows capture").
   Options that only change the raster content can be checked by the filter tests.
2. Add a field to `options::JobConfig` and parse it in `JobConfig::from_cups`. Add the Windows
   dialog's constraints there too (rejected combinations become `OptError`, which the filter reports
   as an `ERROR:` line). Boolean options also go into `BOOL_KEYS`.
3. Apply it where it belongs: `protocol::job` (commands), `geometry`/`layout` (placement), `weave`
   (passes), or `color` (raster content).
4. Add it to `packaging/ppd/tmc3500.ppd` (`*OpenUI Tm...`, with the Windows default and
   `*UIConstraints` for rejected combinations). `ppd.rs` fails until the PPD and the core agree.
5. Document it in `docs/operations.md` §2 and `docs/api.md` §1.
6. Existing installations get the new PPD only when the admin re-runs `tmc3500ctl setup ... --apply`.
   Mention that in the release notes.

### Change a printer command

Change `protocol::job` (or `sn`, `geometry`). Run `cargo test -p tmc3500-core --test golden`. The
failure message names the capture and shows the first differing bytes. Every capture must still
match. If one doesn't, the change is wrong for that setting or needs a new rule; never edit the
captures.

### Add a Windows capture as a golden test

1. Copy `NNN_name.prn` and `NNN_name.drvlog.txt` to `testdata/captures/` and add the row to
   `manifest.csv`. The source image must be in `testdata/images/`.
2. Add `"NNN_name" = { options = "..." }` to `testdata/golden.toml`, using the CUPS options that
   correspond to the driver log. `every_capture_has_a_golden_entry` fails until you do.
3. Explain any new finding in `testdata/captures/NOTES*.md` and in the spec §3.

### Add a printer status reason or alert

`status::printer` maps SNMP values to `Condition`s. Each condition has a CUPS reason keyword and an
`EventKind`. The backend (`reasons.rs`) and the monitor pick it up from there, and so do the CLI
JSON, the metrics and the alert hooks. Add a parser fixture test in `status::snmp`/`printer` first.
For the Epson back-channel reply, fill in `status::epson::parse_reply`; `PrinterStatus::merge_epson`
already connects it.

### Rebuild the colour tables

From a new Windows colour session (capture kit in `../tmc3500_colour_kit/`):

```sh
tools/build_colour_tables.sh <captures-dir>    # fits measured tables, derives the rest, needs all 48
cargo test -p xtask                            # accuracy tests against the Windows chart
```

The script maps captures to tables (`700` plain-speed ... `722` matte-speed-off). It refuses to
finish unless exactly 48 tables were produced. A new chart must be generated with
`tools/make_colour_chart.py` and its layout JSON committed beside it. Fitting works in two stages:
printed chart colours are fitted exactly on a 9-level grid, and unprinted ones are solved by smooth
least squares. The table is then refined on a 17-level grid. The drop-size split is a separate 1-D
curve per ink. The code is in `crates/xtask/src/lib.rs`.

### Change the backend's error handling

Read spec §5 and `docs/operations.md` §4 first: these behaviours are promised to the operator.
Add a fake-printer scenario in `crates/tmc3500-backend/tests/backend.rs` that reproduces the case,
then change `src/lib.rs`. Every socket call needs a timeout. Every path where bytes may have reached
the printer must end in `CUPS_BACKEND_HOLD` plus a marker, never a retry.

## 5. Packaging

`packaging/build-debs.sh` builds static musl binaries and one `.deb` per architecture, using
`cargo deb -p tmc3500ctl`. The asset list is in `crates/tmc3500ctl/Cargo.toml`. The install paths,
owners and modes, and why each is needed, are in `packaging/README.md`. They are tested by
`tmc3500-common/tests/packaging.rs`. Maintainer scripts are in `packaging/debian/`.
