//! Parser for `LC_ALL=C lpstat -l -o <queue>` output.
//!
//! ```text
//! TM-C3500-12   alice   1024   Tue 03 Nov 2026 10:00:00 AM UTC
//! \tStatus: Connection to the printer lost
//! \tAlerts: job-hold-until-specified
//! \tqueued for TM-C3500
//! ```
//! A job is held when its `Alerts:` (job-state-reasons) contain `job-hold-until-specified`, which
//! is what CUPS sets for a backend exit with CUPS_BACKEND_HOLD (and for `lp -H hold`).

/// Ids (`<queue>-<n>`) of held jobs on exactly `queue`, in output order.
pub fn held_jobs(out: &str, queue: &str) -> Vec<String> {
    let mut held = Vec::new();
    let mut current: Option<String> = None;
    let prefix = format!("{queue}-");
    for line in out.lines() {
        if line.is_empty() {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            current = line.split_whitespace().next().and_then(|id| {
                let n = id.strip_prefix(&prefix)?;
                (!n.is_empty() && n.len() <= 10 && n.bytes().all(|c| c.is_ascii_digit()))
                    .then(|| id.to_string())
            });
            continue;
        }
        let Some(id) = &current else { continue };
        if let Some(alerts) = line.trim().strip_prefix("Alerts:") {
            if alerts
                .split(|c: char| c == ',' || c.is_whitespace())
                .any(|a| a == "job-hold-until-specified")
                && !held.contains(id)
            {
                held.push(id.clone());
            }
        }
    }
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_held_jobs_on_the_exact_queue() {
        let out = "TM-C3500-12  a 1 date\n\tAlerts: job-hold-until-specified\n\
                   TM-C3500-13  b 1 date\n\tAlerts: job-incoming\n\
                   TM-C3500-2-14 c 1 date\n\tAlerts: job-hold-until-specified\n\
                   TM-C3500-x  d 1 date\n\tAlerts: job-hold-until-specified\n\
                   TM-C3500-15 e 1 date\n\tStatus: x\n\tAlerts: job-printing, job-hold-until-specified\n";
        assert_eq!(
            held_jobs(out, "TM-C3500"),
            vec!["TM-C3500-12", "TM-C3500-15"]
        );
        assert_eq!(held_jobs(out, "TM-C3500-2"), vec!["TM-C3500-2-14"]);
        assert!(held_jobs("", "TM-C3500").is_empty());
        assert!(held_jobs("\tAlerts: job-hold-until-specified\n", "TM-C3500").is_empty());
    }
}
