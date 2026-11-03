# Phase 5: real-printer session checklist (about 2 h)

Everything the captures and the fake printer could not settle. Do it with the printer on the LAN, the
driver installed from the `.deb` on the print host, and a laptop with Wireshark on the same switch port
(mirror port or the print host itself: `sudo tcpdump -i any -w tmc3500.pcap host <printer-ip>`).

Bring:
- media: 108 × 174.8 mm die-cut labels, plus continuous label stock if available;
- a barcode scanner;
- Windows prints of the same test labels (from the colour session), for side-by-side comparison.

## A. First prints (stop at the first failure and capture it)

| # | action | expect | if not |
|---|---|---|---|
| A1 | `tmc3500ctl status --json` | state idle, 4 inks + maintenance box with sensible % | save the JSON; SNMP OIDs/indices differ (see C) |
| A2 | `lp -d TM-C3500 label.pdf` (test PDF from `docs/operations.md` §2 style) | one label, cut, rectangle 0.25 in from the label edge | photo + `/var/log/cups/error_log` |
| A3 | same with `-o TmQuality=Quality1`, then `Quality2` | prints, no banding, same position | photo |
| A4 | `-o TmBorderless=False` | white margin ≈ 1.5 mm top and bottom | measure |
| A5 | continuous label, `-o TmMediaSaving=LowerUpper` | label length = content + margin; no content cut off | measure the margin under the content (we assume 0; Windows minimum 15 mm) |
| A6 | `-o TmSkipBlank=True` with a 3-page PDF whose page 2 is blank | 2 labels | count; our rule is extrapolated from one capture |
| A7 | `-o PageSize=Custom.4.12x6.88in` | image not shifted onto the liner | left-margin rule (spec §3, `ESC(l`) |
| A8 | `-o TmRotate180=True -o TmPosAdjV=2mm` | rotated, moved 2 mm in the expected direction | note direction (rotate/adjust order) |
| A9 | `-n 3 -o Collate=True` 2-page PDF | 1,2,1,2,1,2 with cuts | |

## B. Barcodes and colour

- **Barcode sheet:** print Code 128, DataMatrix and QR codes at their minimum module sizes in Speed and in
  Quality1. Scan every symbol and record the scanner's grade if it reports one.
- **Side by side with the Windows prints:** compare the colour chart and the brand-colour patches. If a
  spectrophotometer is available, measure the patches (CSV) so the colour tables can be refined.

## C. Status protocol (Wireshark running)

For each, note what `tmc3500ctl status --json` says **and** keep the pcap:
1. idle; 2. printing; 3. paper out (remove media mid-job); 4. cover open; 5. jam if it can be provoked
safely; 6. ink low (if a low cartridge is at hand); 7. maintenance box near full (if available);
8. printer offline/paused button; 9. power-cycle during a job.

Questions the pcaps must answer:
- [ ] Which SNMP values (hrPrinterDetectedErrorState bits, supplies table indices, descriptions, class,
      level direction) the printer actually reports — turn them into parser fixtures.
- [ ] Whether the printer sends an Epson status reply on the 9100 connection (format → `status::epson`).
- [ ] How the printer ends a job connection on 9100: FIN, RST, or stays open — and after how long.
      Decides backend drain behaviour (phase 4 ruling 3).
- [ ] Whether the printer reads the whole job into its buffer before printing (tx_queue 0 ≠ printed).
- [ ] What happens to the next job after a job truncated by cancel (SIGTERM mid-job): does the printer
      reset per connection?

## D. Job and maintenance commands from Windows (needs the Windows VM with the printer reachable)

- [ ] Print once from Windows to the real printer and compare the `JH` 6-byte id with ours; check that the
      printer accepts our id (`/etc/machine-id` hash). If it rejects it, set the Windows id.
- [ ] Record nozzle check and head cleaning from the Windows utility (pcap) → enable
      `tmc3500ctl nozzle-check` / `clean`.
- [ ] Vacuum slider positions 1, 2, 3, 5, 6, 7, 8 (also in the colour kit).

## E. Failure handling (the hard requirements)

| # | action | expect |
|---|---|---|
| E1 | unplug the printer's network cable mid-job | job held, alert `job-held`, nothing reprinted; `tmc3500ctl release-held-jobs` lists it with the warning |
| E2 | paper out mid-job, reload | job continues, `media-empty-error` shown and alerted, then cleared |
| E3 | printer off before a job, on 2 min later | job waits/retries, prints once |
| E4 | `sudo systemctl restart cups` mid-job | job held (partial-send marker), not reprinted |
| E5 | alerts | mail/webhook arrive once per incident, `recovered` when fixed |

## F. After the session

- [ ] Commit pcaps-derived fixtures and fix the status parser.
- [ ] Update the spec's "still to be confirmed" list and close the answered items.
- [ ] AppArmor: after E1–E5, check `journalctl -k | grep tmc3500`; enforce if clean.
