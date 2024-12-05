use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::Mutex;
use std::io;

use crate::{proxy::*, session::Session};
use ::hysteria::{UdpSession, HysteriaClient};

pub struct Handler {
    client: Arc<HysteriaClient>,
    session: Arc<Mutex<Option<Arc<UdpSession>>>>,
}

#[async_trait]
impl OutboundDatagramHandler for Handler {
    fn connect_addr(&self) -> OutboundConnect {
        OutboundConnect::Unknown
    }

    fn transport_type(&self) -> DatagramTransportType {
        // Hysteria 使用 QUIC 不可靠数据报
        DatagramTransportType::Unreliable
    }

    async fn handle<'a>(
        &'a self,
        sess: &'a Session,
        _transport: Option<AnyOutboundTransport>,
    ) -> io::Result<AnyOutboundDatagram> {
        let session = {
            let mut session_guard = self.session.lock().await;
            if session_guard.is_none() {
                *session_guard = Some(
                    self.client
                        .get_or_create_session()
                        .await
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
                );
            }
            session_guard.as_ref().unwrap().clone()
        };
    
        Ok(Box::new(HysteriaDatagram {
            session,
            destination: sess.destination.clone(),
        }))
    }
}

struct HysteriaDatagram {
    session: Arc<UdpSession>,
    destination: SocksAddr,
}

impl OutboundDatagram for HysteriaDatagram {
    fn split(
        self: Box<Self>,
    ) -> (
        Box<dyn OutboundDatagramRecvHalf>,
        Box<dyn OutboundDatagramSendHalf>,
    ) {
        let session = Arc::new(*self);
        (
            Box::new(DatagramRecvHalf(session.clone())),
            Box::new(DatagramSendHalf(session))
        )
    }
}

struct DatagramRecvHalf(Arc<HysteriaDatagram>);
struct DatagramSendHalf(Arc<HysteriaDatagram>);

#[async_trait]
impl OutboundDatagramRecvHalf for DatagramRecvHalf {
    async fn recv_from(&mut self, buf: &mut [u8]) -> io::Result<(usize, SocksAddr)> {
        let (data, addr) = self.0.session
            .receive()
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
        // 使用 UdpSession 的 send 方法发送数据
        self.0.session
            .send(buf, &self.0.destination.to_string())
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            
        Ok(buf.len())
    }

    async fn close(&mut self) -> io::Result<()> {
        // UDP 会话不需要显式关闭
        Ok(())
    }
} 