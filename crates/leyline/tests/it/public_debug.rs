use std::fmt::Debug;

fn debug<T: Debug>() {}

#[test]
fn every_public_type_implements_debug() {
    debug::<leyline::SessionBuilder>();
    debug::<leyline::RequestBuilder>();
    debug::<leyline::profile::ProfileRegistry>();
    debug::<leyline::trace::Dns<'static>>();
    debug::<leyline::trace::Connect<'static>>();
    debug::<leyline::trace::Tls<'static>>();
    debug::<leyline::trace::Sent<'static>>();
    debug::<leyline::trace::Head<'static>>();
    debug::<leyline::trace::Done<'static>>();
    debug::<leyline::audit::Ja3Input<'static>>();
    debug::<leyline::audit::Ja4Input<'static>>();
    debug::<leyline::audit::Ja4hInput<'static>>();
    #[cfg(feature = "multipart")]
    {
        debug::<leyline::multipart::Form>();
        debug::<leyline::multipart::Part>();
    }
    #[cfg(feature = "websocket")]
    {
        debug::<leyline::WebSocketBuilder>();
        debug::<leyline::WsConnection>();
        debug::<leyline::WsSink>();
        debug::<leyline::WsStream>();
    }
    #[cfg(feature = "tower")]
    debug::<leyline::LeylineService>();
}
