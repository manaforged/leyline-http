use leyline::Browser;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

const EXT_KEY_SHARE: u16 = 0x0033;
const GROUP_X25519: u16 = 0x001d;
const GROUP_X25519_MLKEM768: u16 = 0x11ec;
const MLKEM768_EK_LEN: usize = 1184;
const X25519_PUB_LEN: usize = 32;

#[tokio::test]
async fn chrome_pq_key_shares_use_distinct_x25519_ephemerals() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = Vec::with_capacity(4096);
        let mut tmp = [0u8; 2048];
        loop {
            match tokio::time::timeout(std::time::Duration::from_millis(750), stream.read(&mut tmp))
                .await
            {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(n)) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.len() >= 5 {
                        let rec_len = u16::from_be_bytes([buf[3], buf[4]]) as usize;
                        if buf.len() >= 5 + rec_len {
                            break;
                        }
                    }
                }
                Ok(Err(_)) => break,
            }
        }
        buf
    });

    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    drop(
        session
            .get(format!("https://{}:{}/", addr.ip(), addr.port()))
            .await,
    );

    let bytes = server.await.unwrap();
    assert!(
        bytes.len() >= 5,
        "no TLS record captured (got {} bytes)",
        bytes.len()
    );
    assert_eq!(bytes[0], 0x16, "first byte is not a TLS handshake record");

    let shares = parse_key_shares(&bytes)
        .expect("failed to parse key_share extension from captured ClientHello");

    let x25519 = shares
        .iter()
        .find(|(g, _)| *g == GROUP_X25519)
        .map(|(_, kx)| kx.clone())
        .expect("Chrome 147 ClientHello did not contain an X25519 key_share");

    let hybrid = shares
        .iter()
        .find(|(g, _)| *g == GROUP_X25519_MLKEM768)
        .map(|(_, kx)| kx.clone())
        .expect(
            "Chrome 147 ClientHello did not contain an X25519MLKEM768 key_share \
             — check that BoringSSL is configured to send both hybrid and classical shares",
        );

    assert_eq!(x25519.len(), X25519_PUB_LEN, "X25519 share wrong length");
    assert_eq!(
        hybrid.len(),
        MLKEM768_EK_LEN + X25519_PUB_LEN,
        "X25519MLKEM768 share wrong length (expected ML-KEM768 || X25519)"
    );

    let hybrid_head = &hybrid[..X25519_PUB_LEN];
    let hybrid_tail = &hybrid[hybrid.len() - X25519_PUB_LEN..];
    assert_ne!(
        hybrid_head,
        &x25519[..],
        "standalone X25519 ephemeral matches the head of the X25519MLKEM768 \
         share — utls#342 ephemeral reuse distinguisher"
    );
    assert_ne!(
        hybrid_tail,
        &x25519[..],
        "standalone X25519 ephemeral matches the tail of the X25519MLKEM768 \
         share — utls#342 ephemeral reuse distinguisher"
    );

    println!(
        "✓ PQ ephemerals distinct: X25519={}…  hybrid_tail={}…",
        hex_prefix(&x25519),
        hex_prefix(hybrid_tail)
    );
}

fn hex_prefix(b: &[u8]) -> String {
    b.iter().take(6).map(|x| format!("{x:02x}")).collect()
}

fn parse_key_shares(rec: &[u8]) -> Option<Vec<(u16, Vec<u8>)>> {
    if rec.len() < 5 || rec[0] != 0x16 {
        return None;
    }
    let rec_len = u16::from_be_bytes([rec[3], rec[4]]) as usize;
    let body = rec.get(5..5 + rec_len)?;

    if *body.first()? != 0x01 {
        return None;
    }
    let hs_len = ((body[1] as usize) << 16) | ((body[2] as usize) << 8) | (body[3] as usize);
    let ch = body.get(4..4 + hs_len)?;

    let mut i = 2 + 32;
    let sid_len = *ch.get(i)? as usize;
    i += 1 + sid_len;

    let cs_len = u16::from_be_bytes([*ch.get(i)?, *ch.get(i + 1)?]) as usize;
    i += 2 + cs_len;

    let cm_len = *ch.get(i)? as usize;
    i += 1 + cm_len;

    let ext_len = u16::from_be_bytes([*ch.get(i)?, *ch.get(i + 1)?]) as usize;
    i += 2;
    let ext_end = i + ext_len;
    if ext_end > ch.len() {
        return None;
    }

    while i + 4 <= ext_end {
        let etype = u16::from_be_bytes([ch[i], ch[i + 1]]);
        let elen = u16::from_be_bytes([ch[i + 2], ch[i + 3]]) as usize;
        i += 4;
        let edata = ch.get(i..i + elen)?;
        i += elen;
        if etype != EXT_KEY_SHARE {
            continue;
        }

        if edata.len() < 2 {
            return None;
        }
        let ks_len = u16::from_be_bytes([edata[0], edata[1]]) as usize;
        let entries = edata.get(2..2 + ks_len)?;

        let mut out = Vec::new();
        let mut j = 0usize;
        while j + 4 <= entries.len() {
            let group = u16::from_be_bytes([entries[j], entries[j + 1]]);
            let kx_len = u16::from_be_bytes([entries[j + 2], entries[j + 3]]) as usize;
            j += 4;
            let kx = entries.get(j..j + kx_len)?.to_vec();
            j += kx_len;
            out.push((group, kx));
        }
        return Some(out);
    }

    None
}
