use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use bytes::BytesMut;
use etherparse::{IpHeader, PacketHeaders};
use quinn::Connection;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_tun::{Tun, TunBuilder};
use tracing::warn;

pub struct TunWorker {
    tun_read: Arc<RwLock<ReadHalf<Tun>>>,
    tun_write: Arc<RwLock<WriteHalf<Tun>>>,
    write_queue_sender: Arc<UnboundedSender<BytesMut>>,
    write_queue_receiver: Arc<RwLock<UnboundedReceiver<BytesMut>>>,
    active_connections: Arc<RwLock<HashMap<IpAddr, Arc<Connection>>>>,
    buffer_size: usize,
    reader_task: Option<JoinHandle<Result<()>>>,
    writer_task: Option<JoinHandle<Result<()>>>,
}

impl TunWorker {
    pub fn new(tun: Tun, buffer_size: usize) -> Self {
        let (tun_read, tun_write) = tokio::io::split(tun);
        let (write_queue_sender, write_queue_receiver) = tokio::sync::mpsc::unbounded_channel();

        Self {
            tun_read: Arc::new(RwLock::new(tun_read)),
            tun_write: Arc::new(RwLock::new(tun_write)),
            write_queue_sender: Arc::new(write_queue_sender),
            write_queue_receiver: Arc::new(RwLock::new(write_queue_receiver)),
            active_connections: Arc::new(RwLock::new(HashMap::new())),
            buffer_size,
            reader_task: None,
            writer_task: None,
        }
    }

    pub async fn add_connection(&self, remote_addr: IpAddr, connection: Arc<Connection>) {
        self.active_connections
            .write()
            .await
            .insert(remote_addr, connection);
    }

    pub fn get_tun_sender(&self) -> Arc<UnboundedSender<BytesMut>> {
        self.write_queue_sender.clone()
    }

    pub async fn start_workers(&mut self) -> Result<()> {
        if self.reader_task.is_some() {
            return Err(anyhow!("There is already a reader job active"));
        }

        let tun_read = self.tun_read.clone();
        let tun_write = self.tun_write.clone();
        let write_queue_receiver = self.write_queue_receiver.clone();
        let active_connections = self.active_connections.clone();
        let buffer_size = self.buffer_size;

        self.reader_task = Some(tokio::spawn(Self::process_incoming_data(
            tun_read,
            active_connections,
            buffer_size,
        )));
        self.writer_task = Some(tokio::spawn(Self::process_outgoing_data(
            tun_write,
            write_queue_receiver,
        )));

        Ok(())
    }

    pub async fn stop_workers(&mut self) -> Result<()> {
        let reader_task = self
            .reader_task
            .as_mut()
            .ok_or_else(|| anyhow!("Reader task not active"))?;
        reader_task.abort();

        self.reader_task = None;

        let writer_task = self
            .writer_task
            .as_mut()
            .ok_or_else(|| anyhow!("Writer task not active"))?;
        writer_task.abort();

        self.writer_task = None;

        Ok(())
    }

    async fn process_incoming_data(
        tun_read: Arc<RwLock<ReadHalf<Tun>>>,
        active_connections: Arc<RwLock<HashMap<IpAddr, Arc<Connection>>>>,
        buffer_size: usize,
    ) -> Result<()> {
        let mut tun_read = tun_read.write().await;

        loop {
            let mut buf = BytesMut::with_capacity(buffer_size);

            tun_read.read_buf(&mut buf).await?;

            let headers = PacketHeaders::from_ethernet_slice(&buf)?;
            let ip_header = match headers.ip {
                Some(ip_header) => ip_header,
                None => continue,
            };

            let dest_addr: IpAddr = match ip_header {
                IpHeader::Version4(header, _) => header.destination.into(),
                IpHeader::Version6(header, _) => header.destination.into(),
            };

            let connections = active_connections.read().await;

            let connection = connections
                .get(&dest_addr)
                .ok_or_else(|| anyhow!("Received a packet with invalid destination IP"))?;

            let max_datagram_size = connection.max_datagram_size().ok_or_else(|| {
                anyhow!(
                    "Client {} failed to provide maximum datagram size",
                    connection.remote_address()
                )
            })?;

            if buf.len() > max_datagram_size {
                warn!(
                    "Dropping packet of size {} due to maximum datagram size being {}",
                    buf.len(),
                    max_datagram_size
                );
                continue;
            }

            connection.send_datagram(buf.into())?;
        }
    }

    async fn process_outgoing_data(
        tun_write: Arc<RwLock<WriteHalf<Tun>>>,
        write_queue_receiver: Arc<RwLock<UnboundedReceiver<BytesMut>>>,
    ) -> Result<()> {
        let mut tun_write = tun_write.write().await;
        let mut write_queue_receiver = write_queue_receiver.write().await;

        loop {
            let buf = match write_queue_receiver.recv().await {
                Some(buf) => buf,
                None => break,
            };

            tun_write.write_all(&buf).await?;
        }

        Ok(())
    }
}

pub fn make_tun(
    name: String,
    local_addr: Ipv4Addr,
    mask: Ipv4Addr,
    remote_addr: Ipv4Addr,
    mtu: u32,
) -> Result<Tun> {
    let tun = TunBuilder::new()
        .name(&name)
        .tap(false)
        .packet_info(false)
        .mtu(mtu as i32)
        .up()
        .address(local_addr)
        .destination(remote_addr)
        .netmask(mask)
        .try_build()
        .map_err(|e| anyhow!("Failed to create a TUN interface: {e}"))?;

    Ok(tun)
}
