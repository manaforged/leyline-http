mod core_support {
    pub mod forward;
    pub mod wait;
}
#[cfg(feature = "bench-internals")]
mod h2_support;
#[cfg(feature = "http3")]
mod h3_support;
mod http_support {
    pub mod httpbin_lite;
}
mod tls_support;

mod accounts_and_forms;
mod body_cap;
mod cfnetwork_live;
mod claim_guard;
mod connect_timeout;
mod content_encoding;
mod cookie_lifetime;
mod core_digest;
#[cfg(feature = "multipart")]
mod core_multipart;
mod core_retry;
mod core_streaming;
mod core_wire_fidelity;
mod crawl_identity;
mod crawling;
mod credentials_hidden;
mod deadlines_and_lifecycle;
mod devices;
mod digest_redirects;
mod ergonomics;
mod error_body_budget;
#[cfg(feature = "bench-internals")]
mod fingerprint_conformance;
mod forms_and_relay;
mod future_size;
mod h1_error_body;
mod h1_failures;
#[cfg(feature = "bench-internals")]
mod h2_backpressure;
#[cfg(feature = "bench-internals")]
mod h2_body_cap;
#[cfg(feature = "bench-internals")]
mod h2_cancel_safety;
#[cfg(feature = "bench-internals")]
mod h2_connect_method;
#[cfg(feature = "bench-internals")]
mod h2_continuation_timeout;
#[cfg(feature = "bench-internals")]
mod h2_error_prefix;
#[cfg(all(feature = "bench-internals", feature = "websocket"))]
mod h2_extended_connect;
#[cfg(feature = "bench-internals")]
mod h2_fallback_upgrade;
#[cfg(feature = "bench-internals")]
mod h2_flow_control_e2e;
#[cfg(feature = "bench-internals")]
mod h2_flow_overflow;
#[cfg(feature = "bench-internals")]
mod h2_frame_roundtrip;
#[cfg(feature = "bench-internals")]
mod h2_hostile_server;
#[cfg(feature = "bench-internals")]
mod h2_informational_e2e;
#[cfg(feature = "bench-internals")]
mod h2_message_length;
#[cfg(feature = "bench-internals")]
mod h2_multiplex;
#[cfg(feature = "bench-internals")]
mod h2_output_budget;
#[cfg(feature = "bench-internals")]
mod h2_pool_stall;
#[cfg(feature = "bench-internals")]
mod h2_recv_window;
#[cfg(feature = "bench-internals")]
mod h2_refused_retry;
#[cfg(feature = "bench-internals")]
mod h2_resend;
#[cfg(feature = "bench-internals")]
mod h2_rst_flood;
#[cfg(feature = "bench-internals")]
mod h2_rst_flood_e2e;
#[cfg(feature = "bench-internals")]
mod h2_shutdown_streaming_e2e;
#[cfg(feature = "bench-internals")]
mod h2_stream_delivery;
#[cfg(feature = "bench-internals")]
mod h2_stream_lifecycle;
#[cfg(feature = "bench-internals")]
mod h2_stream_state;
#[cfg(feature = "bench-internals")]
mod h2_write_batching;
mod h3_body_cap;
mod h3_error_prefix;
mod h3_request_rejected;
mod h3_stream_errors;
mod h3_stream_limit;
mod h3_timeout;
mod h3_timing;
mod host_limit_hops;
#[cfg(feature = "bench-internals")]
mod hpack_encoding;
mod http_semantics;
#[cfg(feature = "bench-internals")]
mod https_proxy;
mod identity_switch;
mod limit_settings;
mod limits_and_state;
mod pages_and_limits;
mod parity_builder;
#[cfg(feature = "bench-internals")]
mod pool_h1_framing;
#[cfg(feature = "bench-internals")]
mod pool_h1_injection;
#[cfg(feature = "bench-internals")]
mod pool_h1_keepalive;
#[cfg(feature = "bench-internals")]
mod pool_reconnect_storm;
#[cfg(feature = "bench-internals")]
mod pool_stats;
mod pq_key_shares;
#[cfg(feature = "bench-internals")]
mod profile_builtin;
#[cfg(feature = "bench-internals")]
mod profile_validation;
mod proxy_identity;
mod proxy_rebind;
mod proxy_rules;
mod public_debug;
mod read_until;
mod recovery;
mod redirect_host;
mod referers_hints_and_proxies;
mod response_header_timeout;
mod responses_and_errors;
mod retry_rules;
mod retry_unsent_redirect;
mod samesite_cookies;
mod server_chosen_urls;
mod session_clone;
mod session_lifecycle;
mod sessions_and_devices;
#[cfg(feature = "http3")]
mod smoke;
mod state_files;
mod status_and_retry;
mod timeouts;
mod tls_client_hello_order;
mod tls_floor;
#[cfg(feature = "bench-internals")]
mod tls_happy_eyeballs;
#[cfg(all(feature = "bench-internals", feature = "http3", feature = "websocket"))]
mod tls_peet;
#[cfg(feature = "bench-internals")]
mod tls_pinning_hostname;
mod tls_system_trust_live;
mod trace_unwind;
mod websocket_messages;
#[cfg(feature = "websocket")]
mod websocket_subprotocol;
mod wire_headers;
