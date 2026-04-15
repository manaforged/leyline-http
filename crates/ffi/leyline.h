/* Leyline C FFI — Browser-accurate TLS fingerprinting.
 *
 * Usage:
 *   LeylineSession *s = leyline_session_chrome();
 *   LeylineResponse *r = leyline_session_get(s, "https://example.com");
 *   printf("status: %d\n", leyline_response_status(r));
 *   char *body = leyline_response_text(r);
 *   printf("%s\n", body);
 *   leyline_free_string(body);
 *   leyline_response_free(r);
 *   leyline_session_free(s);
 */

#ifndef LEYLINE_H
#define LEYLINE_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handles */
typedef struct LeylineSession LeylineSession;
typedef struct LeylineResponse LeylineResponse;

/* ── Error handling ─────────────────────────────────────────── */

/* Get last error message, or NULL if none. Free with leyline_free_string. */
char *leyline_last_error(void);

/* Free a string returned by any leyline_* function. */
void leyline_free_string(char *s);

/* ── Session ────────────────────────────────────────────────── */

/* Create sessions with browser defaults. Returns NULL on error. */
LeylineSession *leyline_session_chrome(void);
LeylineSession *leyline_session_firefox(void);
LeylineSession *leyline_session_safari(void);

/* Create a fully configured session.
 * browser:  "chrome", "chrome147", "firefox", "safari", etc.
 * platform: "windows", "macos", "linux", "ios", "android"
 * proxy:    proxy URL or NULL
 * timeout:  seconds (0 = default 30s) */
LeylineSession *leyline_session_new(
    const char *browser,
    const char *platform,
    const char *proxy,
    uint32_t timeout_secs
);

void leyline_session_free(LeylineSession *session);

/* ── Requests ───────────────────────────────────────────────── */

/* GET a URL. Returns NULL on error. */
LeylineResponse *leyline_session_get(const LeylineSession *session, const char *url);

/* POST JSON. body must be a valid JSON string. */
LeylineResponse *leyline_session_post_json(
    const LeylineSession *session,
    const char *url,
    const char *body
);

/* POST form data. data is "key=value&key2=value2". */
LeylineResponse *leyline_session_post_form(
    const LeylineSession *session,
    const char *url,
    const char *data
);

/* ── WebSocket ──────────────────────────────────────────────── */

typedef struct LeylineWebSocket LeylineWebSocket;

LeylineWebSocket *leyline_session_websocket(const LeylineSession *session, const char *url);
int leyline_ws_send(LeylineWebSocket *ws, const char *msg);  /* 0=ok, -1=error */
char *leyline_ws_recv(LeylineWebSocket *ws);                  /* free with leyline_free_string */
int leyline_ws_close(LeylineWebSocket *ws);                   /* 0=ok, -1=error */
void leyline_ws_free(LeylineWebSocket *ws);

/* ── One-liner (default Chrome session) ─────────────────────── */

LeylineResponse *leyline_get(const char *url);

/* ── Response ───────────────────────────────────────────────── */

void leyline_response_free(LeylineResponse *response);

uint16_t leyline_response_status(const LeylineResponse *response);
char *leyline_response_version(const LeylineResponse *response);    /* free with leyline_free_string */
char *leyline_response_tls_alpn(const LeylineResponse *response);   /* free, NULL if unavailable */
char *leyline_response_text(const LeylineResponse *response);       /* free with leyline_free_string */
size_t leyline_response_body_len(const LeylineResponse *response);
char *leyline_response_url(const LeylineResponse *response);        /* free with leyline_free_string */
char *leyline_response_header(const LeylineResponse *response, const char *name); /* free */
char *leyline_response_headers_json(const LeylineResponse *response);  /* free, object; duplicates become arrays */
char *leyline_response_headers_array_json(const LeylineResponse *response); /* free, ordered [name,value] pairs */
char *leyline_response_trailers_json(const LeylineResponse *response); /* free, ordered [name,value] pairs */
char *leyline_response_audit_json(const LeylineResponse *response);    /* free, NULL if unavailable */

#ifdef __cplusplus
}
#endif

#endif /* LEYLINE_H */
