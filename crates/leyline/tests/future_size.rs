use leyline::Session;

const SEND_FUTURE_BUDGET: usize = 1024;

#[test]
fn a_request_future_stays_small_enough_to_nest_on_a_thread_stack() {
    let session = Session::new();
    let get = session.get("http://127.0.0.1:9/").send();
    let post = session.post("http://127.0.0.1:9/").body("x").send();
    assert!(
        size_of_val(&get) <= SEND_FUTURE_BUDGET,
        "get send future {} bytes",
        size_of_val(&get)
    );
    assert!(
        size_of_val(&post) <= SEND_FUTURE_BUDGET,
        "post send future {} bytes",
        size_of_val(&post)
    );
}
