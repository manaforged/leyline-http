#![no_main]

use leyline::fuzz::read_chunked_body;
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Builder;

fuzz_target!(|data: &[u8]| {
    let rt = Builder::new_current_thread()
        .build()
        .expect("current-thread runtime");
    rt.block_on(async {
        let mut wire = data;
        drop(read_chunked_body(&mut wire, Vec::new(), 1 << 20).await);
    });
});
