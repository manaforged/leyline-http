use std::path::Path;

use leyline::html::Form;
use leyline::{Browser, Session, SessionBuilder};
use serde::{Deserialize, Serialize};

const ACCOUNT: &str = "account.json";
const PROXY: &str = "http://user:pass@proxy.example:8080";
const LANGUAGES: [&str; 2] = ["en-GB", "fr"];
const HOME: &str = "https://account.example/";
const LOGIN: &str = "https://account.example/login";
const LOGIN_FORM: &str = "login";
const EMAIL_FIELD: &str = "email";
const PASSWORD_FIELD: &str = "password";
const EMAIL_ENV: &str = "ACCOUNT_EMAIL";
const PASSWORD_ENV: &str = "ACCOUNT_PASSWORD";

#[derive(Serialize)]
struct AccountOut<'a> {
    session: &'a Session,
    email: &'a str,
}

#[derive(Deserialize)]
struct AccountIn {
    session: SessionBuilder,
    email: String,
}

async fn log_in(tab: &leyline::Tab, email: &str) -> leyline::Result<()> {
    let page = tab.open(LOGIN).await?.text().await?;
    let Some(mut form) = Form::find(&page, LOGIN_FORM) else {
        println!("no login form: the session is already logged in");
        return Ok(());
    };
    let password = std::env::var(PASSWORD_ENV).unwrap_or_default();
    form.set(EMAIL_FIELD, email.to_owned())
        .set(PASSWORD_FIELD, password);
    let done = tab.submit_form(&form).await?;
    println!("login: {} at {}", done.status(), done.url());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (session, email) = if Path::new(ACCOUNT).exists() {
        let account: AccountIn = serde_json::from_slice(&std::fs::read(ACCOUNT)?)?;
        (account.session.proxy(PROXY).build()?, account.email)
    } else {
        let session = Session::builder()
            .browser(Browser::Chrome154)
            .languages(LANGUAGES)
            .proxy(PROXY)
            .build()?;
        let email = std::env::var(EMAIL_ENV).unwrap_or_default();
        log_in(&session.tab(), &email).await?;
        (session, email)
    };
    println!("account {email}");

    let resp = session.tab().follow(HOME).await?;
    println!("status {} at {}", resp.status(), resp.url());

    let saved = serde_json::to_vec_pretty(&AccountOut {
        session: &session,
        email: &email,
    })?;
    std::fs::write(ACCOUNT, saved)?;
    Ok(())
}
