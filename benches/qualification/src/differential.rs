use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper_util::rt::TokioIo;
use leyline::{Redirect, Session};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::runtime::Builder;

use crate::number;

const WORKERS: usize = 32;
const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
enum Client {
    Leyline,
    Hyper,
}

type Reply = Result<(u16, Vec<u8>), String>;

async fn leyline_reply(url: &str) -> Reply {
    let session = Session::builder()
        .redirect(Redirect::none())
        .retry(0)
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let response = session
        .get(url)
        .decode(false)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    Ok((status, body.to_vec()))
}

async fn hyper_reply(port: u16, path: &str) -> Reply {
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| e.to_string())?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|e| e.to_string())?;
    let driver = tokio::spawn(connection);
    let request = hyper::Request::get(path)
        .header("host", "127.0.0.1")
        .body(Empty::<Bytes>::new())
        .map_err(|e| e.to_string())?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|e| e.to_string())?
        .to_bytes();
    driver.abort();
    Ok((status, body.to_vec()))
}

async fn reply(client: Client, port: u16, case: usize) -> Reply {
    let path = format!("/c/{case}");
    let exchange = async {
        match client {
            Client::Leyline => leyline_reply(&format!("http://127.0.0.1:{port}{path}")).await,
            Client::Hyper => hyper_reply(port, &path).await,
        }
    };
    tokio::time::timeout(TIMEOUT, exchange)
        .await
        .unwrap_or_else(|_| Err("timeout".into()))
}

fn line(case: usize, reply: Reply) -> Value {
    match reply {
        Ok((status, body)) => {
            let hex: String = body.iter().map(|b| format!("{b:02x}")).collect();
            json!({"case": case, "status": status, "body": hex})
        }
        Err(error) => json!({"case": case, "error": error}),
    }
}

pub fn run(args: &[String]) -> ExitCode {
    let client = match number::<String>(args, "--client", String::new()).as_str() {
        "leyline" => Client::Leyline,
        "hyper" => Client::Hyper,
        _ => {
            eprintln!("usage: qualify differential --client leyline|hyper --port N --cases N");
            return ExitCode::from(2);
        }
    };
    let port: u16 = number(args, "--port", 19000);
    let cases: usize = number(args, "--cases", 0);
    let next = Arc::new(AtomicUsize::new(0));
    let lines = Arc::new(Mutex::new(Vec::with_capacity(cases)));
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let mut workers = Vec::with_capacity(WORKERS);
        for _ in 0..WORKERS {
            let (next, lines) = (next.clone(), lines.clone());
            workers.push(tokio::spawn(async move {
                loop {
                    let case = next.fetch_add(1, Ordering::Relaxed);
                    if case >= cases {
                        return;
                    }
                    let outcome = line(case, reply(client, port, case).await);
                    lines.lock().unwrap().push(outcome);
                }
            }));
        }
        for worker in workers {
            worker.await.expect("worker");
        }
    });
    for outcome in lines.lock().unwrap().iter() {
        println!("{outcome}");
    }
    ExitCode::SUCCESS
}
