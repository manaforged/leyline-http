use leyline::{Browser, Family, Platform, ProtocolPolicy, Session};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{PEET_URL, Result, ensure, json_str, normalize_akamai};

const H3_GET_HOST: &str = "cloudflare-quic.com";

pub(super) async fn exact_fingerprint(session: Session, browser: Browser) -> Result<String> {
    let r = session.get(PEET_URL).await?;
    ensure(r.status() == 200, format!("status={}", r.status()))?;
    let v: Value = r.json().await?;

    let ja4 = json_str(&v["tls"]["ja4"], "tls.ja4")?;
    let h2 = normalize_akamai(json_str(
        &v["http2"]["akamai_fingerprint"],
        "http2.akamai_fingerprint",
    )?);

    let profile = browser.profile();
    let expected_ja4 = profile
        .expected_ja4()
        .ok_or_else(|| format!("{browser} profile missing expected JA4"))?;
    let expected_h2 = normalize_akamai(
        profile
            .expected_h2_fingerprint()
            .ok_or_else(|| format!("{browser} profile missing expected H2"))?,
    );

    ensure(
        ja4 == expected_ja4,
        format!("{browser} JA4 mismatch: got {ja4}, expected {expected_ja4}"),
    )?;
    ensure(
        h2 == expected_h2,
        format!("{browser} H2 mismatch: got {h2}, expected {expected_h2}"),
    )?;

    Ok(format!("JA4={ja4} H2={h2}"))
}

pub(super) async fn h1_wire_shape() -> Result<String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut req = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            let n = socket.read(&mut tmp).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("client closed before request headers".to_string());
            }
            req.extend_from_slice(&tmp[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .map_err(|e| e.to_string())?;

        Ok::<_, String>(String::from_utf8_lossy(&req).into_owned())
    });

    let session = Session::browser(Browser::default());
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/wire?q=1"))
        .header("x-proof", "smoke")
        .send()
        .await?;
    ensure(resp.status() == 200, format!("status={}", resp.status()))?;

    let text = server
        .await
        .map_err(|e| format!("h1 server task failed: {e}"))??;

    ensure(
        text.starts_with("GET /wire?q=1 HTTP/1.1\r\n"),
        format!("bad request line: {text}"),
    )?;
    for needle in [
        "\r\nHost: ",
        "\r\nUser-Agent: ",
        "\r\nAccept: ",
        "\r\nAccept-Encoding: ",
        "\r\nAccept-Language: ",
        "\r\nConnection: keep-alive\r\n",
        "\r\nx-proof: smoke\r\n",
    ] {
        ensure(text.contains(needle), format!("missing {needle:?}: {text}"))?;
    }

    Ok(format!("{} request bytes", text.len()))
}

pub(super) async fn h3_get(browser: Browser) -> Result<String> {
    let session = Session::builder()
        .browser(browser)
        .protocol(ProtocolPolicy::Http3)
        .build()?;
    let url = format!("https://{H3_GET_HOST}/");
    let resp = session
        .request(http::Method::GET, url)
        .header("accept", "text/html,application/xhtml+xml")
        .send()
        .await?;

    let status = resp.status();
    let body = resp.bytes().await?;
    ensure(status == 200, format!("status={status}"))?;
    ensure(!body.is_empty(), "empty H3 body")?;
    Ok(format!("status={status} body={}B", body.len()))
}

pub(super) fn firefox() -> Result<Session> {
    Ok(Session::builder()
        .browser(Browser::latest(Family::Firefox))
        .platform(Platform::Windows)
        .build()?)
}

pub(super) fn safari() -> Result<Session> {
    Ok(Session::builder()
        .browser(Browser::Safari26)
        .platform(Platform::MacOS)
        .build()?)
}
