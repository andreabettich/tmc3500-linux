# Working on this repository (for LLM agents and new developers)

A Linux CUPS driver, written in Rust, for the Epson TM-C3500 colour label printer. It reproduces
Windows driver v2.6.0.2. Read this file first, then `docs/developer-guide.md` for the code map and
`docs/api.md` for everything the driver exposes and its current state.

## Commands

```sh
source ~/.cargo/env                      # cargo may not be on PATH in a fresh shell
cargo test --workspace                   # all tests, incl. the 105 Windows-capture golden tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
packaging/build-debs.sh                  # static musl .debs for arm64 + amd64 (needs cargo-deb)
(cd fuzz && cargo fuzz run options -- -max_total_time=60)   # targets: see fuzz/fuzz_targets/
```

The toolchain is pinned in `rust-toolchain.toml`. All four commands must pass before every commit.

## Hard rules

1. **No Epson files in the repo or in packages.** Don't add DLLs, LUTs, PPDs, firmware or installers.
   The Windows driver and capture kits are in the parent directory (`../`) for reference only. The
   colour tables in `packaging/colour/` are our own data, fitted from captured *output*, and may
   ship.
2. **Windows is the reference.** Every printer command the driver emits must match a Windows capture
   byte for byte, except the timestamp, job ID and raster bytes. If you change `protocol`,
   `geometry`, `layout` or `weave`, the golden tests (`crates/tmc3500-core/tests/golden.rs`,
   `crates/rastertotmc3500/tests/golden_raster.rs`) are the judge. Never "fix" a golden test by
   loosening it: find out what Windows does.
3. **Never reprint automatically.** If a job may have partly printed (connection lost, stall, CUPS
   restart), the backend *holds* it and alerts. Labels are often serialised (tickets, shipping), so a
   duplicate is worse than a missing label. Don't add retries after the first byte has been sent.
4. **Parsers never panic.** `tmc3500-core` and every parser use `#![forbid(unsafe_code)]`, explicit
   size limits and checked arithmetic. Anything that reads untrusted bytes (CUPS raster, options,
   SNMP, colour tables, printer replies) gets a fuzz target.
5. **Maintenance stays disabled until it's verified.** `nozzle-check` and `clean` exit with code 3
   until their command bytes have been recorded from Windows on a real printer. Don't guess printer
   commands: an unknown command can stall or misconfigure a production printer.
6. **Test first.** Write the failing test, watch it fail, then fix (bug fixes included).
7. Commit messages end with `Co-Authored-By: ...` when an agent wrote the change. Don't push unless
   asked.

## Where the truth lives

| question | source |
|---|---|
| What Windows sends for a setting | `testdata/captures/*.prn` + `*.drvlog.txt`, indexed in `manifest.csv`, explained in `NOTES.md` / `NOTES-session2.md` |
| Which CUPS options reproduce each capture | `testdata/golden.toml` |
| Protocol, SN block, geometry rules | `docs/superpowers/specs/2026-11-03-tmc3500-linux-driver-design.md` §3 |
| Why a design decision was made | the spec, and the plans in `docs/superpowers/plans/` (rulings are recorded there) |
| How admins install and operate it | `docs/operations.md` |
| What still needs the real printer | `docs/phase5-real-printer-checklist.md` |
| Colour table format and accuracy | `packaging/colour/README`, `crates/tmc3500-core/src/color.rs` |
| Windows driver reverse engineering | `../RE_FINDINGS.md` (outside the repo) |

To inspect a job file, run `python3 tools/escpdump.py <file.prn>` (command listing), or
`python3 tools/cmddiff.py` to diff each capture's commands against its reference capture.

## Traps that cost time before

- **Geometry rounding:** round sizes to 0.1 mm (half up) first, then convert to 1/360 in (half up).
  Converting directly is off by one dot for some sizes, and the SN checksum then differs.
- **Left margin:** `ESC(l` = edge (28 for label forms, 0 for continuous paper) + half the media
  width beyond 1475 dots, rounded down. It is not a fixed offset.
- **Pass grid with Borderless off:** it moves down 21 rows in Speed, 11 in Quality1 and 0 in
  Quality2.
- **Raster block edges:** a pass's first and last columns depend on where sparse dots fall, which
  depends on colour and dither. So `golden_raster.rs` uses the provisional colours, not the fitted
  tables. Colour accuracy is tested separately in `crates/xtask/tests/fit.rs`.
- **The session-2 colour chart PNG is malformed** (rows 36 bytes too long). Windows decoded it with
  fixed-length rows, so the print is sheared. `xtask::read_chart_png` reproduces that decode on
  purpose; don't "fix" it. The generator `tools/make_colour_chart.py` itself is fixed.
- **Quality1 and Quality2 colour tables are identical** because Windows uses the same ink amounts for
  both. This is real, not a bug.
- **Colour tables are found by file name** (`{coating}-{quality}-{mode}.lut`, see
  `rastertotmc3500::colour_table_name`). A missing table falls back to provisional colours with a log
  line instead of failing the job. Check the job log after changing names.
- **CUPS device URI:** a query string needs a path first: `tmc3500://host:9100/?snmp-port=1161`.
- **The config is not a dpkg conffile** (since `setup` rewrites it). postinst creates it from
  `/usr/share/tmc3500/tmc3500.toml.example` only when missing, so a change to
  `packaging/tmc3500.toml` reaches new installs only. Don't add it back to `conf-files`.
- **cupsd in the dev container:** right after `cupsctl` restarts cupsd, `lp` can report that the
  queue doesn't exist. Retry after a second.

## Typical changes

`docs/developer-guide.md` §4 has step-by-step recipes: add or change a print option, change a
printer command, add a status reason or alert, rebuild the colour tables, add a Windows capture as a
golden test.
