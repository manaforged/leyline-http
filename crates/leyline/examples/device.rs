use std::path::Path;
use std::time::Duration;

use leyline::html::Form;
use leyline::{Browser, Device, ProxyConfig, ProxyUrl, Session};
use serde_json::Value;

const DEVICE: &str = "device.json";
const JAR: &str = "cookies.json";
const PROXY: &str = "http://user:pass@proxy.example:8080";
const LANGUAGES: [&str; 2] = ["en-GB", "fr"];
const HOME: &str = "https://account.example/";
const LOGIN: &str = "https://account.example/login";
const LOGIN_FORM: &str = "login";
const EMAIL_FIELD: &str = "email";
const PASSWORD_FIELD: &str = "password";
const EMAIL_ENV: &str = "ACCOUNT_EMAIL";
const PASSWORD_ENV: &str = "ACCOUNT_PASSWORD";
const EMAIL_KEY: &str = "email";
const DEBOUNCE: Duration = Duration::from_secs(2);

fn create() -> leyline::Result<(Session, Device)> {
    let proxy = ProxyUrl::parse(PROXY)?;
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .languages(LANGUAGES)
        .proxy(ProxyConfig::from(proxy.clone()).env(false))
        .build()?;
    let mut device = Device::capture(&session, Some(proxy));
    device.pin_profile(&session)?;
    device.strict = true;
    device.jar_path = Some(JAR.into());
    if let Ok(email) = std::env::var(EMAIL_ENV) {
        device.app.insert(EMAIL_KEY.into(), Value::String(email));
    }
    Ok((session, device))
}

fn restore() -> leyline::Result<(Session, Device)> {
    let device = Device::load_from(DEVICE)?;
    let session = device.open()?;
    Ok((session, device))
}

async fn log_in(tab: &leyline::Tab, device: &Device) -> leyline::Result<()> {
    let page = tab.open(LOGIN).await?.text().await?;
    let Some(mut form) = Form::find(&page, LOGIN_FORM) else {
        println!("no login form: the session is already logged in");
        return Ok(());
    };
    let email = device
        .app
        .get(EMAIL_KEY)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let password = std::env::var(PASSWORD_ENV).unwrap_or_default();
    form.set(EMAIL_FIELD, email).set(PASSWORD_FIELD, password);
    let done = tab.submit_form(&form).await?;
    println!("login: {} at {}", done.status(), done.url());
    Ok(())
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let (session, device) = if Path::new(DEVICE).exists() {
        restore()?
    } else {
        create()?
    };
    let tab = device.tab(&session);
    let autosave = device.autosave(&session, DEVICE, DEBOUNCE);
    autosave.track(&tab);

    if device.page.is_none() {
        log_in(&tab, &device).await?;
    }

    let resp = tab.follow(HOME).await?;
    println!("status {} at {}", resp.status(), resp.url());

    autosave.shutdown().await?;
    Ok(())
}
