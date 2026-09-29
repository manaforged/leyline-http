use super::*;

#[test]
fn writer_appends_lines_and_flushes() {
    let dir = std::env::temp_dir().join(format!(
        "leyline-keylog-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sslkeylog.txt");

    let write = keylog_writer(path.to_str().unwrap()).unwrap();
    write("CLIENT_HANDSHAKE_TRAFFIC_SECRET abc 123");
    write("SERVER_TRAFFIC_SECRET_0 abc 456");

    let contents = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        contents,
        "CLIENT_HANDSHAKE_TRAFFIC_SECRET abc 123\nSERVER_TRAFFIC_SECRET_0 abc 456\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
