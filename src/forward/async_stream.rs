use async_trait::async_trait;
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};

#[async_trait]
pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {
    async fn try_shutdown(self) -> std::io::Result<()>;
}

#[async_trait]
impl AsyncStream for TcpStream {
    async fn try_shutdown(mut self) -> std::io::Result<()> {
        let _ = self.shutdown().await;

        // Unfortunately, AsyncWriteExt::shutdown/AsyncWrite::poll_shutdown only ends up
        // calling std::net::Shutdown::Write and seems to leave sockets in
        // CLOSE-WAIT/TIME-WAIT/FIN-WAIT states.

        // We should shutdown the entire socket.
        let std = self.into_std()?;
        std.shutdown(std::net::Shutdown::Both)
    }
}
