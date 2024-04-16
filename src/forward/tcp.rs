use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use futures::join;
use log::{debug, error};
use tokio::net::TcpStream;

use crate::forward::{
    async_stream::AsyncStream,
    copy_bidirectional::copy_bidirectional,
    location::{Location, NetLocation},
    tokio_util::resolve_host,
};

pub struct TargetLocationData {
    pub location: Location,
}

pub struct TargetData {
    pub location_data: Vec<TargetLocationData>,
    pub next_address_index: AtomicUsize,
    pub tcp_nodelay: bool,
}

const BUFFER_SIZE: usize = 8192;

pub async fn process_generic_stream(
    mut source_stream: Box<TcpStream>,
    addr: &std::net::SocketAddr,
    target_data: Arc<TargetData>,
) -> std::io::Result<()> {
    let target_location = if target_data.location_data.len() > 1 {
        // fetch_add wraps around on overflow.
        let index = target_data
            .next_address_index
            .fetch_add(1, Ordering::Relaxed);
        &target_data.location_data[index % target_data.location_data.len()]
    } else {
        &target_data.location_data[0]
    };

    let mut target_stream =
        match setup_target_stream(addr, &target_location, target_data.tcp_nodelay).await {
            Ok(s) => s,
            Err(e) => {
                source_stream.try_shutdown().await?;
                return Err(e);
            }
        };

    debug!(
        "Copying: {}:{} to {}",
        addr.ip(),
        addr.port(),
        &target_location.location,
    );

    let copy_result = copy_bidirectional(&mut source_stream, &mut target_stream, BUFFER_SIZE).await;

    debug!(
        "Shutdown: {}:{} to {}",
        addr.ip(),
        addr.port(),
        &target_location.location,
    );

    let (_, _) = join!(source_stream.try_shutdown(), target_stream.try_shutdown());

    debug!(
        "Done: {}:{} to {}",
        addr.ip(),
        addr.port(),
        &target_location.location,
    );

    copy_result?;

    Ok(())
}

async fn setup_target_stream(
    addr: &std::net::SocketAddr,
    target_location: &TargetLocationData,
    tcp_nodelay: bool,
) -> std::io::Result<Box<TcpStream>> {
    match target_location.location {
        Location::Address(NetLocation { ref address, port }) => {
            let target_addr = resolve_host((address.as_str(), port)).await?;
            let tcp_stream = TcpStream::connect(target_addr).await?;
            if tcp_nodelay {
                if let Err(e) = tcp_stream.set_nodelay(true) {
                    error!("Failed to set tcp_nodelay on target stream: {}", e);
                }
            }
            debug!(
                "Connected to remote: {} using local addr {}",
                addr,
                tcp_stream.local_addr().unwrap()
            );

            Ok(Box::new(tcp_stream))
        }
    }
}
