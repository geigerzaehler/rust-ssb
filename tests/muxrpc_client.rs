//! Test the RPC base client against the NodeJS test server
use futures::prelude::*;

#[async_std::test]
async fn echo_string() -> anyhow::Result<()> {
    let mut endpoint = connect_client().await?;

    let result = endpoint
        .client()
        .send_async(
            vec!["asyncEcho".to_string()],
            vec![serde_json::json!("hello world")],
        )
        .await?;

    assert_eq!(
        result,
        ssb::rpc::base::AsyncResponse::String("hello world".to_string())
    );

    Ok(())
}

#[async_std::test]
async fn echo_json() -> anyhow::Result<()> {
    let mut endpoint = connect_client().await?;

    let payload = serde_json::json!({
        "hello": "world"
    });
    let result = endpoint
        .client()
        .send_async(vec!["asyncEcho".to_string()], vec![payload.clone()])
        .await?;

    assert_eq!(
        result,
        ssb::rpc::base::AsyncResponse::Json(serde_json::to_vec(&payload).unwrap())
    );

    Ok(())
}

#[async_std::test]
async fn echo_error() -> anyhow::Result<()> {
    let mut endpoint = connect_client().await?;

    let payload = serde_json::json!({
        "name": "ERROR",
        "message": "MSG"
    });
    let result = endpoint
        .client()
        .send_async(vec!["asyncError".to_string()], vec![payload.clone()])
        .await?;

    assert_eq!(
        result,
        ssb::rpc::base::AsyncResponse::Error {
            name: "ERROR".to_string(),
            message: "MSG".to_string()
        }
    );

    Ok(())
}

const SERVER_ADDR: &str = "127.0.0.1:8080";

// Create a client that connects to a server at [SERVER_ADDR].
async fn connect_client() -> Result<ssb::rpc::base::Endpoint, std::io::Error> {
    let connection = async_std::net::TcpStream::connect(SERVER_ADDR).await?;
    let (read, write) = connection.split();
    let stream = ssb::utils::read_to_stream(read);
    Ok(ssb::rpc::base::Endpoint::new_client(
        write.into_sink(),
        stream,
    ))
}
