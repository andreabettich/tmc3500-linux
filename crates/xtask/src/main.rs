#![forbid(unsafe_code)]
//! cargo run -p xtask -- fit-colour <capture.prn> <layout.json> <out.lut>

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [cmd, job, layout, out] if cmd == "fit-colour" => run_fit(job, layout, out),
        _ => Err("usage: xtask fit-colour <capture.prn> <layout.json> <out.lut>".to_string()),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run_fit(job: &str, layout: &str, out: &str) -> Result<(), String> {
    let job = std::fs::read(job).map_err(|e| format!("{job}: {e}"))?;
    let layout: xtask::Layout = serde_json::from_str(
        &std::fs::read_to_string(layout).map_err(|e| format!("{layout}: {e}"))?,
    )
    .map_err(|e| format!("layout: {e}"))?;
    let (lut, report) = xtask::fit(&job, &layout)?;
    std::fs::write(out, lut.to_bytes()).map_err(|e| format!("{out}: {e}"))?;
    for l in &report.lines {
        println!("{l}");
    }
    println!("max grey/ramp error vs table: {:.3}", report.max_ramp_error);
    Ok(())
}
