# Phase 4 (backend, monitor, alerts, tmc3500ctl) implementation plan

**Goal:** Everything around the job bytes: a CUPS backend that delivers jobs safely, a status monitor,
alert hooks and an admin CLI. All tested against a simulated printer; no hardware needed.

**Spec:** `docs/superpowers/specs/2026-11-03-tmc3500-linux-driver-design.md`, sections 1, 2, 5, 6, 7, 8.

**Architecture:**
- `tmc3500-core::status` (pure, std only): BER/SNMP v1/v2c codec, typed `PrinterStatus` from the
  Printer MIB / Host Resources MIB, mapping to CUPS `printer-state-reasons` and alert events, and an
  extension point for the Epson status reply (format unknown until phase 5).
- `tmc3500-common` (lib): config file, `tmc3500://` URI, blocking UDP SNMP client with timeouts, alert
  dispatch (JSON on stdin to `alert.d/*`, per-key repeat suppression with a state file), CUPS exit codes.
- `tmc3500-fakeprinter` (lib + bin): TCP job port validating jobs with `decode::decode`, UDP SNMP agent
  with scripted state, fault injection by in-process API (tests) and CLI flags / stdin commands (bin).
- `tmc3500-backend` (bin `tmc3500`): CUPS backend.
- `tmc3500-monitor` (bin): SNMP poller, alerts on change, Prometheus textfile.
- `tmc3500ctl` (bin): admin CLI with `--json`.

**Global constraints:** `#![forbid(unsafe_code)]` in every crate; no panics on external input; no
async runtime; every socket operation has a timeout; replies are size-capped. Do not modify existing
core files except `lib.rs` (`pub mod status;`). Commit per task after `cargo fmt --all`,
`cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`.

**Dependencies:** `serde`, `serde_json`, `toml`, `clap` (derive), `signal-hook`; dev: `proptest`,
`tempfile`.

---

### Task 1: `status::ber` and `status::snmp` codec
- [x] Tests: encode GetRequest matches a hand-assembled reference packet (net-snmp layout); round trip
  request → decode; decode a GetResponse with INTEGER, OCTET STRING, OID, Counter32, Gauge32,
  noSuchObject; reject: > 64 KiB, nesting > 8, indefinite length, length past end, trailing bytes,
  non-minimal/oversized INTEGER, mismatched request-id, wrong PDU type, bad version; OID encode/decode
  incl. large sub-identifiers; proptest: arbitrary bytes never panic; arbitrary varbind lists round trip.
- [x] Implement, see tests pass, commit.

### Task 2: `status::printer` and `status::epson`
- [x] Tests: hrPrinterDetectedErrorState bit decoding (all 15 bits, short and long octet strings);
  supplies identified by description (K/C/M/Y, maintenance box), percent computation with -1/-2/-3
  special values and max ≤ 0; reasons: paper out → `media-empty-error`, jam → `media-jam-error`, door →
  `cover-open`, ink low/empty, waste almost full/full, offline → `offline-report`, device down without
  cause → `other-error`; `blocking()`; alert conditions with keys; CUPS `ATTR:` marker values; Epson
  extension point returns `Unknown` and never panics (proptest).
- [x] Implement, commit.

### Task 3: `tmc3500-common`
- [x] Tests: config defaults when the file is missing; full TOML parse; unknown keys rejected; file
  size cap; URI parse (`host`, `host:port`, `[v6]:port`, query `snmp-port`, `community`, percent
  decoding; rejects bad scheme/port/host); SNMP client against a local UDP responder (timeout, garbage,
  mismatched id, retry); alert dispatch runs every executable hook with JSON on stdin, skips
  non-executable and world-writable files, kills a hook after its timeout; suppression: second send
  within the interval is suppressed, after `clear(key)` it is sent again, corrupt state file ignored;
  RFC 3339 time formatting.
- [x] Implement, commit.

### Task 4: `tmc3500-fakeprinter`
- [x] Tests: accepts a capture job and stores it byte-identical; rejects garbage (stored as rejected);
  SNMP get/get-next of the scripted values; paper out stops reading and sets noPaper; drop after N
  bytes; refuse connections (connect fails, works again after clearing); garbage and silent SNMP.
- [x] Implement lib + bin (CLI flags, stdin control commands), commit.

### Task 5: `tmc3500-backend`
- [x] Tests (unit): argv handling, discovery line, state-reason diffing to `STATE:` lines, backoff
  schedule. Integration (binary vs fake printer): normal job byte-identical (file and stdin); paper out
  mid-job then cleared → completes, `STATE: +media-empty-error` / `-media-empty-error`; error before
  start → waits, reports, prints after clearing; connection dropped mid-job → exit 3, `job-held` hook,
  at most one partial delivery, no second attempt; unreachable → retries then exit 3 + `unreachable`
  and `job-held` alerts; unreachable then reachable → prints; garbage SNMP and silent SNMP → WARNING,
  job printed; slow reads → completes; SIGTERM → clean exit 5; invalid config → exit 4.
- [x] Implement, commit.

### Task 6: `tmc3500-monitor` and systemd unit
- [x] Tests: state diff → events (new condition, cleared, unreachable, recovered, state-change);
  Prometheus text rendering; atomic textfile write; integration: `--once` against the fake writes
  metrics and runs hooks; SIGTERM ends the loop promptly.
- [x] Implement, add `packaging/systemd/tmc3500-monitor.service`, commit.

### Task 7: `tmc3500ctl`
- [x] Tests: `status`/`ink`/`counters` human and `--json` against the fake; `nozzle-check` / `clean`
  refuse (and refuse differently with the hidden flag); `setup` writes config, prints `lpadmin`, runs it
  only with `--apply` (fake `lpadmin` in PATH); `release-held-jobs` parses `lpstat -l` output, `--dry-run`
  lists, otherwise calls `lp -i <id> -H resume` (fake tools in PATH).
- [x] Implement, commit.

### Task 8: example hooks and packaging files
- [x] `packaging/alert.d/{50-syslog,60-mail,70-webhook}` (shell, config via environment file), example
  `packaging/tmc3500.toml`; test: hooks are executable, pass `sh -n`; example config parses.
- [x] Commit.
