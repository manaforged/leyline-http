use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use http::StatusCode;
use tokio::net::TcpListener;

use super::fake_server::{Reply, spawn_hung, spawn_replies};
use super::httpbin::{
    HTTPBIN_ATTEMPTS, HttpbinAttempt, HttpbinUnavailable, attempt_httpbin, expect_httpbin_status,
    httpbin_eq, read_httpbin_json, read_httpbin_text, session_with_timeout, with_httpbin,
};
use super::{finish, smoke};

#[test]
fn server_error_is_retried_and_other_status_fails() {
    match expect_httpbin_status(StatusCode::SERVICE_UNAVAILABLE, StatusCode::OK) {
        Err(HttpbinAttempt::Unavailable(reason)) => assert!(reason.contains("503"), "{reason}"),
        other => panic!("expected an unavailable status, got {other:?}"),
    }
    match expect_httpbin_status(StatusCode::NOT_FOUND, StatusCode::OK) {
        Err(HttpbinAttempt::Fatal(reason)) => assert!(reason.contains("404"), "{reason}"),
        other => panic!("expected a fatal status, got {other:?}"),
    }
    match expect_httpbin_status(StatusCode::BAD_REQUEST, StatusCode::NOT_FOUND) {
        Err(HttpbinAttempt::Fatal(reason)) => assert!(reason.contains("400"), "{reason}"),
        other => panic!("expected a fatal status, got {other:?}"),
    }
    expect_httpbin_status(StatusCode::NOT_FOUND, StatusCode::NOT_FOUND)
        .expect("the status the subtest asked for");
    expect_httpbin_status(StatusCode::OK, StatusCode::OK).expect("200 matches");
}

#[tokio::test]
async fn httpbin_timeout_skips_and_other_timeout_fails() {
    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    finish(
        "hung httpbin",
        &mut passed,
        &mut failed,
        &mut skipped,
        true,
        std::time::Duration::from_millis(40),
        smoke(async {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            Ok("late".to_string())
        }),
    )
    .await;
    assert_eq!((passed, failed, skipped), (0, 0, 1));

    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    finish(
        "hung fingerprint",
        &mut passed,
        &mut failed,
        &mut skipped,
        false,
        std::time::Duration::from_millis(40),
        smoke(async {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            Ok("late".to_string())
        }),
    )
    .await;
    assert_eq!((passed, failed, skipped), (0, 1, 0));
}

#[tokio::test]
async fn wrong_body_fails_without_retry_and_outage_skips() {
    let url = spawn_replies(vec![
        Reply {
            status: 200,
            headers: "",
            body: r#"{"json":{"test":"nope"}}"#,
        },
        Reply {
            status: 200,
            headers: "",
            body: r#"{"json":{"test":"leyline"}}"#,
        },
    ])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let err = attempt_httpbin(3, std::time::Duration::ZERO, {
        let url = url.clone();
        let calls = Arc::clone(&calls);
        move || {
            let url = url.clone();
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                let response = session.post(&url).await?;
                let value = read_httpbin_json(response).await?;
                let echoed = value["json"]["test"].as_str().unwrap_or("");
                httpbin_eq(echoed == "leyline", format!("echoed={echoed}"))?;
                Ok(format!("echoed={echoed}"))
            }
        }
    })
    .await
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(err.downcast_ref::<HttpbinUnavailable>().is_none(), "{err}");
    assert!(err.to_string().contains("echoed=nope"), "{err}");

    let url = spawn_replies(vec![Reply {
        status: 503,
        headers: "",
        body: "<html>unavailable</html>",
    }])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let err = with_httpbin({
        let url = url.clone();
        let calls = Arc::clone(&calls);
        move || {
            let url = url.clone();
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                let response = session.get(&url).await?;
                let body = read_httpbin_text(response, StatusCode::OK).await?;
                httpbin_eq(body.contains("headers"), "httpbin body missing headers")?;
                Ok(body)
            }
        }
    })
    .await
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), HTTPBIN_ATTEMPTS as usize);
    let skip = err
        .downcast_ref::<HttpbinUnavailable>()
        .unwrap_or_else(|| panic!("expected a skip, got {err}"));
    assert!(skip.to_string().contains("503"), "{skip}");
}

#[tokio::test]
async fn non_json_retries_then_skips_and_bad_gzip_fails() {
    let url = spawn_replies(vec![
        Reply {
            status: 200,
            headers: "Content-Type: text/html\r\n",
            body: "<html>down</html>",
        },
        Reply {
            status: 200,
            headers: "Content-Type: application/json\r\n",
            body: r#"{"args":{"foo":"bar"}}"#,
        },
    ])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let detail = attempt_httpbin(3, std::time::Duration::ZERO, {
        let url = url.clone();
        let calls = Arc::clone(&calls);
        move || {
            let url = url.clone();
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                let response = session.get(&url).await?;
                let value = read_httpbin_json(response).await?;
                let foo = value["args"]["foo"].as_str().unwrap_or("");
                httpbin_eq(foo == "bar", format!("foo={foo}"))?;
                Ok(format!("foo={foo}"))
            }
        }
    })
    .await
    .expect("second body is JSON");
    assert_eq!(detail, "foo=bar");
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let url = spawn_replies(vec![Reply {
        status: 200,
        headers: "Content-Type: text/html\r\n",
        body: "not-json",
    }])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let err = attempt_httpbin(3, std::time::Duration::ZERO, {
        let calls = Arc::clone(&calls);
        move || {
            let url = url.clone();
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                let response = session.get(&url).await?;
                let _ = read_httpbin_json(response).await?;
                Ok("ok".to_string())
            }
        }
    })
    .await
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let skip = err
        .downcast_ref::<HttpbinUnavailable>()
        .unwrap_or_else(|| panic!("expected a skip, got {err}"));
    assert!(skip.to_string().contains("non-JSON"), "{skip}");

    let url = spawn_replies(vec![Reply {
        status: 200,
        headers: "Content-Encoding: gzip\r\n",
        body: "this is not gzip",
    }])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let err = attempt_httpbin(3, std::time::Duration::ZERO, {
        let calls = Arc::clone(&calls);
        move || {
            let url = url.clone();
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                let response = session.get(&url).await?;
                let _ = read_httpbin_text(response, StatusCode::OK).await?;
                Ok("ok".to_string())
            }
        }
    })
    .await
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "{err}");
    assert!(err.downcast_ref::<HttpbinUnavailable>().is_none(), "{err}");
    assert!(err.to_string().contains("gzip"), "{err}");
}

#[tokio::test]
async fn connect_and_read_timeouts_skip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let calls = Arc::new(AtomicUsize::new(0));
    let err = attempt_httpbin(3, std::time::Duration::ZERO, {
        let calls = Arc::clone(&calls);
        move || {
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                let response = session.get(format!("http://{addr}/get")).await?;
                let _ = read_httpbin_text(response, StatusCode::OK).await?;
                Ok("ok".to_string())
            }
        }
    })
    .await
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(err.downcast_ref::<HttpbinUnavailable>().is_some(), "{err}");

    let url = spawn_hung().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let err = attempt_httpbin(2, std::time::Duration::ZERO, {
        let calls = Arc::clone(&calls);
        move || {
            let url = url.clone();
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let session = session_with_timeout(std::time::Duration::from_millis(200))?;
                let response = session.get(&url).await?;
                let _ = read_httpbin_text(response, StatusCode::OK).await?;
                Ok("ok".to_string())
            }
        }
    })
    .await
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let skip = err
        .downcast_ref::<HttpbinUnavailable>()
        .unwrap_or_else(|| panic!("expected a skip, got {err}"));
    assert!(skip.to_string().contains("timeout"), "{skip}");
}

#[tokio::test]
async fn recovery_after_a_5xx_checks_the_body() {
    let url = spawn_replies(vec![
        Reply {
            status: 503,
            headers: "",
            body: "down",
        },
        Reply {
            status: 200,
            headers: "",
            body: r#"{"headers":{"Authorization":"Bearer test-token-123"}}"#,
        },
    ])
    .await;
    let calls = Arc::new(AtomicUsize::new(0));
    let detail = attempt_httpbin(3, std::time::Duration::ZERO, {
        let calls = Arc::clone(&calls);
        move || {
            let url = url.clone();
            let calls = Arc::clone(&calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                let response = session.get(&url).await?;
                let value = read_httpbin_json(response).await?;
                let auth = value["headers"]["Authorization"].as_str().unwrap_or("");
                httpbin_eq(auth.contains("test-token-123"), format!("auth={auth}"))?;
                Ok(format!("auth={auth}"))
            }
        }
    })
    .await
    .expect("the second response is valid");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(detail.contains("test-token-123"), "{detail}");
}
