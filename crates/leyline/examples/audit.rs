use leyline::audit::{FieldOutcome, Observed};
use leyline::{Browser, Platform, Session};

const ECHO_URL: &str = "https://tls.peet.ws/api/all";

fn show(name: &str, outcome: &FieldOutcome) {
    match outcome {
        FieldOutcome::Match => println!("{name:<15} match"),
        FieldOutcome::Mismatch { expected, observed } => {
            println!("{name:<15} MISMATCH sent {expected}, echo saw {observed}")
        }
        FieldOutcome::Informational { expected, observed } => {
            println!(
                "{name:<15} differs, expected for this profile: sent {expected}, echo saw {observed}"
            )
        }
        FieldOutcome::NotReported => println!("{name:<15} not reported"),
        _ => println!("{name:<15} {outcome:?}"),
    }
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::default())
        .platform(Platform::Linux)
        .audit(true)
        .build()?;

    let resp = session.get(ECHO_URL).send().await?;
    let Some(audit) = resp.audit().cloned() else {
        eprintln!("no audit data on this response");
        return Ok(());
    };
    println!("JA4T:           {}", audit.ja4t);
    println!("JA4H:           {}", audit.ja4h);

    let observed = Observed::from_json(&resp.text().await?)?;
    let report = audit.compare(&observed);
    show("JA4", &report.ja4);
    show("JA3", &report.ja3);
    show("H2 fingerprint", &report.h2_fingerprint);
    println!("fingerprint matches: {}", report.is_match());
    Ok(())
}
