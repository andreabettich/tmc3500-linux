# Driver interfaces and capabilities

Everything the TM-C3500 driver exposes to applications, administrators and monitoring, followed by
what is verified, what is only simulated, and what is missing (§9). Version 0.1.0, reference: Epson
Windows driver v2.6.0.2.

Interfaces at a glance:

| interface | used by | section |
|---|---|---|
| CUPS queue `TM-C3500` and its options | applications (`lp`, IPP, print dialogs) | §1 |
| Device URI `tmc3500://...` | CUPS / `lpadmin` | §2 |
| CUPS status (`printer-state-reasons`, job states) | CUPS clients, `lpstat` | §3 |
| `tmc3500ctl` CLI with `--json` | administrators, scripts | §4 |
| Alert hooks (JSON on stdin) | mail, webhook, syslog, your own scripts | §5 |
| Prometheus textfile | node_exporter | §6 |
| `/etc/tmc3500/tmc3500.toml` | administrators | §7 |
| Filter and backend process contract | CUPS | §8 |

## 1. Print options (CUPS queue)

Applications print PDFs or images to the queue. Each option maps to one setting in the Windows
driver dialog. Defaults are the Windows defaults (first value listed). Values are case-sensitive and
must match the PPD.

| option | values | Windows setting |
|---|---|---|
| `PageSize` | `Label108x174.8mm`, `Custom.<w>x<h><unit>` (unit `in`, `mm`, `cm` or `pt`; width 1.00–4.25 in, length 0.31–44 in) | Media size |
| `TmMediaForm` | `DieCutGap`, `DieCutBlackMark`, `ContinuousLabel`, `ContinuousPaper`, `ContinuousPaperBlackMark` | Media form |
| `TmCoating` | `Matte`, `Plain`, `Synthetic`, `Glossy`, `Wristband` | Media coating type |
| `TmQuality` | `Speed`, `Quality1`, `Quality2` (Plain: Speed only; Wristband: no Quality2) | Print quality |
| `TmAfterPrint` | `CutEveryPage`, `CutEveryN` (with `TmCutInterval=1..255`), `CutLastPage`, `PeelOff`, `CutPosition`, `PrintEnd` | After printing |
| `TmMediaSaving` | `None`, `Lower`, `LowerUpper` (continuous forms only) | Media saving |
| `TmNotify` | `None`, `PageEnd`, `JobEnd` | Notification (buzzer) |
| `TmBorderless` | `True`, `False` | Borderless |
| `TmBandingReduction` | `True`, `False` (Speed only) | Banding reduction |
| `TmBidirectional` | `True`, `False` | Bidirectional printing |
| `TmRotate180` | `False`, `True` | Rotate 180° |
| `TmSkipBlank` | `False`, `True` | Skip blank page |
| `TmVacuum` | `Auto`, `0` … `9` | Platen vacuum slider |
| `TmPosAdjH`, `TmPosAdjV` | a length such as `0.5mm` or `-0.02in` (±4.09 in horizontally, ±44 in vertically) | Print position adjustment |
| `TmColorMode` | `Preferred`, `Standard`, `Off` | Colour correction |
| `TmGamma` | `2.2`, `1.8` | Gamma |
| `TmInkAdjust` | `False`, `True` | Ink adjustment on/off |
| `TmInkBkCMY`, `TmInkK` | `0` … `-6` (only with `TmInkAdjust=True`) | Ink profile |
| `TmBrightness` | `0`, `-2` … `2` | Brightness |
| `TmBlack` | `Rich`, `KOnly` | (Linux only) black from all four inks, as Windows does, or black and greys from black ink only, for barcodes |

Standard CUPS options handle the rest of the Windows dialog:

| Windows setting | CUPS |
|---|---|
| Copies, collate | `-n N`, `-o Collate=True` (each copy's pages are sent again, like Windows' high-speed copies) |
| Orientation | `-o orientation-requested=4` |
| Reverse order | `-o outputorder=reverse` |
| Size reduction / fit | `-o fit-to-page=true`, `-o scaling=50` (done by cups-filters) |

Invalid values and combinations the Windows dialog forbids (e.g. `TmCoating=Plain TmQuality=Quality2`)
make the job fail. The job log then has an `ERROR:` line naming the option. The PPD carries the same
constraints, so print dialogs don't offer them. Defaults per queue: `lpadmin -p TM-C3500 -o
TmCoating-default=Glossy`.

**Input resolution:** cups-filters renders at 360×360 dpi (Speed) or 720×360 dpi (Quality: 720 across, 360 down). For
sharp barcodes, send vectors or a raster at exactly that resolution.

## 2. Device URI

```
tmc3500://<host>[:<port>][/][?snmp-port=<n>&community=<c>]
```

`host` is an IPv4 address, a name, or an IPv6 address in brackets. `port` is the raw job port
(default 9100). SNMP defaults come from `[snmp]` in the config file. CUPS only accepts a query string
after a path, so write `tmc3500://192.0.2.50:9100/?snmp-port=1161`. `tmc3500ctl setup` writes the
URI for you.

## 3. CUPS status

The backend reports the printer's condition as standard `printer-state-reasons`, visible in
`lpstat -l -p` and every CUPS client:

| reason | when |
|---|---|
| `media-empty-error`, `media-low-report`, `media-jam-error` | paper out / low / jam |
| `cover-open` | cover open |
| `marker-supply-low-warning`, `marker-supply-empty-error` | an ink at or below `ink_low_percent`, or empty |
| `marker-waste-almost-full-warning`, `marker-waste-full-error` | maintenance box near full / full |
| `offline-report` | printer offline |
| `other-error`, `other-warning`, `input-tray-missing-error`, `output-tray-missing-error`, `marker-supply-missing-error`, `output-area-almost-full-warning`, `output-area-full-error` | other SNMP-reported conditions |

The backend also reports ink levels (`marker-levels`, `marker-names`, `marker-colors`, `marker-types`
with `ink-cartridge`/`waste-ink`), so CUPS web pages and dialogs show them.

Job outcomes:

| situation | job state |
|---|---|
| printed | completed |
| printer in error before the job starts | job waits (processing) and shows the reason |
| paper or ink runs out mid-job | job waits, then continues when it's fixed |
| connection lost or printer stalls mid-job | **held**, alert `job-held`; never resent automatically |
| printer unreachable before start | retried with back-off, then **held** after `unreachable_hold_after_ms` |
| CUPS restarted or job cancelled after part was sent | marker written; if CUPS restarts the job, it is held |
| invalid options | aborted, with an `ERROR:` line in the job log |

## 4. `tmc3500ctl`

```
tmc3500ctl [--json] [--config <path>] <command>
```

| command | does |
|---|---|
| `status [--queue Q \| --all \| --printer <host>]` | state, CUPS reasons, raw SNMP error bits, supplies |
| `ink [--queue Q \| --all \| --printer <host>]` | supplies only |
| `counters [--queue Q \| --all \| --printer <host>]` | `sysDescr`, uptime, life count, power-on count |
| `setup --printer <host> [--queue Q] [--ppd P] [--apply]` | records the printer for queue Q in the config: a known queue gets the new address, a known address gets the new queue name, anything else is added; prints the `lpadmin` command, or with `--apply` runs it, sets `printer-error-policy=stop-printer` and (re)starts the monitor |
| `release-held-jobs [--queue Q] [--dry-run] [--job ID]... [--yes]` | lists held jobs on Q (default: every configured queue) with the reprint warning; releases the jobs named with `--job` (a full ID or a bare job number) or all with `--yes`, and removes their partial-send markers |
| `nozzle-check`, `clean` | **disabled**: always exit 3 (§9) |

**Which printers** `status`, `ink` and `counters` report:

| arguments | printers | JSON |
|---|---|---|
| none, one printer configured | that printer | one object (below) |
| none, several configured, or `--all` | all of them, queried in parallel | `{"printers": [object, ...]}`; exit 2 if any is unreachable |
| `--queue Q` | the printer of queue Q (exit 1 if Q isn't configured) | one object |
| `--printer <host>` | that address, configured or not | one object |

`setup` without `--queue` uses `[printer] queue` (`TM-C3500`).

**Exit codes:** 0 success; 1 error (config, usage, CUPS tool failure); 2 printer unreachable (clap
also uses 2 for usage errors); 3 maintenance command disabled.

**JSON output** (one object on stdout; human messages go to stderr):

```jsonc
// tmc3500ctl --json status
{
  "queue": "TM-C3500",
  "printer": "192.0.2.50",
  "reachable": true,
  "sys_descr": "EPSON TM-C3500",          // null if not reported
  "device_status": "running",             // hrDeviceStatus: unknown|running|warning|testing|down|invalid, or null
  "printer_status": "idle",               // hrPrinterStatus: other|unknown|idle|printing|warmup|invalid, or null
  "blocking": false,                      // true if an error stops printing
  "reasons": ["marker-supply-low-warning"],  // CUPS printer-state-reasons (§3)
  "errors": ["lowToner"],                 // hrPrinterDetectedErrorState bits: lowPaper, noPaper, lowToner,
                                          // noToner, doorOpen, jammed, offline, serviceRequested, inputTrayMissing,
                                          // outputTrayMissing, markerSupplyMissing, outputNearFull, outputFull,
                                          // inputTrayEmpty, overduePreventMaint
  "supplies": [
    { "index": 1, "description": "Cyan", "kind": "ink", "color": "cyan",
      "percent": 8, "state": "low" },     // kind: ink|maintenance-box|other; color: black|cyan|magenta|yellow|null
    { "index": 5, "description": "Maintenance box", "kind": "maintenance-box", "color": null,
      "percent": 70, "state": "ok" }      // percent: remaining (maintenance box: free space), or null;
  ]                                       // state: ok|low|empty|unknown (maintenance box: low = near full)
}

// tmc3500ctl --json ink
{ "queue": "...", "printer": "...", "reachable": true, "supplies": [ ... as above ... ] }

// tmc3500ctl --json counters
{ "queue": "...", "printer": "...", "reachable": true, "sys_descr": "...", "uptime_seconds": 86400,
  "life_count": 12345, "power_on_count": 42 }            // each null if not reported

// any failure (exit code 1, 2 or 3)
{ "ok": false, "error": "...", ... }                     // unreachable adds "queue", "printer", "reachable": false;
                                                         // disabled maintenance adds "command"

// several printers (one unreachable: exit code 2; the list form has no "ok" field, check "reachable")
{ "printers": [ { "queue": "TM-C3500-1", "printer": "192.0.2.51", "reachable": true, ... },
                { "queue": "TM-C3500-2", "printer": "192.0.2.52", "reachable": false, "error": "..." } ] }

// tmc3500ctl --json setup --printer 192.0.2.50 --apply
{ "ok": true, "queue": "TM-C3500", "printer": "192.0.2.50", "config": "/etc/tmc3500/tmc3500.toml",
  "uri": "tmc3500://192.0.2.50", "command": ["lpadmin", "-p", "TM-C3500", ...], "applied": true }

// tmc3500ctl --json release-held-jobs --job 42
// "queue" is null when several queues were covered; a configured queue CUPS doesn't know is skipped
// with a line in "warnings" (asked for with --queue, it's an error)
{ "ok": true, "queue": "TM-C3500", "queues": ["TM-C3500"], "dry_run": false, "held": [...], "released": [...],
  "failed": [{ "id": "...", "error": "..." }], "warning": "Warning: a released job prints again ...",
  "warnings": [] }
```

The sample values are illustrative: the real printer's SNMP descriptions and supply indices are not
known yet (§9).

## 5. Alert hooks

Every executable in `[alerts] dir` (default `/etc/tmc3500/alert.d/`) receives each event as one JSON
object on stdin. Hooks must be owned by root (or the running user) and not group- or world-writable.
Symlinks are skipped. Each hook gets `hook_timeout_ms`. Hook failures are logged and never affect
printing. An event with the same `key` for the same printer is not sent again within
`repeat_interval_s`; the same problem on another printer is reported separately. The backend and the
monitor share the suppression state and both name the printer by its address, so an incident is
reported once.

```json
{
  "event": "ink-low",
  "key": "ink-low:cyan",
  "severity": "warning",
  "printer": "192.0.2.50",
  "queue": "TM-C3500",
  "message": "cyan ink low (8% left)",
  "time": "2026-11-03T10:15:00Z",
  "timestamp": 1793700900,
  "source": "monitor",
  "job_id": "42",
  "reasons": ["marker-supply-low-warning"]
}
```

`queue` is the printer's CUPS queue (omitted if unknown). `job_id` is present on events from the
backend (the job being printed). `source` is `backend` or
`monitor`. Conditions are re-sent only after `repeat_interval_s`. When a condition clears, its key is
reset, so it alerts again at once if it comes back.

| `event` | severity | `key` |
|---|---|---|
| `state-change` | info | `state-change:<reasons>` (monitor; reasons comma-joined, or `none`) |
| `recovered` | info | `recovered` (printer reachable again) |
| `ink-low` | warning | `ink-low:<colour>` |
| `maintenance-box-near-full` | warning | `maintenance-box-near-full` |
| `offline` | warning | `offline` |
| `ink-empty` | error | `ink-empty:<colour>` |
| `maintenance-box-full`, `media-out`, `media-jam`, `cover-open`, `cutter-error` | error | same as the event name |
| `printer-error` | error | `printer-error:<reason>` |
| `job-held` | error | `job-held:<job id>` |
| `job-failed` | error | `job-failed:<job id>` |
| `unreachable` | error | `unreachable` |

Example hooks shipped: `50-syslog` (on), `60-mail` (msmtp) and `70-webhook` (curl, POSTs the JSON).
The mail and webhook hooks are configured in `/etc/tmc3500/alert.env`.

## 6. Prometheus metrics

`tmc3500-monitor` rewrites `[monitor] textfile` (default
`/var/lib/prometheus/node-exporter/tmc3500.prom`) after every poll, with every configured printer in
one file. Every series has the labels `printer="<host>"` and `queue="<CUPS queue>"`.

| metric | type | value |
|---|---|---|
| `tmc3500_up` | gauge | 1 if the last SNMP poll was answered |
| `tmc3500_last_poll_timestamp_seconds` | gauge | Unix time of the last poll |
| `tmc3500_device_status` | gauge | hrDeviceStatus (1 unknown, 2 running, 3 warning, 4 testing, 5 down) |
| `tmc3500_printer_status` | gauge | hrPrinterStatus (1 other, 2 unknown, 3 idle, 4 printing, 5 warmup) |
| `tmc3500_blocking` | gauge | 1 if an error stops printing |
| `tmc3500_state_reason{reason=...}` | gauge | 1 per current CUPS reason |
| `tmc3500_marker_level_percent{index,marker,description}` | gauge | supply remaining (maintenance box: free space); `marker` is a colour, `maintenance-box` or `other` |
| `tmc3500_marker_life_count` | counter | prtMarkerLifeCount |

## 7. Configuration file

`/etc/tmc3500/tmc3500.toml` (or `$TMC3500_CONFIG`, or `tmc3500ctl --config`) is read by the
backend, the monitor and the ctl. Every key is optional, and unknown keys are an error. The
installed file documents every key with its default. Sections:

- `[printer]`: `host`, `port`, `queue`: a single printer;
- `[[printers]]`: further printers, one table each with `queue`, `host` and optional `snmp_port`
  (at most 64; every queue at most once across `[printer]` and `[[printers]]`; IPv6 addresses
  without brackets). `tmc3500ctl setup` maintains both. A mistake in this list is reported by the
  monitor and `tmc3500ctl`; the backend ignores the list (it uses the queue's device URI), so it
  never stops printing;
- `[snmp]`: `enabled`, `port`, `community`, `version` (`1`/`2c`), `timeout_ms`, `retries`;
- `[backend]`: `connect_timeout_ms`, `io_timeout_ms`, `status_interval_ms`, `retry_initial_ms`,
  `retry_max_ms`, `unreachable_hold_after_ms` (10 min), `stall_timeout_ms` (5 min),
  `drain_timeout_ms`;
- `[alerts]`: `dir`, `hook_timeout_ms`, `repeat_interval_s` (1 h), `state_dir`;
- `[monitor]`: `interval_ms` (30 s), `textfile`, `textfile_mode`;
- `[thresholds]`: `ink_low_percent` (10), `waste_near_full_percent` (90).

`TMC3500_COLOUR_DIR` overrides the colour table directory (`/usr/share/tmc3500/colour/`).

## 8. Filter and backend contract (for CUPS integrators)

- **Filter `rastertotmc3500`:**
  - Input: CUPS raster on stdin or as a file. Arguments: `job-id user title copies options [file]`.
  - Output: printer job bytes on stdout.
  - Settings: PPD defaults (`$PPD`), then the page size recorded in the raster, then the job
    options; later ones win.
  - Job log on stderr: `PAGE:` per page; `INFO:` for the colour table used or a fallback;
    `WARNING:` for an unreadable table; `ERROR:` for invalid options.
  - Exits non-zero on error.
  - The 6-byte host id in the `JH` command is derived from `/etc/machine-id`.
- **Backend `tmc3500`:**
  - Device discovery when run without arguments.
  - Standard CUPS exit codes: 0 ok, 1 failed, 3 hold, 4 stop (invalid URI or config), 5 cancel
    (SIGTERM).
  - Writes `STATE:` and `ATTR:` lines for the reasons and marker levels.
  - Partial-send markers: `/var/lib/tmc3500/partial-job-<id>`.

## 9. Capabilities and status

**Verified against Windows driver output** (captures from Windows printing to a file):

| capability | evidence |
|---|---|
| Every option in §1: commands, media block, checksums, geometry, pass structure, compression | 105 captures match byte for byte (timestamp and job ID masked) |
| Copies, collate, reverse order, orientation, rotate 180, position adjustment, skip blank, media saving, all forms, coatings, qualities, cut modes, buzzer, borderless, vacuum 0–9 | individual captures per setting |
| Colours: 15 measured tables (every coating × quality in Preferred mode, plus Matte Speed in Standard, gamma 1.8 and Off) | ink per chart colour within 0.3 % (median), 0.7 % (p90) of Windows |
| Brightness and ink profile | within 1.7 % (median) on a capture not used for fitting |

**Working, but not verified against Windows:**

| capability | limitation |
|---|---|
| Standard, gamma 1.8 and Off for coatings other than Matte Speed (33 tables) | derived from Matte Speed's change, capped at the coating's ink limit; Windows never captured them |
| 145 of 729 chart colours | the session-2 chart didn't print them; inferred from neighbours: typically within 1.3 %, up to about 10 % |
| `TmBlack=KOnly` | our own option; Windows has no equivalent |
| Dither | our own ordered dither aims for the same ink per drop size as Windows; dot positions differ (invisible at 360 dpi, but not byte-identical) |

**Tested only against the simulated printer and CUPS in a container** (no real printer yet):

- backend error policy: hold on loss or stall, wait on paper out, retry when unreachable, cancel,
  CUPS-restart markers;
- SNMP status, CUPS reasons, ink levels, alerts, metrics, `tmc3500ctl`;
- `.deb` install on arm64; amd64 is built the same way.

The phase-5 checklist (`docs/phase5-real-printer-checklist.md`) must still confirm on the real
printer:

- which SNMP values, supply indices and descriptions it reports;
- that it accepts our `JH` host id;
- how it closes the job connection;
- barcode scan grades;
- colour side by side with Windows;
- the order of rotate 180 and position adjustment;
- the media-saving bottom margin.

**Not implemented:**

| capability | why / plan |
|---|---|
| Nozzle check, head cleaning | command bytes not captured yet; the commands exit 3 until recorded from Windows (phase 5) |
| Epson status reply on the job connection | format unknown; replies are logged (hex) for analysis, and status comes from SNMP |
| Error-diffusion dither for photos | not needed for labels; ordered dither is used everywhere |
| Windows driver preferences, log settings, favourites, settings import/export | no effect on printer output; CUPS has its own defaults (`lpadmin -o ...-default=`) |
| Brand-colour verification | waiting for the brand colour list (`name,#RRGGBB`) and a chart rerun |
| AppArmor confinement by default | profiles ship in complain mode and need a cupsd hand-over (`docs/operations.md` §7) |
