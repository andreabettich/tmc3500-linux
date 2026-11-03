# Packaging notes (TM-C3500 driver)

Install layout, owners and modes the code relies on. The `.deb` (phase 6) must reproduce this.

| path | owner:group | mode | why |
|---|---|---|---|
| `/usr/lib/cups/backend/tmc3500` | root:root | 0755 | CUPS runs a non-0700 backend as `lp` (spec section 6). |
| `/usr/bin/tmc3500-monitor`, `/usr/bin/tmc3500ctl` | root:root | 0755 | |
| `/etc/tmc3500/tmc3500.toml` | root:root | 0644 | Read by backend (`lp`), monitor and ctl. Contains the SNMP community only. **Not a dpkg conffile**: `tmc3500ctl setup` rewrites it, so postinst copies it from `/usr/share/tmc3500/tmc3500.toml.example` only when missing, upgrades never touch it, and purge removes it. Changes to `packaging/tmc3500.toml` therefore reach only new installs. |
| `/etc/tmc3500/alert.d/` | root:root | 0755 | Hooks are skipped unless the directory and each hook are owned by root (or the running user) and not group/world-writable; symlinked hooks are skipped. |
| `/etc/tmc3500/alert.d/*` | root:root | 0755 | Example hooks from `packaging/alert.d/`. |
| `/etc/tmc3500/alert.env` | root:root | 0644 | `KEY=value` settings for the mail/webhook hooks (read, never sourced). Use 0640 root:lp plus the tmc3500 user in group lp if it holds secrets. |
| `/var/lib/tmc3500/` | root:lp | 2770 | Shared state: `alerts.json` (repeat suppression, written 0660 by atomic rename) and `partial-job-<id>` markers. Writable by the backend (`lp`) and the monitor (user `tmc3500`, `SupplementaryGroups=lp`). |
| textfile dir (default `/var/lib/prometheus/node-exporter/`) | prometheus:tmc3500 | 2775 | The monitor writes `tmc3500.prom` there (mode from `[monitor] textfile_mode`, default 0644 so node_exporter can read it). |

postinst:

```sh
[ -e /etc/tmc3500/tmc3500.toml ] || install -m 0644 /usr/share/tmc3500/tmc3500.toml.example /etc/tmc3500/tmc3500.toml
adduser --system --group --no-create-home --home /nonexistent tmc3500
install -d -o root -g lp -m 2770 /var/lib/tmc3500
```

The monitor unit (`systemd/tmc3500-monitor.service`) therefore uses `SupplementaryGroups=lp` and
`ReadWritePaths=/var/lib/tmc3500` instead of `StateDirectory=` (which would chown the directory to
`tmc3500:tmc3500`).
