#[async_std::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    muxrpc::test_server::run("127.0.0.1:9000").await?;
    Ok(())
}
