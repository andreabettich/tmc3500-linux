# TM-C3500 Linux driver: operations guide

For administrators of the print host (Debian/Ubuntu, amd64 or arm64). The printer must have a fixed IP
address or a DHCP reservation.

## 1. Install

```sh
sudo apt install ./tmc3500_<version>_<arch>.deb        # pulls in cups and cups-filters
sudo tmc3500ctl setup --printer 192.0.2.50 --apply     # queue "TM-C3500" + starts the monitor
lpstat -p TM-C3500
```

`setup` writes `/etc/tmc3500/tmc3500.toml`, creates the CUPS queue with device URI
`tmc3500://192.0.2.50` and the PPD `/usr/share/ppd/tmc3500/tmc3500.ppd`, sets
`printer-error-policy=stop-printer`, and enables `tmc3500-monitor.service`. Without `--apply` it only
prints what it would run.

Installed files:

| path | what |
|---|---|
| `/usr/lib/cups/filter/rastertotmc3500` | CUPS raster → printer commands |
| `/usr/lib/cups/backend/tmc3500` | sends jobs, watches status, holds jobs on errors (runs as `lp`) |
| `/usr/bin/tmc3500-monitor` | idle status polling, alerts, Prometheus textfile (user `tmc3500`) |
| `/usr/bin/tmc3500ctl` | admin CLI, every command supports `--json` |
| `/etc/tmc3500/tmc3500.toml` | configuration (all keys documented in the file) |
| `/etc/tmc3500/alert.d/` | alert hooks (syslog active; mail and webhook inactive until configured) |
| `/var/lib/tmc3500/` | shared state: alert suppression, partial-job markers (root:lp 2770) |
| `/usr/share/tmc3500/colour/` | fitted colour tables (see section 6) |
| `/etc/apparmor.d/tmc3500` | AppArmor profiles, complain mode (section 7) |

## 2. Printing

Print PDFs or images to the `TM-C3500` queue. Options use the PPD names, e.g.

```sh
lp -d TM-C3500 -o PageSize=Custom.50x30mm -o TmCoating=Glossy -o TmQuality=Quality1 label.pdf
lp -d TM-C3500 -n 3 -o Collate=True -o TmAfterPrint=CutLastPage labels.pdf
```

| option | values (default first) |
|---|---|
| `PageSize` | `Label108x174.8mm`, `Custom.<w>x<h>{in,mm,cm,pt}` (width 1.00–4.25 in, length 0.31–44 in) |
| `TmMediaForm` | `DieCutGap`, `DieCutBlackMark`, `ContinuousLabel`, `ContinuousPaper`, `ContinuousPaperBlackMark` |
| `TmCoating` | `Matte`, `Plain`, `Synthetic`, `Glossy`, `Wristband` |
| `TmQuality` | `Speed`, `Quality1`, `Quality2` (Plain: Speed only; Wristband: not Quality2) |
| `TmAfterPrint` | `CutEveryPage`, `CutEveryN` (+ `TmCutInterval=1..255`), `CutLastPage`, `PeelOff`, `CutPosition`, `PrintEnd` |
| `TmMediaSaving` | `None`, `Lower`, `LowerUpper` (continuous forms only) |
| `TmNotify` | `None`, `PageEnd`, `JobEnd` (buzzer) |
| `TmBorderless`, `TmBandingReduction`, `TmBidirectional` | `True` / `False` |
| `TmRotate180`, `TmSkipBlank`, `TmInkAdjust` | `False` / `True` |
| `TmVacuum` | `Auto`, `9`, `4`, `0` (other slider positions not yet captured) |
| `TmPosAdjH`, `TmPosAdjV` | any length, e.g. `0.5mm`, `-0.02in` (±4.09 in / ±44 in) |
| `TmBlack` | `Rich` (as Windows), `KOnly` |
| `TmColorMode` | `Preferred`, `Standard`, `Off`; `TmGamma` `2.2` / `1.8` |
| `TmInkBkCMY`, `TmInkK` | `0` … `-6` (with `TmInkAdjust=True`); `TmBrightness` `-2` … `2` |

Change defaults for everyone with `lpadmin -p TM-C3500 -o TmCoating-default=Glossy`.

For barcodes, render at the printer resolution (360 dpi Speed, 720×360 dpi Quality) or as vectors in the
PDF; the driver never rescales a 360-dpi raster in Speed mode.

## 3. Status and maintenance

```sh
tmc3500ctl status            # state, reasons, supplies   (--json for scripts)
tmc3500ctl ink
tmc3500ctl counters
```

`nozzle-check` and `clean` exist but are disabled until their commands have been recorded from the real
printer (phase 5).

## 4. Errors, held jobs, duplicates

The backend never reprints on its own:

| situation | what happens |
|---|---|
| printer in error before the job (paper out, cover open, …) | job waits; reason shown in CUPS (`lpstat -p -l`) and alerted |
| paper or ink runs out mid-job | connection kept open, job resumes when fixed |
| connection lost mid-job, or printer stalls | job **held**, alert `job-held`; marker in `/var/lib/tmc3500/partial-job-<id>` |
| printer unreachable before start | retries with back-off, then held after `unreachable_hold_after_ms` |
| CUPS restarted or job cancelled after part was sent | marker written; if CUPS restarts the job, it is held instead of reprinted |

A held job may already have printed partly or completely. **Check the output before releasing it.**

```sh
tmc3500ctl release-held-jobs              # lists held jobs with the warning, releases nothing
tmc3500ctl release-held-jobs --job 42     # release one job (prints it again from the start)
cancel TM-C3500-42                        # or drop it
```

If the queue was stopped (`stop-printer` policy, e.g. after a rejected job), fix the cause and run
`cupsenable TM-C3500`. A job with invalid options (e.g. `TmCoating=Plain TmQuality=Quality2`) is rejected
by the filter with an `ERROR:` line naming the option.

## 5. Alerts and metrics

Every event (state change, ink low/empty, maintenance box, media out, jam, cover open, job held or failed,
unreachable, recovered) is passed as JSON on stdin to each executable in `/etc/tmc3500/alert.d/`. Hooks
must be owned by root and not group/world-writable; symlinks are ignored. Repeats are suppressed for
`repeat_interval_s` (default 1 h).

- **syslog** (`50-syslog`): active by default (`journalctl -t tmc3500`).
- **mail** (`60-mail`): install `msmtp`, configure `/etc/msmtprc`, set `MAIL_TO=` in
  `/etc/tmc3500/alert.env`. (`sendmail` from Postfix/Exim does not work under the monitor's
  `NoNewPrivileges` sandbox.)
- **webhook** (`70-webhook`): set `WEBHOOK_URL=` in `/etc/tmc3500/alert.env` (needs `curl`).

The monitor writes `/var/lib/prometheus/node-exporter/tmc3500.prom` every 30 s for node_exporter's
textfile collector (create the directory if node_exporter is not installed, or change `[monitor] textfile`).

Test the chain: `sudo -u tmc3500 /etc/tmc3500/alert.d/50-syslog < /dev/null` should log a line.

## 6. Colour tables

Brand colours depend on fitted colour tables in `/usr/share/tmc3500/colour/`, one per coating, quality and
colour mode. Until they are installed the driver prints with provisional colours (barcodes and black text
are unaffected). Tables are produced from Windows colour-chart captures with
`cargo run -p xtask -- fit-colour <capture.prn> <layout.json> <name>.lut` (see
`tmc3500_colour_kit/README.md`).

## 7. Hardening checklist

- **CUPS:** accept jobs only from this host or listed hosts.
  - `sudo cupsctl --no-remote-any --no-share-printers`
  - In `/etc/cups/cupsd.conf`: `Listen localhost:631`, or restrict `<Location />` to the application
    hosts.
  - Disable the web interface if unused: `sudo cupsctl WebInterface=no`.
- **Network:** allow the print host to reach the printer on TCP 9100 and UDP 161 (SNMP) only. Restrict the
  monitor further with a drop-in:
  ```ini
  # systemctl edit tmc3500-monitor
  [Service]
  IPAddressDeny=any
  IPAddressAllow=192.0.2.50 localhost
  ```
- **SNMP:** set a non-default read community on the printer and in `[snmp] community`. A non-default
  SNMP port or community can also be given in the device URI:
  `lpadmin -p TM-C3500 -v 'tmc3500://192.0.2.50:9100/?snmp-port=1161&community=xyz'` (the `/` before `?`
  is required by CUPS).
- **AppArmor:** profiles ship in complain mode.
  - After a full test (print, cut, held job, alert), check `journalctl -k | grep tmc3500`.
  - If the log is clean, run `sudo aa-enforce /etc/apparmor.d/tmc3500`.
  - On Ubuntu, a confined cupsd may run backends in its own child profile, so check with `ps -eZ` while
    printing.
- **Updates:** `apt` upgrades restart nothing; jobs in progress are protected by the partial-job markers.

## 8. Troubleshooting

| symptom | look at |
|---|---|
| job stays "processing" | `lpstat -l -p TM-C3500`; `tmc3500ctl status`; is the printer in error? |
| job held | alert text; `/var/lib/tmc3500/partial-job-*`; check output before releasing |
| "no colour table … provisional" in the job log | expected until colour tables are installed |
| filter `ERROR:` | options in the message; `lp -o ...` values must match the PPD names above |
| debugging | `sudo cupsctl --debug-logging`; `/var/log/cups/error_log` |
