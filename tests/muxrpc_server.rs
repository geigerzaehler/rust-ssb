#[cfg(feature = "test-server")]
#[test]
fn server() {
    tracing_subscriber::fmt::init();

    let port = "15399";
    async_std::task::spawn(async move {
        ssb::rpc::base::test_server::run(format!("127.0.0.1:{}", port))
            .await
            .unwrap();
    });

    let mut cmd = assert_cmd::Command::new("node");
    cmd.current_dir("muxrpc-test-suite");
    cmd.args(&["node_modules/.bin/mocha", "--invert", "--fgrep", "no-rust"]);
    cmd.env("EXTERNAL_SERVER", port);
    cmd.unwrap();
}
