use std::sync::Arc;
use async_trait::async_trait;
use std::io;

use crate::{proxy::*, session::Session};
use ::hysteria::HysteriaClient;

pub struct Handler {
    client: Arc<HysteriaClient>,
}

impl Handler {
    pub fn new(client: Arc<HysteriaClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl OutboundDatagramHandler for Handler {
    fn connect_addr(&self) -> OutboundConnect {
        OutboundConnect::Unknown
    }

    fn transport_type(&self) -> DatagramTransportType {
        DatagramTransportType::Unreliable
    }

    async fn handle<'a>(
        &'a self,
        sess: &'a Session,
        _transport: Option<AnyOutboundTransport>,
    ) -> io::Result<AnyOutboundDatagram> {
        Ok(Box::new(HysteriaDatagram {
            client: self.client.clone(),
            destination: sess.destination.clone(),
        }))
    }
}

struct HysteriaDatagram {
    client: Arc<HysteriaClient>,
    destination: SocksAddr,
}

impl OutboundDatagram for HysteriaDatagram {
    fn split(
        self: Box<Self>,
    ) -> (
        Box<dyn OutboundDatagramRecvHalf>,
        Box<dyn OutboundDatagramSendHalf>,
    ) {
        let datagram = Arc::new(*self);
        (
            Box::new(DatagramRecvHalf(datagram.clone())),
            Box::new(DatagramSendHalf(datagram)),
        )
    }
}

struct DatagramRecvHalf(Arc<HysteriaDatagram>);
struct DatagramSendHalf(Arc<HysteriaDatagram>);

#[async_trait]
impl OutboundDatagramRecvHalf for DatagramRecvHalf {
    async fn recv_from(&mut self, buf: &mut [u8]) -> io::Result<(usize, SocksAddr)> {
        let (data, addr) = self.0.client
            .udp_receive()
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);

        let addr = addr.rsplit_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid address format"))?;
        
        let port = addr.1.parse::<u16>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let socks_addr = if let Ok(ip) = addr.0.parse::<std::net::IpAddr>() {
            SocksAddr::from((ip, port))
        } else {
            SocksAddr::Domain(addr.0.to_string(), port)
        };

        Ok((n, socks_addr))
    }
}

#[async_trait]
impl OutboundDatagramSendHalf for DatagramSendHalf {
    async fn send_to(&mut self, buf: &[u8], _target: &SocksAddr) -> io::Result<usize> {
        self.0.client
            .udp_send(buf, &self.0.destination.to_string())
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            
        Ok(buf.len())
    }

    async fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
} 