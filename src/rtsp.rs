use std::net::SocketAddr;
use log::{debug, error};
use tokio::net::TcpListener;

pub async fn serve_rtsp_typewriter(listen: String) -> anyhow::Result<()> {
    let socket = listen.parse::<SocketAddr>()
        .expect("Failed to parse listen address");

    loop {
        if let Err(e) = listen_tcp(socket).await {
            error!("RTSP typewriter exited with error, restarting\n{:?}", e);
        } else {
            debug!("RTSP typewriter exited unexpectedly, restarting...");
        }
    }
}

async fn listen_tcp(
    socket: SocketAddr,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(socket).await?;

    loop {
        let (stream, client) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                error!("RTSP Accept failed: {:?}", e);
                continue;
            }
        };

        // TODO: Implement RTSP typewriter
        let _ = stream;
        let _ = client;
    }
}
