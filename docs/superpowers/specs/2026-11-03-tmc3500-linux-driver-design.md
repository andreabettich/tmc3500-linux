# TM-C3500 Linux driver: design

Date: 2026-11-03. Status: draft for review.

## 1. Goal and success criteria

The Epson TM-C3500 (ColorWorks label printer, SJIC22P K/C/M/Y inks) is to be driven from Linux in a
mission-critical installation, with no Windows machine, and with every function the Windows driver
v2.6.0.2 offers.

The driver is done when:
1. Every option in the Windows driver's settings (see `tmc3500_captures/SETTINGS_INVENTORY.md`) can be
   used from CUPS, and produces the same printer commands as Windows. This is checked byte-for-byte
   against the captures, except for the timestamp, job ID and raster bytes.
2. Barcodes and text print sharp and scan reliably: Code 128, DataMatrix and QR at their minimum module
   sizes.
3. Brand colours closely match the Windows output: per-patch ink coverage within a set tolerance, and a
   visual or spectrophotometer check on the real printer.
4. Printer errors never lose a job and never print duplicates silently. Every error and warning shows in
   CUPS and triggers alerts. Status and maintenance work from the command line, with JSON output.
5. It runs on Debian/Ubuntu amd64 and arm64 (Raspberry Pi class), installed from a `.deb`.

Requirements as stated by the user:
- Linux only, rock solid, all Windows functions.
- Printer on the LAN.
- Content is barcodes and text plus brand colours that must match.
- Hosts are Debian/Ubuntu x86_64 and ARM64.
- Status: CUPS status, alerts, and a CLI/JSON tool.
- Implementation language: Rust.
- On a mid-job connection loss: hold the job and alert.

Assumptions:
- Apps print PDF or images through CUPS.
- The printer has a fixed IP address or a DHCP reservation.

## 2. Architecture

```
apps ──PDF──▶ CUPS ──(cups-filters: Ghostscript/Poppler)──▶ CUPS raster (RGB, 8 bit, printer dpi)
                                                                 │
   rastertotmc3500 (filter) ── tmc3500-core ──▶ ESC/P2 job bytes ┘
                                                                 │
   tmc3500 backend (tmc3500://host[:9100]) ── TCP 9100 + status ─▶ printer
   tmc3500-monitor (systemd) ── idle status polling ─▶ alert hooks / metrics
   tmc3500ctl ── status / ink / counters / nozzle-check / clean / setup / release-held-jobs
```

Cargo workspace `tmc3500-linux/`:

| crate | kind | role |
|---|---|---|
| `tmc3500-core` | lib, no I/O | options, job model, protocol encoder, colour, dither, weave, status parser |
| `rastertotmc3500` | bin | CUPS filter: CUPS raster from stdin → job bytes to stdout |
| `tmc3500-backend` | bin (`tmc3500`) | CUPS backend: send the job, watch status, pause/hold/retry, alerts |
| `tmc3500-monitor` | bin | idle status poller (systemd service), alerts, Prometheus textfile |
| `tmc3500ctl` | bin | admin CLI with `--json` output |
| `tmc3500-fakeprinter` | bin (test only) | simulated printer for integration tests |
| `xtask` | bin | golden-test generation, colour-table fitting, packaging |

Modules of `tmc3500-core`. Each is pure and testable on its own:
- `options`: turns CUPS option strings into a validated `JobConfig`, with the Windows dialog's rules:
  - coating decides which qualities are allowed;
  - quality decides banding reduction;
  - a length under 15 mm forces "No Auto Cut (Feed To Peel Off)";
  - continuous forms have no gap;
  - a minimum width per form.
- `protocol`: writes the job.
  - **Header:** EJL/1284.4 entry, `ESC @`, REMOTE1 blocks.
  - **REMOTE1 commands:** EX, SV, TI, JS, JH, PM, SN, BC, BB, LD, JE.
  - **Page setup:** `ESC(G`, `ESC(U`, `ESC U`, `ESC(i`, `ESC(C`, `ESC(S`, `ESC(l`, `ESC(c`, `ESC(D`, `ESC(m`,
    `ESC(e`.
  - **Raster:** `ESC($`, `ESC(v`, `ESC i` with RLE compression.
  - **Trailer.**

  The clock and job ID are injected, which makes the output deterministic.
- `weave`: maps the dithered page onto head passes:
  - 175 lines per block, two nozzle rows per ink (colour codes `c` and `c|0x40`);
  - the paper advance per pass, quality-mode passes, and trimming blank areas.
- `dither`: turns 16-bit ink amounts into 2-bit dots (4 levels):
  - a stable threshold pattern for flat and solid areas, error diffusion for photos and gradients;
  - edges are kept intact, so bar widths don't change.
- `color`: converts RGB to ink amounts (K, C, M, Y, plus dot-size split) with 3-D lookup tables:
  - one table per coating × quality × colour mode;
  - gamma, ink-profile and brightness adjustments;
  - "Disable Color Correction" and an optional K-only black.
- `status`: parses SNMP values and Epson status replies into a typed `PrinterStatus`. Strict length and
  range checks; it never panics.

## 3. Protocol (known so far, from the captures)

Byte layout of a job, as produced by Windows (default settings):

1. **Block 1 (pre-job):**
   - `00 00 00`, then `ESC 01 "@EJL 1284.4\n@EJL     \n"`, then `ESC @ ESC @`;
   - `ESC(R 08 00 00 "REMOTE1"`, then `EX 06 00 00 00 00 00 24 02`, `SV 00 00`,
     `EX 07 00 00 00 00 00 27 02 02`;
   - `ESC 00 00 00`.
2. **Block 2 (job start):**
   - EJL entry again;
   - REMOTE1 with:
     - `TI 08 00 00 YYYY(be) MM DD hh mm ss`;
     - `JS 04 00 00 00 00 00`;
     - `JH 0c 00 00 01 <jobid> 00 00 00 <6-byte id 7c ed 8d 69 eb c7>`.
3. **Padding:** two `ESC(d ff 7f` blocks (2 × 32767 zero bytes).
4. **Page setup (per page):**
   - EJL entry, then REMOTE1 with:
     - `PM 02 00 00 00`;
     - `EX 06 00 00 00 00 00 2d 01`;
     - `SN`, the media block (§3.1);
     - `BC 04 00 00 00 <n> 00` (cut every n pages);
     - `BC 04 00 00 01 00 00`;
     - `BB` (beep, optional);
   - then:
     - `ESC(G 01` (graphics mode);
     - `ESC(U 04 04 <v> a0 05`: units 1440/4, with v = 4 (360 dpi) or 2 (720 dpi vertical, Quality modes);
     - `ESC U <0 bidirectional | 1 unidirectional>`;
     - `ESC(i <0 Speed | 1 Quality>`;
     - `ESC(C <page length>`;
     - `ESC(S <width> <length>`;
     - `ESC(l <left>`;
     - `ESC(c <top> <bottom>`;
     - `ESC(D 40 38 50 28`;
     - `ESC(m <mode>`:

       | value | meaning |
       |---|---|
       | 0x23 | Speed, banding reduction on |
       | 0x22 | Speed, banding reduction off, or Plain coating |
       | 0x21 | borderless off |
       | 0x43 | Quality Mode1 |
       | 0x47 | Quality Mode2 |

     - `ESC(e 00 <0x11 Speed | 0x14 Quality>`.
5. **Raster (per pass):**
   - `ESC($ <x>`, then `ESC i <c> 01 02 <bytes/line> <175 lines>` + RLE data + `CR`, for colours in the
     order C(2), C'(66), M(1), M'(65), Y(4), Y'(68), K(0), K'(64);
   - `ESC(v <advance>` between passes (330 in Speed; Quality uses interleaved passes, e.g. 10 / 320).
6. **Page end:** `FF`.
7. **Trailer:** `ESC @`, REMOTE1 `LD`, `ESC @`, REMOTE1 `LD`, `JE 01 00 00`.

### 3.1 SN media block

The `SN` payload is 26 bytes (REMOTE1 length field `1a 00`):

| bytes | field |
|---|---|
| 0–1 | `00 40` |
| 2–3 | `ff ff` |
| 4–5 | width in 0.1 mm, rounded up from the `ESC(S` width (e.g. 1587 dots → 1120 = 112.0 mm) |
| 6–7 | label length in 0.1 mm, rounded up from the length in dots (`ffff` for continuous forms) |
| 8 | coating: 1 Plain, 3 Matte, 4 Synthetic, 5 Wristband, 6 Glossy (2 is used by continuous paper) |
| 9 | `00` |
| 10–17 | `ff…` |
| 18 | vacuum (0x5A = 90, 0x41 = 65, 0x1E = 30; `ff` = automatic) |
| 19–23 | `ff…` |
| 24–25 | checksum: the 16-bit little-endian sum of bytes 2–23 |

Raster compression: Epson RLE, applied per line (runs never cross a line). Runs of 3–128 identical bytes
use control `257 − n`, then the byte. Everything else goes out as literals of 1–127 bytes, with control
`n − 1`. This reproduces all 283,202 lines in the captures exactly.

Geometry: sizes are first rounded to 0.1 mm (half up), then converted to 1/360 in (half up). The minimum
length is 8.0 mm (0.31 in in the UI). Label forms add 2 × 28 dots of edge to the width. `ESC(l` is 56 when
the width exceeds 1531 dots, otherwise 28 (an assumption; only those two cases were captured). The
die-cut gap is 43 dots for the predefined 108 × 174.8 mm media and 42 for custom sizes.

### 3.2 Setting → command map

Verified from the captures. All lengths are in 1/360 in unless stated otherwise.

| setting | effect on the job |
|---|---|
| Media form | `EX 24 nn`: 02 die-cut gap, 03 die-cut black mark, 00 continuous label or paper, 01 continuous paper black mark. Also SN length and page geometry. |
| Media saving | `ESC(C` / `ESC(S` / `ESC(c` length shrinks to the printed area |
| After print | Auto cut: `BC n` (every n pages), `BC` dropped for "last page". No cut: `EX 27 02 01` + `EX 25 0x` (02 peel-off, 01 cut position, 00 print end). |
| Notification | `BB 00 01 00 00 00` job end, `BB 00 00 00 01 00` page end |
| Borderless off | `ESC(c` bottom −21, `ESC(m 21` |
| Bidirectional | `ESC U` |
| Banding reduction | `ESC(m` bit 0 |
| Vacuum | SN byte 18 |
| Position adjustment, orientation, rotate 180, size reduction, copies, collate, reverse | Raster placement or page order only, no command changes. High-speed copies sends the page once per copy. |
| Colour mode, gamma, ink profile, brightness | Raster content only (colour tables, §2) |
| Driver preferences, log settings, favourites, import/export | No effect on output. Not reproduced, except Skip Blank Page. |

Still to be confirmed:
- **The 6-byte `JH` id:** probably a host or driver instance id. Generate a fixed id per installation;
  check on the real printer.
- **504 skip-blank:** 44 differing bytes, not yet analysed.
- **Nozzle row split and pass layout:** to be confirmed against every capture in phase 2.

## 4. Colour and barcode quality

- **Colour-chart capture:** one more Windows session on the stopped Azure VM.
  - About a 9×9×9 RGB cube, grey ramps, pure-ink ramps, and the user's brand colours.
  - For each coating × quality combination (13) and each colour mode (Preferred, Standard, gamma 1.8,
    Disable).
  - From each patch, measure the per-ink coverage per dot size. Fit 3-D tables with tetrahedral
    interpolation.
  - The fitting tool lives in `xtask`. The tables are our own derived data and contain no Epson files.
- **Dithering:** our own dither aims for the same per-ink, per-dot-size coverage.
- **Barcodes:**
  - solid areas get full coverage with no dither noise, and edges stay sharp;
  - default black is the Windows rich black, with an option for K-only black;
  - the input resolution equals the printer resolution (360×360 Speed, 360×720 Quality), so nothing is
    rescaled.

## 5. Status, errors, alerts

- **Status sources:**
  - SNMP: Printer MIB and Host Resources MIB, with supply levels.
  - The Epson status reply over the job connection. Its exact format will be recorded in the real-printer
    session and turned into parser test fixtures.
- **Backend behaviour:**
  - **Before sending:** wait while the printer is in error. Report CUPS `printer-state-reasons` (e.g.
    `media-empty-error`, `cover-open`, `marker-supply-empty-error`) with clear messages.
  - **Paper or ink runs out mid-job:** keep the connection open and report the reason; the printer resumes
    on its own.
  - **Connection lost mid-job:** **hold the job** (CUPS backend hold) and alert. It is never reprinted
    automatically. `tmc3500ctl release-held-jobs` releases it.
  - **Printer unreachable before start:** retry with backoff. After a configurable time limit, hold and
    alert.
  - Every socket operation has a timeout, and reply sizes are capped.
- **`tmc3500-monitor`:** polls every 30 s (configurable) while idle, so it detects "ink empty" or "offline"
  before the next job. It writes a Prometheus textfile.
- **Alerts:**
  - Each event is passed as JSON on stdin to every executable in `/etc/tmc3500/alert.d/`.
  - Events: state change, ink low or empty per colour, maintenance box near full or full, media out, jam,
    cover open, cutter error, job held or failed, unreachable, recovered.
  - Repeats are suppressed per event, with a configurable repeat interval.
  - Example hooks for mail, webhook and syslog.
- **`tmc3500ctl`:** `status`, `ink`, `counters`, `nozzle-check`, `clean`, `setup --printer <ip>`,
  `release-held-jobs`, all with `--json`. Maintenance commands stay disabled until verified on the real
  printer.

## 6. Security

- No `unsafe` code in `tmc3500-core` or the parsers (`#![forbid(unsafe_code)]`).
- Explicit size limits everywhere, and no panics on any input (checked by fuzzing).
- Minimal dependencies. CI runs `cargo audit` and `cargo deny` (licences, advisories).
- The filter and backend run as `lp`. The monitor runs as a dedicated system user with systemd hardening:
  - `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`;
  - network access restricted to the printer's address (`IPAddressAllow`).
- AppArmor profile for the filter and backend.
- The install guide covers CUPS hardening:
  - accept jobs only from localhost or listed hosts;
  - protect or disable the web interface.
- No Epson binaries or data files are shipped.

## 7. Testing

1. Unit tests per core module.
2. Golden tests: for every capture in `tmc3500_captures/`, rebuild the job from its logged settings and
   source image.
   - Commands, media block, checksums, geometry, pass structure and compression must match exactly.
   - Timestamp and job ID are masked.
   - Raster content is compared by per-ink coverage within a tolerance.
3. Colour tests against the chart captures, within tolerance.
4. Barcode test sheet: check by software that bar widths are exact and no stray dots appear.
5. `cargo fuzz` targets: CUPS raster reader, option parser, SNMP and Epson status parsers. A short run on
   every CI build, plus a long campaign before release.
6. `tmc3500-fakeprinter` for integration tests. It simulates:
   - accepting and checking jobs, status replies;
   - paper out, jam, cover open, a dropped connection mid-job, a slow network, garbage replies.

   Checked properties: no lost job, no duplicate, no hang, correct states and alerts.
7. End-to-end: CUPS in a container, a PDF in, the job arriving at the fake printer, on amd64 and arm64
   (QEMU).
8. Soak test: thousands of labels against the fake printer, with random faults injected.
9. Real-printer acceptance: side-by-side colour and barcode labels against Windows output, and every error
   case triggered by hand.

## 8. Packaging

- A `tmc3500_<ver>_{amd64,arm64}.deb` built with `cargo-deb`, containing:
  - `/usr/lib/cups/filter/rastertotmc3500`;
  - `/usr/lib/cups/backend/tmc3500` (mode 0700, root, as CUPS requires for backends that run as root);
  - the PPD in `/usr/share/ppd/tmc3500/`;
  - `tmc3500-monitor.service`;
  - `/etc/tmc3500/{tmc3500.toml,alert.d/}`;
  - the AppArmor profile;
  - `tmc3500ctl`.
- Setup command: `tmc3500ctl setup --printer <ip>`.

## 9. Phases

1. **Protocol core:** options, `JobConfig`, all commands; golden tests on the existing captures. No
   hardware needed.
2. **Weave, RLE and basic dither, filter, PPD:** a complete job from CUPS to the fake printer.
3. **Colour:** the chart capture session (new prompt for the VM), table fitting, colour and barcode tests.
4. **Backend, monitor, alerts, `tmc3500ctl`:** tested with the fake printer.
5. **Real-printer session** (about 2 h, printer on the LAN):
   - Wireshark recording of status and maintenance;
   - side-by-side labels;
   - first real prints;
   - confirm the JH id and maintenance commands.
6. **Hardening and release:** soak, fuzz campaign, packages, operations guide.

## 10. Risks

| risk | mitigation |
|---|---|
| Colour can't be matched closely enough from coverage alone | Use a spectrophotometer in phase 5 and refine the tables |
| The Epson status reply format is different or unavailable over the LAN | SNMP covers the basic states; the Epson reply adds detail |
| Unknown commands are needed for maintenance | Keep maintenance disabled until it's recorded from Windows on the real printer |
| The JH id is validated by the printer | Capture it from the real printer and reuse it, if needed |
| Ghostscript/Poppler is the main attack surface | Use CUPS hardening and only accept jobs from known hosts |
