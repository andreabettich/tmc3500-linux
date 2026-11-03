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
`printer-error-policy=stop-printer`, and enables and (re)starts `tmc3500-monitor.service`. Without
`--apply` it writes the configuration file but only prints the `lpadmin` command. Re-run it with the new
address whenever the printer's IP changes.

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
| `TmVacuum` | `Auto`, `0` (weak) … `9` (strong): the Windows slider positions |
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

Colours come from tables in `/usr/share/tmc3500/colour/`, one per coating, quality and colour mode,
fitted from the Windows driver's own output (see the `README` there). Printed through the filter, the
colour chart puts down the same ink as Windows within 0.3 % per colour (median). The Preferred tables
of every coating/quality and Matte Speed's Standard, gamma 1.8 and Off are measured; the other
Standard/gamma 1.8/Off tables are derived from them. A missing or unreadable table makes the filter
fall back to provisional colours with an `INFO`/`WARNING` line in the job log.

For barcodes and black text, `-o TmBlack=KOnly` prints black and greys (R = G = B) with black ink
only; other colours on the same label still come from the table (which, like Windows, builds black
from all four inks).

Tables are rebuilt from Windows chart captures with `tools/build_colour_tables.sh <captures-dir>`
in the source tree.

## 7. Hardening checklist

- **CUPS:** accept jobs only from this host or listed hosts.
  - `sudo cupsctl --no-remote-any --no-share-printers`
  - In `/etc/cups/cupsd.conf`: `Listen localhost:631`, or restrict `<Location />` to the application
    hosts.
  - Disable the web interface if unused: `sudo cupsctl WebInterface=no`.
- **Network:** allow the print host to reach the printer on TCP 9100 and UDP 161 (SNMP). Restrict the
  monitor further with a drop-in. Alert hooks run inside the monitor's sandbox, so list the mail relay,
  the webhook host and the DNS servers as well, or idle-time alerts are silently lost:
  ```ini
  # systemctl edit tmc3500-monitor
  [Service]
  IPAddressDeny=any
  IPAddressAllow=192.0.2.50 localhost <mail-relay-ip> <webhook-ip> <dns-server-ip>
  ```
  Test an alert afterwards (section 5).
- **SNMP:** set a non-default read community on the printer and in `[snmp] community`. A non-default
  SNMP port or community can also be given in the device URI:
  `lpadmin -p TM-C3500 -v 'tmc3500://192.0.2.50:9100/?snmp-port=1161&community=xyz'` (the `/` before `?`
  is required by CUPS).
- **AppArmor:** profiles ship in complain mode and are **not attached on a stock system**: Ubuntu's and
  Debian's cupsd profile runs every backend and filter in its own `third_party` child profile, and
  Raspberry Pi OS has AppArmor off. To use ours, make cupsd hand over to them in
  `/etc/apparmor.d/local/usr.sbin.cupsd`:
  ```
  /usr/lib/cups/backend/tmc3500 Px -> tmc3500-backend,
  /usr/lib/cups/filter/rastertotmc3500 Px -> tmc3500-filter,
  ```
  then `sudo apparmor_parser -r /etc/apparmor.d/usr.sbin.cupsd`. Check with `ps -eZ | grep tmc3500`
  while printing. After a full test (print, cut, held job, alert), check
  `journalctl -k | grep tmc3500`; if clean, `sudo aa-enforce /etc/apparmor.d/tmc3500`.
- **Updates:** an `apt` upgrade restarts `tmc3500-monitor` (if running) but not CUPS; a job in progress
  keeps running with the old backend, and the partial-job markers protect jobs interrupted by a CUPS
  restart. CUPS keeps its own copy of the PPD per queue: re-run `sudo tmc3500ctl setup --printer
  <ip> --apply` after an upgrade so new options (e.g. all vacuum positions) appear in print dialogs;
  the queue's default options are kept.

## 8. Troubleshooting

| symptom | look at |
|---|---|
| job stays "processing" | `lpstat -l -p TM-C3500`; `tmc3500ctl status`; is the printer in error? |
| job held | alert text; `/var/lib/tmc3500/partial-job-*`; check output before releasing |
| "no colour table … provisional" in the job log | the table for that coating/quality/mode is missing from `/usr/share/tmc3500/colour/` |
| filter `ERROR:` | options in the message; `lp -o ...` values must match the PPD names above |
| debugging | `sudo cupsctl --debug-logging`; `/var/log/cups/error_log` |
