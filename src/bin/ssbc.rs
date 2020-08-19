#[async_std::main]
async fn main() -> anyhow::Result<()> {
    ssb::ssbc::main().await
}
