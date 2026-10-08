use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use leyline_bssl::ssl::SslAcceptor;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::task::{JoinHandle, JoinSet};

use crate::tls::trust::TlsTrustConfig;

mod cert;
mod conn;
mod types;

use types::Recorder;
pub use types::{Handler, RecordedRequest, TestResponse, queue};

const BIND_ADDR: &str = "127.0.0.1:0";
const HTTP_SCHEME: &str = "http";
const HTTPS_SCHEME: &str = "https";

pub struct TestServer {
    addr: SocketAddr,
    scheme: &'static str,
    ca_der: Option<Vec<u8>>,
    recorder: Recorder,
    incoming: Mutex<UnboundedReceiver<RecordedRequest>>,
    task: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for TestServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestServer")
            .field("addr", &self.addr)
            .field("scheme", &self.scheme)
            .finish_non_exhaustive()
    }
}

impl TestServer {
    pub async fn http<F>(handler: F) -> io::Result<Self>
    where
        F: Fn(&RecordedRequest) -> TestResponse + Send + Sync + 'static,
    {
        Self::start(TcpListener::bind(BIND_ADDR).await?, Arc::new(handler), None)
    }

    pub fn http_on<F>(listener: std::net::TcpListener, handler: F) -> io::Result<Self>
    where
        F: Fn(&RecordedRequest) -> TestResponse + Send + Sync + 'static,
    {
        listener.set_nonblocking(true)?;
        Self::start(TcpListener::from_std(listener)?, Arc::new(handler), None)
    }

    pub async fn https<F>(handler: F) -> io::Result<Self>
    where
        F: Fn(&RecordedRequest) -> TestResponse + Send + Sync + 'static,
    {
        Self::start(
            TcpListener::bind(BIND_ADDR).await?,
            Arc::new(handler),
            Some(cert::build()?),
        )
    }

    fn start(
        listener: TcpListener,
        handler: Handler,
        tls: Option<cert::TestTls>,
    ) -> io::Result<Self> {
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
        let task = tokio::spawn(run(listener, acceptor, handler, recorder.clone()));
        Ok(Self {
            addr,
            scheme,
            ca_der,
            recorder,
            incoming: Mutex::new(receiver),
            task: Some(task),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self, path: &str) -> String {
        let slash = if path.starts_with('/') { "" } else { "/" };
        format!("{}://{}{slash}{path}", self.scheme, self.addr)
    }

    pub fn trust(&self) -> TlsTrustConfig {
        let base = TlsTrustConfig::new().system_roots(false).env_roots(false);
        match &self.ca_der {
            Some(der) => base.add_ca_der(der.clone()),
            None => base,
        }
    }

    pub fn ca_der(&self) -> Option<&[u8]> {
        self.ca_der.as_deref()
    }

    pub async fn requests(&self) -> Vec<RecordedRequest> {
        self.recorder.snapshot()
    }

    pub async fn next_request(&self) -> Option<RecordedRequest> {
        self.incoming.lock().await.recv().await
    }

    pub async fn shutdown(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            drop(task.await);
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
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
