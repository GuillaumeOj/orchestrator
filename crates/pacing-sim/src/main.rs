//! `pacing-sim`: replay a week against the Governor and print a summary.
//!
//! ```text
//! cargo run --release -p pacing-sim -- [--scenario full|nfr9-130|nfr9-60] [--demand PCT]
//!     [--seed N] [--shadow] [--file week.json] [--dump-scenario week.json]
//!     [--out DIR] [--json]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use pacing_sim::{Scenario, SyntheticParams, csv, run};

fn usage() -> &'static str {
    "usage: pacing-sim [--scenario full|nfr9-130|nfr9-60] [--demand PCT] [--seed N] [--shadow]\n\
     \x20                 [--file week.json] [--dump-scenario week.json] [--out DIR] [--json]"
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut scenario = "full".to_owned();
    let mut demand: Option<f64> = None;
    let mut seed = 1u64;
    let mut shadow = false;
    let mut file: Option<PathBuf> = None;
    let mut dump: Option<PathBuf> = None;
    let mut out = PathBuf::from("out");
    let mut json = false;
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        let r: Result<(), String> = (|| {
            match a.as_str() {
                "--scenario" => scenario = value("--scenario")?,
                "--demand" => demand = Some(value("--demand")?.parse().map_err(|e| format!("--demand: {e}"))?),
                "--seed" => seed = value("--seed")?.parse().map_err(|e| format!("--seed: {e}"))?,
                "--shadow" => shadow = true,
                "--file" => file = Some(value("--file")?.into()),
                "--dump-scenario" => dump = Some(value("--dump-scenario")?.into()),
                "--out" => out = value("--out")?.into(),
                "--json" => json = true,
                "-h" | "--help" => return Err(String::new()),
                other => return Err(format!("unknown argument {other}")),
            }
            Ok(())
        })();
        if let Err(e) = r {
            if !e.is_empty() {
                eprintln!("{e}");
            }
            eprintln!("{}", usage());
            return ExitCode::from(2);
        }
    }

    let mut sc = match &file {
        Some(path) => {
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("{}: {e}", path.display());
                    return ExitCode::FAILURE;
                }
            };
            match serde_json::from_str::<Scenario>(&text) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{}: {e}", path.display());
                    return ExitCode::FAILURE;
                }
            }
        }
        None => {
            let mut p = match scenario.as_str() {
                "full" => SyntheticParams::full(seed),
                "nfr9-130" => SyntheticParams::nfr9(130.0, seed),
                "nfr9-60" => SyntheticParams::nfr9(60.0, seed),
                other => {
                    eprintln!("unknown scenario {other}\n{}", usage());
                    return ExitCode::from(2);
                }
            };
            if let Some(d) = demand {
                p.demand_pct = d;
                p.name = format!("{}-d{d:.0}", p.name);
            }
            p.seed = seed;
            Scenario::synthetic(&p)
        }
    };
    if shadow {
        sc.shadow_mode = true;
        sc.name.push_str("-shadow");
    }
    if let Some(path) = dump {
        let text = serde_json::to_string_pretty(&sc).expect("serialisable");
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("{}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        eprintln!("scenario written to {}", path.display());
    }

    let result = run(&sc);
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("{}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    let csv_path = out.join(format!("{}.csv", sc.name));
    let actions_path = out.join(format!("{}.actions.jsonl", sc.name));
    let actions: String =
        result.actions.iter().map(|a| serde_json::to_string(a).expect("serialisable") + "\n").collect();
    if let Err(e) = std::fs::write(&csv_path, csv(&result.rows)).and_then(|_| std::fs::write(&actions_path, actions)) {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&result.summary).expect("serialisable"));
    } else {
        print!("{}", result.summary.render());
        println!("curves              {}", csv_path.display());
        println!("action log          {}", actions_path.display());
    }
    ExitCode::SUCCESS
}
