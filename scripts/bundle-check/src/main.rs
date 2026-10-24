use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use leyline::Session;
use leyline::profile::{BundleSource, ProfileBundle, ProfileError};

type Outcome = Result<(), Box<dyn Error>>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Outcome {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["official", min_left] => {
            let session = Session::builder().build()?;
            let bundle = ProfileBundle::fetch(&session, &BundleSource::default()).await?;
            official(&bundle)?;
            let left = bundle.expires().duration_since(SystemTime::now())?;
            if left < Duration::from_secs(min_left.parse()?) {
                return Err(format!(
                    "the public bundle expires in {}s; approve the pending profile-bundle deployment",
                    left.as_secs()
                )
                .into());
            }
            Ok(())
        }
        ["file", path] => official(&ProfileBundle::from_bytes(
            &std::fs::read(path)?,
            &BundleSource::default(),
        )?),
        ["fixtures", dir] => fixtures(Path::new(dir)),
        _ => Err("usage: leyline-bundle-check official <min-seconds-left> | file <path> | fixtures <dir>".into()),
    }
}

fn official(bundle: &ProfileBundle) -> Outcome {
    if let Some(skip) = bundle.skipped().first() {
        return Err(format!(
            "{} profiles fail this release, first {} {}: {}",
            bundle.skipped().len(),
            skip.browser,
            skip.version,
            skip.error
        )
        .into());
    }
    println!(
        "official bundle: serial {}, expires {:?}, all profiles load",
        bundle.serial(),
        bundle.expires()
    );
    Ok(())
}

fn fixtures(dir: &Path) -> Outcome {
    let mut key = [0; 32];
    hex::decode_to_slice(
        std::fs::read_to_string(dir.join("key.hex"))?.trim(),
        &mut key,
    )?;
    let fixed_lifetime = BundleSource::new("fixtures", vec![key]);
    let source = fixed_lifetime
        .clone()
        .max_lifetime(Duration::from_secs(u64::from(u32::MAX)));
    let read = |name: &str| -> Result<Vec<u8>, Box<dyn Error>> {
        let path: PathBuf = dir.join(format!("{name}.json"));
        Ok(std::fs::read(path)?)
    };
    let valid = ProfileBundle::from_bytes(&read("two-chrome")?, &source)?;
    let trusted = source.clone().min_serial(valid.serial());
    let refused: [(&str, &BundleSource, fn(&ProfileError) -> bool); 4] = [
        ("tampered", &source, |error| {
            matches!(error, ProfileError::Signature)
        }),
        ("rolled-back", &trusted, |error| {
            matches!(error, ProfileError::Rollback { .. })
        }),
        ("future-serial", &source, |error| {
            matches!(error, ProfileError::FutureSerial { .. })
        }),
        ("expired", &source, |error| {
            matches!(error, ProfileError::Expired { .. })
        }),
    ];
    for (name, from, expected) in refused {
        match ProfileBundle::from_bytes(&read(name)?, from) {
            Err(error) if expected(&error) => println!("{name}: {error}"),
            other => return Err(format!("{name} was not refused as expected: {other:?}").into()),
        }
    }
    for name in [
        "tls10",
        "weak-cipher",
        "borrowed-cipher",
        "header-injection",
    ] {
        let bundle = ProfileBundle::from_bytes(&read(name)?, &source)?;
        match bundle.skipped() {
            [skip]
                if matches!(skip.error, ProfileError::Policy { .. })
                    && bundle.profiles().get(&skip.browser, skip.version).is_none() =>
            {
                println!("{name}: {}", skip.error);
            }
            other => {
                return Err(format!("{name} profile was not refused by policy: {other:?}").into());
            }
        }
    }
    let at_floor = ProfileBundle::from_bytes(&read("tls12")?, &source)?;
    if !at_floor.skipped().is_empty() {
        return Err(format!(
            "a profile at the TLS floor was refused: {:?}",
            at_floor.skipped()
        )
        .into());
    }
    println!("tls12: a profile at the floor loads");
    match ProfileBundle::from_bytes(&read("two-chrome")?, &fixed_lifetime) {
        Err(error @ ProfileError::Lifetime { .. }) => println!("lifetime bound: {error}"),
        other => return Err(format!("a bundle longer than the lifetime loaded: {other:?}").into()),
    }
    println!(
        "fixtures: 5 rejected as expected (tampered, rolled-back, future-serial, expired, tls10), plus 3 policy refusals"
    );
    Ok(())
}
