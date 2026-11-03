#![forbid(unsafe_code)]
//! cargo run -p xtask -- fit-colour <capture.prn> <chart.png> <layout.json> <out.lut>
//! cargo run -p xtask -- derive-colour-mode <base.lut> <ref-preferred.lut> <ref-mode.lut> <out.lut>

const USAGE: &str = "usage:
  xtask fit-colour <capture.prn> <chart.png> <layout.json> <out.lut>
  xtask derive-colour-mode <base.lut> <ref-preferred.lut> <ref-mode.lut> <out.lut>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [cmd, job, png, layout, out] if cmd == "fit-colour" => run_fit(job, png, layout, out),
        [cmd, base, pref, mode, out] if cmd == "derive-colour-mode" => {
            run_derive(base, pref, mode, out)
        }
        _ => Err(USAGE.to_string()),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

fn read_lut(path: &str) -> Result<tmc3500_core::color::Lut, String> {
    tmc3500_core::color::Lut::parse(&read(path)?).map_err(|e| format!("{path}: {e}"))
}

fn run_fit(job: &str, png: &str, layout: &str, out: &str) -> Result<(), String> {
    let job = read(job)?;
    let img = xtask::read_chart_png(&read(png)?).map_err(|e| format!("{png}: {e}"))?;
    let layout: xtask::Layout =
        serde_json::from_slice(&read(layout)?).map_err(|e| format!("layout: {e}"))?;
    let (lut, report) = xtask::fit(&job, &img, &layout)?;
    std::fs::write(out, lut.to_bytes()).map_err(|e| format!("{out}: {e}"))?;
    for l in &report.lines {
        println!("{l}");
    }
    println!("max grey/ramp error vs table: {:.3}", report.max_ramp_error);
    Ok(())
}

fn run_derive(base: &str, pref: &str, mode: &str, out: &str) -> Result<(), String> {
    let lut = xtask::derive_mode(&read_lut(base)?, &read_lut(pref)?, &read_lut(mode)?)
        .ok_or("tables have different grids")?;
    std::fs::write(out, lut.to_bytes()).map_err(|e| format!("{out}: {e}"))
}
