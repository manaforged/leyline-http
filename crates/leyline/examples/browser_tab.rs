use leyline::html::Form;
use leyline::{Browser, Session};
use serde_json::Value;

const HOME: &str = "https://shop.example/";
const CART_API: &str = "/api/cart";
const LOGIN: &str = "/login";
const LOGIN_FORM: &str = "login";
const FIELDS: [(&str, &str); 2] = [("email", "user@example.com"), ("password", "secret")];

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::default())
        .languages(["de-DE", "de", "en"])
        .build()?;
    let tab = session.tab();

    let home = tab.open(HOME).await?;
    println!("home: {} {}", home.status(), home.url());

    let cart: Value = tab.xhr(CART_API).send().await?.json().await?;
    println!("cart: {cart}");

    let page = tab.follow(LOGIN).await?.text().await?;
    let Some(mut form) = Form::find(&page, LOGIN_FORM) else {
        println!("no form with id or name {LOGIN_FORM:?}");
        return Ok(());
    };
    for (name, value) in FIELDS {
        form.set(name, value);
    }

    let done = tab.submit_form(&form).await?;
    println!("after submit: {} at {:?}", done.status(), tab.current());
    Ok(())
}
