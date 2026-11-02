use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use leyline::Session;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);

// SAFETY: every call forwards to the system allocator with the caller's layout and pointer.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the caller upholds GlobalAlloc::alloc's contract for this layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr came from System.alloc with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: ptr came from System.alloc with this layout.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const RESPONSE: &[u8] =
    b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\ncontent-type: text/plain\r\n\r\nok-10byte!";
const WARM: u64 = 64;
const MEASURED: u64 = 256;
const ALLOCS_PER_REQUEST: u64 = 14;

async fn origin() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buf = [0u8; 4096];
        let mut filled = 0;
        loop {
            let n = stream.read(&mut buf[filled..]).await.expect("read");
            if n == 0 {
                return;
            }
            filled += n;
            while let Some(end) = buf[..filled].windows(4).position(|w| w == b"\r\n\r\n") {
                stream.write_all(RESPONSE).await.expect("write");
                buf.copy_within(end + 4..filled, 0);
                filled -= end + 4;
            }
        }
    });
    format!("http://{addr}/")
}

#[tokio::test]
async fn a_warm_http1_request_stays_within_its_allocation_budget() {
    let url = origin().await;
    let session = Session::new();
    let fetch = || async {
        let response = session.get(url.as_str()).send().await.expect("send");
        assert_eq!(&response.bytes().await.expect("body")[..], b"ok-10byte!");
    };
    for _ in 0..WARM {
        fetch().await;
    }
    let before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..MEASURED {
        fetch().await;
    }
    let per_request = (ALLOCS.load(Ordering::Relaxed) - before) / MEASURED;
    assert!(
        per_request <= ALLOCS_PER_REQUEST,
        "{per_request} allocations per request"
    );
}
