#[cfg(feature = "test-server")]
#[async_std::test]
async fn server() {
    tracing_subscriber::fmt::init();

    let port = "15399";
    let server_task =
        async_std::task::spawn(muxrpc::test_server::run(format!("127.0.0.1:{}", port)));

    let mut cmd = assert_cmd::Command::new("node");
    cmd.current_dir("muxrpc-test-suite");
    cmd.args(&["node_modules/.bin/mocha", "--invert", "--fgrep", "no-rust"]);
    cmd.env("EXTERNAL_SERVER", port);
    cmd.unwrap();

    if let Some(Err(err)) = server_task.cancel().await {
        panic!("{:?}", err);
    }
}
