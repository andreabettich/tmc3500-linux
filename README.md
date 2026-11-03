# tmc3500-linux

A Linux CUPS driver for the Epson TM-C3500 colour label printer, written in Rust. It reproduces the
Windows driver v2.6.0.2: the same printer commands for every setting, colours fitted to the Windows
output, and an error policy that never reprints a label on its own.

- Debian/Ubuntu (and Raspberry Pi OS), amd64 and arm64, one static `.deb` per architecture.
- Printer on the LAN (TCP 9100 + SNMP).
- CUPS status and ink levels, alert hooks (syslog, mail, webhook), Prometheus metrics, and an admin
  CLI with JSON output.

```sh
sudo apt install ./tmc3500_<version>_<arch>.deb
sudo tmc3500ctl setup --printer 192.0.2.50 --apply
lp -d TM-C3500 -o PageSize=Custom.50x30mm -o TmCoating=Glossy label.pdf
tmc3500ctl --json status
```

## Documentation

| document | for |
|---|---|
| [docs/operations.md](docs/operations.md) | administrators: install, print options, held jobs, alerts, hardening, troubleshooting |
| [docs/api.md](docs/api.md) | integrators: every interface (options, CLI/JSON, alerts, metrics, config) and what works and what doesn't yet |
| [docs/developer-guide.md](docs/developer-guide.md) | developers: architecture, data flow, tests, recipes for common changes |
| [AGENTS.md](AGENTS.md) | LLM agents and newcomers: commands, hard rules, known traps |
| [docs/superpowers/specs/…-design.md](docs/superpowers/specs/2026-11-03-tmc3500-linux-driver-design.md) | the design and the reverse-engineered protocol |
| [docs/phase5-real-printer-checklist.md](docs/phase5-real-printer-checklist.md) | what still has to be confirmed on a real printer |
| [packaging/README.md](packaging/README.md), [packaging/colour/README](packaging/colour/README) | install layout; colour tables |

## Build and test

```sh
cargo test --workspace          # includes 105 byte-exact comparisons with Windows captures
packaging/build-debs.sh         # target/<triple>/debian/tmc3500_<version>_<arch>.deb
```

## Licence and Epson material

Licensed MIT OR Apache-2.0. The repository and packages contain no Epson files. The colour tables
are our own data, fitted from the Windows driver's printed output.
