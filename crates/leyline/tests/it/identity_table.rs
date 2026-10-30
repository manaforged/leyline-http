use leyline::testing;
use leyline::{Browser, Platform, Session, SessionBuilder};

fn browser_session(browser: Browser, platform: Option<Platform>) -> leyline::Result<Session> {
    let builder = Session::builder().audit(true).browser(browser);
    match platform {
        Some(platform) => builder.platform(platform).build(),
        None => builder.build(),
    }
}

fn profile_session(profile: &str, platform: Platform) -> leyline::Result<Session> {
    Session::builder()
        .audit(true)
        .profile(profile)
        .platform(platform)
        .build()
}

fn user_agent_session(browser: Browser, agent: &str) -> leyline::Result<Session> {
    Session::builder()
        .audit(true)
        .browser(browser)
        .platform(Platform::Windows)
        .headers([("user-agent", agent)])
        .build()
}

fn plain_session(platform: Platform) -> leyline::Result<Session> {
    Session::builder().audit(true).platform(platform).build()
}

fn platforms() -> impl Iterator<Item = Platform> {
    Platform::all()
        .iter()
        .copied()
        .filter(|platform| *platform != Platform::Host)
}

async fn row(server: &testing::Server, label: String, session: leyline::Result<Session>) -> String {
    let session = match session {
        Ok(session) => session,
        Err(err) => return format!("{label} => error {:?}\n", err.kind()),
    };
    session.get(server.url("/")).send().await.unwrap();
    let request = server.next_request().await.unwrap();
    let headers = request
        .headers
        .iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("host"))
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join(" ; ");
    format!(
        "{label} => {} | {:?} | {session:?} | {headers}\n",
        session.profile_id().as_deref().unwrap_or("-"),
        session.platform(),
    )
}

#[tokio::test]
async fn every_identity_keeps_its_fingerprint() {
    let server = testing::Server::http(|_| testing::Response::new(200))
        .await
        .unwrap();
    let mut table = String::new();
    for &browser in Browser::all() {
        let label = format!("{browser:?} native -");
        table += &row(&server, label, browser_session(browser, None)).await;
        for platform in platforms() {
            let label = format!("{browser:?} {platform:?} -");
            let session = browser_session(browser, Some(platform));
            table += &row(&server, label, session).await;
        }
    }
    let chrome = Browser::chrome();
    let exported = Session::from(chrome).profile().unwrap().to_owned();
    for platform in platforms() {
        let label = format!("custom {platform:?}");
        table += &row(&server, label, profile_session(&exported, platform)).await;
        let label = format!("plain {platform:?}");
        table += &row(&server, label, plain_session(platform)).await;
    }
    let label = "user-agent override".to_owned();
    table += &row(&server, label, user_agent_session(chrome, "probe/1.0")).await;
    if let Ok(path) = std::env::var("IDENTITY_TABLE_OUT") {
        std::fs::write(path, &table).unwrap();
    }
    let mut changed = String::new();
    for (now, then) in table
        .lines()
        .zip(include_str!("identity_table.txt").lines())
    {
        if now != then {
            changed += &format!("- {then}\n+ {now}\n");
        }
    }
    assert_eq!(
        table.lines().count(),
        include_str!("identity_table.txt").lines().count()
    );
    assert!(changed.is_empty(), "{changed}");
}

#[test]
fn a_device_saved_before_the_identity_fold_opens_the_same_profile() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let expected = std::fs::read_to_string(dir.join("device-58f586f6.profile-id")).unwrap();
    let builder = Session::load(dir.join("device-58f586f6.json")).unwrap();
    let session = builder.build().unwrap();
    assert_eq!(session.profile_id().as_deref(), Some(expected.trim()));
}

#[test]
fn a_profile_identity_with_languages_survives_a_save() {
    let chrome = Browser::chrome();
    let toml = Session::from(chrome).profile().unwrap().to_owned();
    let direct = Session::builder()
        .profile(&toml)
        .platform(Platform::MacOS)
        .languages(["de-DE", "de"])
        .build()
        .unwrap();
    let saved = serde_json::to_string(&direct).unwrap();
    let reloaded = serde_json::from_str::<SessionBuilder>(&saved)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(reloaded.profile_id(), direct.profile_id());
    assert_eq!(reloaded.platform(), Platform::MacOS);
    assert_eq!(
        reloaded.languages(),
        Some(&["de-DE".to_owned(), "de".to_owned()][..])
    );
}

#[test]
fn a_brand_on_a_chromium_major_is_found_without_bench_internals() {
    let chrome = Browser::Chrome150;
    let opera = Browser::all()
        .iter()
        .copied()
        .find(|browser| {
            browser.id().starts_with("opera-")
                && browser.chromium_major() == chrome.chromium_major()
        })
        .expect("an Opera build on Chromium 150");
    assert_eq!(opera.chromium_major(), Some(150));
    assert_eq!(Browser::Firefox151.chromium_major(), None);
}
