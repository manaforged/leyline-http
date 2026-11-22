use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};

use leyline_bssl::ssl::SslAcceptor;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::task::{JoinHandle, JoinSet};

use crate::tls::trust::Tls;

mod cert;
mod conn;
mod types;

use types::Handler;
use types::Recorder;
pub use types::{Request, Response, queue};

const BIND_ADDR: &str = "127.0.0.1:0";
const HTTP_SCHEME: &str = "http";
const HTTPS_SCHEME: &str = "https";
const BACKGROUND_THREAD: &str = "leyline-test-server";

static BACKGROUND: OnceLock<Result<Handle, io::ErrorKind>> = OnceLock::new();

/// A local HTTP or HTTPS server for tests that answers each request with a handler and records it.
pub struct Server {
    addr: SocketAddr,
    scheme: &'static str,
    ca_der: Option<Vec<u8>>,
    recorder: Recorder,
    incoming: Mutex<UnboundedReceiver<Request>>,
    task: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("addr", &self.addr)
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Starts a plain HTTP server on a free loopback port.
    pub async fn http<F>(handler: F) -> io::Result<Self>
    where
        F: Fn(&Request) -> Response + Send + Sync + 'static,
    {
        Self::start(
            std::net::TcpListener::bind(BIND_ADDR)?,
            Arc::new(handler),
            None,
        )
    }

    /// Starts a plain HTTP server on a listener the caller already bound; it needs no runtime, so synchronous code can call it.
    pub fn http_on<F>(listener: std::net::TcpListener, handler: F) -> io::Result<Self>
    where
        F: Fn(&Request) -> Response + Send + Sync + 'static,
    {
        Self::start(listener, Arc::new(handler), None)
    }

    /// Starts an HTTPS server on a free loopback port with a fresh self-signed certificate.
    pub async fn https<F>(handler: F) -> io::Result<Self>
    where
        F: Fn(&Request) -> Response + Send + Sync + 'static,
    {
        Self::https_on(std::net::TcpListener::bind(BIND_ADDR)?, handler)
    }

    /// Starts an HTTPS server with a fresh self-signed certificate on a listener the caller already bound; it needs no runtime, so synchronous code can call it.
    pub fn https_on<F>(listener: std::net::TcpListener, handler: F) -> io::Result<Self>
    where
        F: Fn(&Request) -> Response + Send + Sync + 'static,
    {
        let ip = listener.local_addr()?.ip();
        Self::start(listener, Arc::new(handler), Some(cert::build(Some(ip))?))
    }

    fn start(
        listener: std::net::TcpListener,
        handler: Handler,
        tls: Option<cert::TestTls>,
    ) -> io::Result<Self> {
        listener.set_nonblocking(true)?;
        let runtime = runtime()?;
        let listener = {
            let _entered = runtime.enter();
            TcpListener::from_std(listener)?
        };
        let addr = listener.local_addr()?;
        let (sender, receiver) = unbounded_channel();
        let recorder = Recorder {
            log: Arc::default(),
            sender,
        };
        let scheme = if tls.is_some() {
            HTTPS_SCHEME
        } else {
            HTTP_SCHEME
        };
        let ca_der = tls.as_ref().map(|tls| tls.ca_der.clone());
        let acceptor = tls.map(|tls| Arc::new(tls.acceptor));
        let task = runtime.spawn(run(listener, acceptor, handler, recorder.clone()));
        Ok(Self {
            addr,
            scheme,
            ca_der,
            recorder,
            incoming: Mutex::new(receiver),
            task: Some(task),
        })
    }

    /// Returns the address the server listens on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Returns the absolute `http://` or `https://` URL of `path` on this server, where `path` begins with `/` and may carry a query.
    pub fn url(&self, path: &str) -> String {
        let slash = if path.starts_with('/') { "" } else { "/" };
        format!("{}://{}{slash}{path}", self.scheme, self.addr)
    }

    /// Returns a ready trust configuration for a client, passed to `SessionBuilder::tls`, that trusts this server's certificate and no other root.
    pub fn tls(&self) -> Tls {
        let base = Tls::new().system_roots(false).env_roots(false);
        match &self.ca_der {
            Some(der) => base.ca_der(der.clone()),
            None => base,
        }
    }

    /// Returns the DER bytes of the certificate an HTTPS server presents, or `None` for a plain-HTTP server.
    pub fn ca_der(&self) -> Option<&[u8]> {
        self.ca_der.as_deref()
    }

    /// Returns a copy of every request received so far, in arrival order, without removing any; `next_request` does not change this list.
    pub async fn requests(&self) -> Vec<Request> {
        self.recorder.snapshot()
    }

    /// Returns each request once, in arrival order, waiting when none is unread, or `None` after the server stops; `requests` still lists it.
    pub async fn next_request(&self) -> Option<Request> {
        self.incoming.lock().await.recv().await
    }

    /// Stops the server and waits for its task to end.
    pub async fn shutdown(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            drop(task.await);
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn runtime() -> io::Result<Handle> {
    match Handle::try_current() {
        Ok(current) if !crate::blocking::active() => Ok(current),
        _ => BACKGROUND
            .get_or_init(background)
            .clone()
            .map_err(io::Error::from),
    }
}

fn background() -> Result<Handle, io::ErrorKind> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.kind())?;
    let handle = runtime.handle().clone();
    std::thread::Builder::new()
        .name(BACKGROUND_THREAD.to_owned())
        .spawn(move || runtime.block_on(std::future::pending::<()>()))
        .map_err(|error| error.kind())?;
    Ok(handle)
}

async fn run(
    listener: TcpListener,
    acceptor: Option<Arc<SslAcceptor>>,
    handler: Handler,
    recorder: Recorder,
) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((tcp, _)) = accepted else { return };
                connections.spawn(connection(tcp, acceptor.clone(), Arc::clone(&handler), recorder.clone()));
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

async fn connection(
    tcp: TcpStream,
    acceptor: Option<Arc<SslAcceptor>>,
    handler: Handler,
    recorder: Recorder,
) {
    if let Err(e) = tcp.set_nodelay(true) {
        tracing::warn!(error = %e, "test server could not set TCP_NODELAY");
    }
    match acceptor {
        Some(acceptor) => {
            if let Ok(stream) = leyline_bssl_tokio::accept(&acceptor, tcp).await {
                conn::serve(stream, handler, recorder).await;
            }
        }
        None => conn::serve(tcp, handler, recorder).await,
    }
}
