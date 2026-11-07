/// How far a failed request got, which decides whether it is sent again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The connection closed before the request reached the peer.
    Unsent,
    /// The peer refused the request without processing it.
    Refused,
    /// The connection failed after the request may have reached the peer.
    Connection,
    /// The request failed on its own.
    Request,
}

impl Failure {
    /// Returns whether the pool stops handing out the `version` connection the request used.
    pub(crate) fn retires_connection(self, version: http::Version) -> bool {
        match self {
            Self::Unsent | Self::Connection => true,
            Self::Refused => version == http::Version::HTTP_2,
            Self::Request => false,
        }
    }
}
