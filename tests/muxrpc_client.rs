//! Test the RPC base client against the NodeJS test server
use futures::prelude::*;
use std::pin::Pin;
use std::task::Poll;

#[async_std::test]
async fn echo_string() -> anyhow::Result<()> {
    let mut client = connect_client().await?;

    let result = client
        .send_async(
            vec!["echo".to_string()],
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
    let mut client = connect_client().await?;

    let payload = serde_json::json!({
        "hello": "world"
    });
    let result = client
        .send_async(vec!["echo".to_string()], vec![payload.clone()])
        .await?;

    assert_eq!(
        result,
        ssb::rpc::base::AsyncResponse::Json(serde_json::to_vec(&payload).unwrap())
    );

    Ok(())
}

#[async_std::test]
async fn echo_error() -> anyhow::Result<()> {
    let mut client = connect_client().await?;

    let payload = serde_json::json!({
        "name": "ERROR",
        "message": "MSG"
    });
    let result = client
        .send_async(vec!["errorAsync".to_string()], vec![payload.clone()])
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
async fn connect_client() -> Result<ssb::rpc::base::Client, std::io::Error> {
    let connection = async_std::net::TcpStream::connect(SERVER_ADDR).await?;
    let (read, write) = connection.split();
    let stream = read_to_stream(read);
    Ok(ssb::rpc::base::Client::new(write.into_sink(), stream))
}

/// Convert [AsyncRead] into a [Stream]. Polling the resulting stream will poll
/// the reader for 4096 bytes and return a [Vec] of all the bytes that were read.
fn read_to_stream(
    read: impl AsyncRead + Unpin,
) -> impl Stream<Item = Result<Vec<u8>, std::io::Error>> {
    const BUF_SIZE: usize = 4096;
    let mut read = read;
    let mut buf = vec![0u8; BUF_SIZE];
    futures::stream::poll_fn(move |cx| {
        let result = match futures::ready!(Pin::new(&mut read).poll_read(cx, &mut buf)) {
            Ok(size) => {
                if size == 0 {
                    None
                } else {
                    Some(Ok(Vec::from(&buf[..size])))
                }
            }
            Err(err) => Some(Err(err)),
        };
        Poll::Ready(result)
    })
}
