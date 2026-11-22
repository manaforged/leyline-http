#![no_main]

use leyline::internals::fuzz::read_chunked_body;
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Builder;

const LIMIT: usize = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let rt = Builder::new_current_thread()
        .build()
        .expect("current-thread runtime");
    rt.block_on(async {
        let mut wire = data;
        if let Ok(body) = read_chunked_body(&mut wire, Vec::new(), LIMIT).await {
            assert!(body.len() <= LIMIT);
        }
    });
});
