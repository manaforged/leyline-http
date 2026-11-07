mod cancel;
mod chaos;
mod differential;
mod edge;
mod hostile;
mod life;
mod mux;
mod origin;
mod plan;
mod rival;
mod probe;

use std::fs;
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::runtime::Builder;

#[derive(Clone, Copy)]
enum Scenario {
    Chaos,
    Cancel,
}

#[derive(Clone)]
pub struct Options {
    pub seed: u64,
    pub seconds: u64,
    pub concurrency: usize,
    pub deadline: Duration,
    pub retries: u32,
    pub fault_percent: u64,
    pub client_threads: usize,
    pub origin_threads: usize,
    pub out: Option<String>,
}

#[derive(Default)]
pub struct Verdict {
    failures: Mutex<Vec<String>>,
}

impl Verdict {
    const SHOWN: usize = 40;

    pub fn fail(&self, message: String) {
        self.failures.lock().unwrap().push(message);
    }

    pub fn count(&self) -> usize {
        self.failures.lock().unwrap().len()
    }

    fn report(&self) -> Vec<String> {
        self.failures.lock().unwrap().iter().take(Self::SHOWN).cloned().collect()
    }
}

pub fn number<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> T {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map_or(default, |v| v.parse().ok().unwrap_or_else(|| panic!("bad value for {name}")))
}

fn options(args: &[String]) -> Options {
    let cores = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    Options {
        seed: number(args, "--seed", 1),
        seconds: number(args, "--seconds", 30),
        concurrency: number(args, "--concurrency", 64),
        deadline: Duration::from_millis(number(args, "--deadline-ms", 500)),
        retries: number(args, "--retries", 2),
        fault_percent: number(args, "--fault-percent", 30),
        client_threads: number(args, "--client-threads", cores),
        origin_threads: number(args, "--origin-threads", 2),
        out: args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)).cloned(),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("differential") => return differential::run(&args),
        Some("origin") => edge::serve_origin(&args),
        Some("hostile-origin") => hostile::serve(&args),
        Some("mux-origin") => mux::serve(&args),
        Some("life") => {
            let (options, verdict) = (options(&args), Verdict::default());
            let detail = life::run(&options, &args, &verdict);
            return finish(&options, &verdict, detail);
        }
        Some("reconnect") | Some("idle") => {
            let (options, verdict) = (options(&args), Verdict::default());
            let detail = match args[0].as_str() {
                "reconnect" => mux::run_reconnect(&options, &args, &verdict),
                _ => mux::run_idle(&options, &args, &verdict),
            };
            return finish(&options, &verdict, detail);
        }
        Some("hostile") => {
            let (options, verdict) = (options(&args), Verdict::default());
            let detail = hostile::run(&options, &args, &verdict);
            return finish(&options, &verdict, detail);
        }
        Some("edge") => {
            let (options, verdict) = (options(&args), Verdict::default());
            let detail = edge::run(&options, &args, &verdict);
            return finish(&options, &verdict, detail);
        }
        _ => {}
    }
    let scenario = match args.first().map(String::as_str) {
        Some("chaos") => Scenario::Chaos,
        Some("cancel") => Scenario::Cancel,
        _ => {
            eprintln!("usage: qualify chaos|cancel|differential|origin|edge|hostile-origin|hostile|mux-origin|reconnect|idle|life [--seed N] [--seconds N] [--out PATH]");
            return ExitCode::from(2);
        }
    };
    let options = options(&args);
    let verdict = Verdict::default();
    let runtime = Builder::new_multi_thread()
        .worker_threads(options.client_threads)
        .enable_all()
        .build()
        .expect("client runtime");
    let detail: Value = runtime.block_on(async {
        match scenario {
            Scenario::Chaos => chaos::run(&options, &verdict).await,
            Scenario::Cancel => cancel::run(&options, &verdict).await,
        }
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    finish(&options, &verdict, detail)
}

fn finish(options: &Options, verdict: &Verdict, detail: Value) -> ExitCode {
    let failures = verdict.count();
    let shown = verdict.report();
    let receipt = json!({
        "seed": options.seed,
        "seconds": options.seconds,
        "concurrency": options.concurrency,
        "deadline_ms": options.deadline.as_millis() as u64,
        "retries": options.retries,
        "fault_percent": options.fault_percent,
        "failures": failures,
        "first_failures": shown,
        "detail": detail,
    });
    let text = serde_json::to_string_pretty(&receipt).expect("receipt");
    if let Some(path) = &options.out {
        fs::write(path, &text).expect("write receipt");
    }
    println!("{text}");
    if failures == 0 {
        println!("PASS seed={}", options.seed);
        ExitCode::SUCCESS
    } else {
        println!("FAIL seed={} failures={failures}", options.seed);
        ExitCode::FAILURE
    }
}
