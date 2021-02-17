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
        muxrpc::AsyncResponse::String("hello world".to_string())
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
        muxrpc::AsyncResponse::Json(serde_json::to_vec(&payload).unwrap())
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
        muxrpc::AsyncResponse::Error(muxrpc::Error::new("ERROR", "MSG"))
    );

    Ok(())
}

#[async_std::test]
async fn duplex_add() {
    let _ = tracing_subscriber::fmt::init();

    let mut endpoint = connect_client().await.unwrap();
    let (receive, mut send) = endpoint
        .client()
        .start_duplex(
            vec!["duplexAdd".to_string()],
            vec![serde_json::to_value(&1u32).unwrap()],
        )
        .await
        .unwrap();

    let inputs = 0..6u32;
    let inputs2 = inputs.clone();
    let expected_outputs = inputs.map(|x| x + 1).collect::<Vec<_>>();
    async_std::task::spawn(async move {
        for i in inputs2 {
            send.send(muxrpc::Body::json(&i)).await.unwrap();
        }
        send.close().await.unwrap();
    });

    let outputs = receive
        .map_ok(|body| body.decode_json::<u32>().unwrap())
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(outputs, expected_outputs);
}

const SERVER_ADDR: &str = "127.0.0.1:19423";

// Create a client that connects to a server at [SERVER_ADDR].
async fn connect_client() -> Result<muxrpc::Endpoint, std::io::Error> {
    let connection = async_std::net::TcpStream::connect(SERVER_ADDR).await?;
    let (read, write) = connection.split();
    let stream = futures_codec::FramedRead::new(read, futures_codec::BytesCodec)
        .map(|result| result.map(|bytes| Vec::from(bytes.as_ref())));
    Ok(muxrpc::Endpoint::new_client(write.into_sink(), stream))
}
