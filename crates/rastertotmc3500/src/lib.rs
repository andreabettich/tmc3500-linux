//! CUPS filter `rastertotmc3500`: reads CUPS raster, writes the Epson TM-C3500 job.
//!
//! argv: job-id user title copies options [file]. Option defaults come from the PPD (`PPD` env), then
//! the page size recorded in the raster, then the job options (later ones win).
#![forbid(unsafe_code)]
use std::io::{Read, Write};
use tmc3500_core::color::Converter;
use tmc3500_core::cupsraster::{PageHeader, Reader};
use tmc3500_core::options::JobConfig;
use tmc3500_core::pipeline::render_page;
use tmc3500_core::protocol::job::{JobMeta, JobWriter, Page};

/// The id the Windows host sent in `JH`; used when no machine id is available.
pub const FALLBACK_HOST_ID: [u8; 6] = [0x7c, 0xed, 0x8d, 0x69, 0xeb, 0xc7];
const MAX_PPD: usize = 1 << 20;

/// `*DefaultX: value` lines of the PPD as an option string (only options the driver uses).
pub fn ppd_defaults(ppd: &str) -> String {
    let mut out = Vec::new();
    for line in ppd.lines().take(100_000) {
        let Some(rest) = line.strip_prefix("*Default") else {
            continue;
        };
        let Some((key, value)) = rest.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let wanted = key.starts_with("Tm") || key == "PageSize";
        let clean = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_graphic() && c != '"' && c != '\'' && c != '\\')
        };
        if wanted && clean(key) && clean(value) && value != "Custom" {
            out.push(format!("{key}={value}"));
        }
    }
    out.join(" ")
}

/// PageSize option derived from the raster page header.
pub fn page_size_from_header(h: &PageHeader) -> Option<String> {
    let n = h.page_size_name.as_str();
    if n == "Label108x174.8mm"
        || (n.starts_with("Custom.") && n.chars().all(|c| c.is_ascii_graphic()))
    {
        return Some(n.to_string());
    }
    let [w, l] = h.page_size_pt;
    (w > 0 && l > 0)
        .then(|| format!("PageSize=Custom.{w}x{l}"))
        .map(|s| s["PageSize=".len()..].to_string())
}

/// Stable 6-byte host id from the machine id (FNV-1a).
pub fn host_id(machine_id: &str) -> [u8; 6] {
    let id = machine_id.trim();
    if id.is_empty() {
        return FALLBACK_HOST_ID;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let b = h.to_be_bytes();
    [b[0], b[1], b[2], b[3], b[4], b[5]]
}

/// Job metadata from a Unix time (UTC) and the CUPS job id.
pub fn meta(unix_secs: u64, job_id: u32, host: [u8; 6]) -> JobMeta {
    let days = (unix_secs / 86_400) as i64;
    let secs = unix_secs % 86_400;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as u16;
    JobMeta {
        year,
        month,
        day,
        hour: (secs / 3600) as u8,
        minute: (secs / 60 % 60) as u8,
        second: (secs % 60) as u8,
        job_id,
        host_id: host,
    }
}

pub struct Env {
    pub ppd: Option<String>,
    /// Directory with fitted colour tables (`None` = provisional colours only).
    pub colour_dir: Option<std::path::PathBuf>,
    pub unix_secs: u64,
    pub host_id: [u8; 6],
}

/// Run the filter; returns the process exit code. Messages go to `log` with CUPS prefixes.
pub fn run<R: Read, W: Write, L: Write>(
    args: &[String],
    input: R,
    out: W,
    log: &mut L,
    env: &Env,
) -> i32 {
    match run_inner(args, input, out, log, env) {
        Ok(()) => 0,
        Err(msg) => {
            let _ = writeln!(log, "ERROR: {msg}");
            1
        }
    }
}

fn run_inner<R: Read, W: Write, L: Write>(
    args: &[String],
    input: R,
    out: W,
    log: &mut L,
    env: &Env,
) -> Result<(), String> {
    if !(6..=7).contains(&args.len()) {
        return Err("usage: rastertotmc3500 job-id user title copies options [file]".into());
    }
    let job_id: u32 = args[1].parse().unwrap_or(0);
    let copies: u32 = args[4]
        .parse()
        .map_err(|_| format!("invalid copies {:?}", args[4]))?;
    let defaults = env.ppd.as_deref().map(ppd_defaults).unwrap_or_default();

    let mut rd = Reader::new(input).map_err(|e| e.to_string())?;
    let first = rd.next_page().map_err(|e| e.to_string())?;
    let header_size = first
        .as_ref()
        .and_then(page_size_from_header)
        .map(|s| format!("PageSize={s}"))
        .unwrap_or_default();
    let options = format!("{defaults} {header_size} {}", args[5]);
    let cfg = JobConfig::from_cups(&options, copies).map_err(|e| e.to_string())?;
    let conv = converter(&cfg, env.colour_dir.as_deref(), log);
    let mut jw = JobWriter::begin(
        std::io::BufWriter::new(out),
        &cfg,
        &meta(env.unix_secs, job_id, env.host_id),
    )
    .map_err(|e| format!("write error: {e}"))?;
    let io = |e: std::io::Error| format!("write error: {e}");
    // Any error after the job header still closes the job (LD/JE), so the printer is never left
    // inside an unfinished job; the error is reported afterwards.
    let mut pages_sent = 0u32;
    let result = write_pages(&mut rd, first, &cfg, &conv, &mut jw, log, &mut pages_sent);
    if let Err(e) = result {
        let _ = jw.end_set();
        let _ = jw.finish();
        return Err(e);
    }
    jw.finish().map_err(io)?;
    let _ = writeln!(log, "INFO: {pages_sent} page(s) sent");
    Ok(())
}

fn write_pages<R: Read, W: Write, L: Write>(
    rd: &mut Reader<R>,
    first: Option<PageHeader>,
    cfg: &JobConfig,
    conv: &Converter,
    jw: &mut JobWriter<W>,
    log: &mut L,
    pages_sent: &mut u32,
) -> Result<(), String> {
    let io = |e: std::io::Error| format!("write error: {e}");
    let collate = cfg.collate && cfg.copies > 1;
    let mut set: Vec<Page> = Vec::new();
    let mut header = first;
    let mut n = 0u32;
    while let Some(h) = header {
        n += 1;
        let page = render_page(rd, &h, cfg, conv).map_err(|e| format!("page {n}: {e}"))?;
        if collate {
            jw.page(&page).map_err(io)?;
            set.push(page);
            let _ = writeln!(log, "PAGE: {n} 1");
        } else {
            for _ in 0..cfg.copies {
                jw.page(&page).map_err(io)?;
            }
            let _ = writeln!(log, "PAGE: {n} {}", cfg.copies);
        }
        header = rd.next_page().map_err(|e| e.to_string())?;
    }
    if collate {
        jw.end_set().map_err(io)?;
        for _ in 1..cfg.copies {
            for p in &set {
                jw.page(p).map_err(io)?;
            }
            jw.end_set().map_err(io)?;
        }
    }
    *pages_sent = n;
    Ok(())
}

/// File name of the colour table for a job: `{coating}-{quality}-{mode}.lut`.
pub fn colour_table_name(cfg: &JobConfig) -> String {
    use tmc3500_core::options::{Coating, ColorMode, Gamma, Quality};
    let coating = match cfg.coating {
        Coating::Plain => "plain",
        Coating::Matte => "matte",
        Coating::Synthetic => "synthetic",
        Coating::Wristband => "wristband",
        Coating::Glossy => "glossy",
    };
    let quality = match cfg.quality {
        Quality::Speed => "speed",
        Quality::Quality1 => "q1",
        Quality::Quality2 => "q2",
    };
    let mode = match (cfg.color.mode, cfg.color.gamma) {
        (ColorMode::Off, _) => "off",
        (_, Gamma::G18) => "gamma18",
        (ColorMode::Standard, _) => "standard",
        (ColorMode::Preferred, _) => "preferred",
    };
    format!("{coating}-{quality}-{mode}.lut")
}

/// Colour converter for the job: the fitted table if one is installed, else the provisional formula.
pub fn converter<L: Write>(
    cfg: &JobConfig,
    dir: Option<&std::path::Path>,
    log: &mut L,
) -> Converter {
    let name = colour_table_name(cfg);
    let Some(path) = dir.map(|d| d.join(&name)) else {
        return Converter::new(&cfg.color, cfg.black);
    };
    let bytes = match std::fs::File::open(&path) {
        Ok(f) => {
            let mut b = Vec::new();
            match f.take(1 << 20).read_to_end(&mut b) {
                Ok(_) => b,
                Err(e) => {
                    let _ = writeln!(
                        log,
                        "WARNING: cannot read colour table {}: {e}; using provisional colours",
                        path.display()
                    );
                    return Converter::new(&cfg.color, cfg.black);
                }
            }
        }
        Err(_) => {
            let _ = writeln!(
                log,
                "INFO: no colour table {name}; using provisional colours"
            );
            return Converter::new(&cfg.color, cfg.black);
        }
    };
    match tmc3500_core::color::Lut::parse(&bytes) {
        Ok(lut) => {
            if cfg.black == tmc3500_core::options::BlackMode::KOnly {
                let _ = writeln!(
                    log,
                    "INFO: TmBlack=KOnly: greys and black with black ink only, colours from {name}"
                );
            }
            Converter::with_lut(&cfg.color, cfg.black, lut)
        }
        Err(e) => {
            let _ = writeln!(
                log,
                "WARNING: {e} in {}; using provisional colours",
                path.display()
            );
            Converter::new(&cfg.color, cfg.black)
        }
    }
}

pub fn read_ppd(path: &str) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut s = String::new();
    f.take(MAX_PPD as u64).read_to_string(&mut s).ok()?;
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ppd_defaults() {
        let ppd = "*PPD-Adobe: \"4.3\"\n*DefaultTmCoating: Glossy\n*DefaultPageSize: Label108x174.8mm\n*DefaultResolution: 360dpi\n*DefaultTmBad: \"x y\"\n*DefaultPageRegion: Custom\n";
        assert_eq!(
            ppd_defaults(ppd),
            "TmCoating=Glossy PageSize=Label108x174.8mm"
        );
    }

    #[test]
    fn utc_time_conversion() {
        let m = meta(1_793_662_697, 2, FALLBACK_HOST_ID); // 2026-11-02 23:38:17 UTC
        assert_eq!(
            (m.year, m.month, m.day, m.hour, m.minute, m.second),
            (2026, 11, 2, 23, 38, 17)
        );
        let m = meta(951_782_400, 1, FALLBACK_HOST_ID); // 2000-02-29 00:00:00
        assert_eq!((m.year, m.month, m.day), (2000, 2, 29));
    }

    #[test]
    fn host_id_is_stable_and_has_fallback() {
        assert_eq!(host_id(""), FALLBACK_HOST_ID);
        assert_eq!(host_id("abc\n"), host_id("abc"));
        assert_ne!(host_id("abc"), host_id("abd"));
    }

    #[test]
    fn colour_table_names() {
        let n = |o: &str| colour_table_name(&JobConfig::from_cups(o, 1).unwrap());
        assert_eq!(n(""), "matte-speed-preferred.lut");
        assert_eq!(
            n("TmCoating=Glossy TmQuality=Quality2"),
            "glossy-q2-preferred.lut"
        );
        assert_eq!(n("TmColorMode=Standard"), "matte-speed-standard.lut");
        assert_eq!(n("TmGamma=1.8"), "matte-speed-gamma18.lut");
        assert_eq!(n("TmColorMode=Off TmGamma=1.8"), "matte-speed-off.lut");
    }

    #[test]
    fn uses_the_colour_table_when_present() {
        use tmc3500_core::color::Lut;
        let dir = std::env::temp_dir().join(format!("tmc3500-lut-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // every colour except white → large black drops only
        let nodes = (0..27)
            .map(|i| {
                if i == 26 {
                    [[0; 3]; 4]
                } else {
                    [[0, 0, 65535], [0; 3], [0; 3], [0; 3]]
                }
            })
            .collect();
        std::fs::write(
            dir.join("matte-speed-preferred.lut"),
            Lut::from_nodes(3, nodes).unwrap().to_bytes(),
        )
        .unwrap();
        let cfg = JobConfig::default();
        let mut log = Vec::new();
        let conv = converter(&cfg, Some(&dir), &mut log);
        assert_eq!(
            conv.drops([0, 255, 255]),
            [[0, 0, 65535], [0; 3], [0; 3], [0; 3]]
        );
        // corrupt table: warning and provisional colours
        std::fs::write(dir.join("matte-speed-preferred.lut"), b"TMCLUT1junk").unwrap();
        let conv = converter(&cfg, Some(&dir), &mut log);
        assert_eq!(conv.drops([0, 255, 255])[1], [0, 0, 65535]);
        let log = String::from_utf8(log).unwrap();
        assert!(
            log.contains("WARNING:") && log.contains("matte-speed-preferred.lut"),
            "{log}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
