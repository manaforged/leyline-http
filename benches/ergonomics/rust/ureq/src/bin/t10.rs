use ureq::{Agent, Error, ResponseExt};

fn main() -> Result<(), Error> {
    let agent: Agent = Agent::config_builder().max_redirects(5).build().into();
    let response = agent.get("http://127.0.0.1:18743/t10/start").call()?;
    println!("{}", response.get_uri());
    match agent.get("http://127.0.0.1:18743/t10/loop/0").call() {
        Err(Error::TooManyRedirects) => println!("capped"),
        Err(error) => return Err(error),
        Ok(_) => println!("uncapped"),
    }
    Ok(())
}
