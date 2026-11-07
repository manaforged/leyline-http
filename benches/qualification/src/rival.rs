use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use pingora_core::connectors::ConnectorOptions;
use pingora_core::connectors::http::Connector;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_http::RequestHeader;

use crate::plan::{ID_HEADER, SIZE_HEADER};

#[derive(Clone)]
pub struct Rival {
    connector: Arc<Connector>,
    deadline: Duration,
}

pub struct Answer {
    pub status: u16,
    pub body: BytesMut,
}

impl Rival {
    pub fn new(pool: usize, deadline: Duration) -> Self {
        let connector = Arc::new(Connector::new(Some(ConnectorOptions::new(pool.max(1)))));
        Self { connector, deadline }
    }

    pub async fn get(&self, ip: std::net::Ipv4Addr, port: u16, id: u64, size: usize) -> Result<Answer, String> {
        tokio::time::timeout(self.deadline, self.exchange(ip, port, id, size))
            .await
            .map_err(|_| "deadline".to_owned())?
    }

    async fn exchange(&self, ip: std::net::Ipv4Addr, port: u16, id: u64, size: usize) -> Result<Answer, String> {
        let peer = HttpPeer::new((ip, port), false, String::new());
        let mut request = RequestHeader::build_no_case("GET", b"/", Some(3)).map_err(|e| e.to_string())?;
        let set = |request: &mut RequestHeader, name: &'static str, value: String| {
            request.insert_header(name, value).map_err(|e| e.to_string())
        };
        set(&mut request, "host", format!("{ip}:{port}"))?;
        set(&mut request, ID_HEADER, id.to_string())?;
        set(&mut request, SIZE_HEADER, size.to_string())?;
        let (mut session, _) = self.connector.get_http_session(&peer).await.map_err(|e| e.to_string())?;
        session.write_request_header(Box::new(request)).await.map_err(|e| e.to_string())?;
        session.read_response_header().await.map_err(|e| e.to_string())?;
        let status = session.response_header().ok_or("no response header")?.status.as_u16();
        let mut body = BytesMut::new();
        while let Some(chunk) = session.read_response_body().await.map_err(|e| e.to_string())? {
            body.extend_from_slice(&chunk);
        }
        self.connector.release_http_session(session, &peer, None).await;
        Ok(Answer { status, body })
    }
}
