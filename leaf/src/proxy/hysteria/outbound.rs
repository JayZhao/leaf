use std::sync::Arc;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use anyhow::Result;
use std::{
    pin::Pin,
    task::{Context, Poll},
    io,
};
use tracing::{debug, info, error};

use crate::{proxy::*, session::Session};
use ::hysteria::{Config, HysteriaClient, quinn};

pub struct Handler {
    client: Arc<HysteriaClient>,
}

struct HysteriaStream {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}
  
impl AsyncRead for HysteriaStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

impl AsyncWrite for HysteriaStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        debug!(target: "hysteria", "[Hysteria客户端] 开始向发送流中写入数据");
        match Pin::new(&mut self.send).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                info!(target: "hysteria", "[Hysteria客户端] 成功写入 {} 字节", n);
                Poll::Ready(Ok(n))
            },
            Poll::Ready(Err(e)) => {
                error!(target: "hysteria", "[Hysteria客户端] 写入失败: {}", e);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            },
            Poll::Pending => {
                debug!(target: "hysteria", "[Hysteria客户端] 写入操作挂起");
                Poll::Pending
            },
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        debug!(target: "hysteria", "[Hysteria客户端] 开始刷新发送流");
        match Pin::new(&mut self.send).poll_flush(cx) {
            Poll::Ready(Ok(())) => {
                info!(target: "hysteria", "[Hysteria客户端] 刷新成功");
                Poll::Ready(Ok(()))
            },
            Poll::Ready(Err(e)) => {
                error!(target: "hysteria", "[Hysteria客户端] 刷新失败: {}", e);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            },
            Poll::Pending => {
                debug!(target: "hysteria", "[Hysteria客户端] 刷新操作挂起");
                Poll::Pending
            },
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        debug!(target: "hysteria", "[Hysteria客户端] 开始关闭发送流");
        match Pin::new(&mut self.send).poll_shutdown(cx) {
            Poll::Ready(Ok(())) => {
                info!(target: "hysteria", "[Hysteria客户端] 关闭成功");
                Poll::Ready(Ok(()))
            },
            Poll::Ready(Err(e)) => {
                error!(target: "hysteria", "[Hysteria客户端] 关闭失败: {}", e);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            },
            Poll::Pending => {
                debug!(target: "hysteria", "[Hysteria客户端] 关闭操作挂起");
                Poll::Pending
            },
        }
    }
}

#[async_trait]
impl OutboundStreamHandler for Handler {
    fn connect_addr(&self) -> OutboundConnect {
        OutboundConnect::Unknown
    }

    async fn handle<'a>(
        &'a self,
        sess: &'a Session,
        _lhs: Option<&mut AnyStream>,
        _stream: Option<AnyStream>,
    ) -> io::Result<AnyStream> {
        let dest = match &sess.destination {
            SocksAddr::Ip(addr) => addr.to_string(),
            SocksAddr::Domain(domain, port) => format!("{}:{}", domain, port),
        };
    
        let (send, recv) = self.client.tcp_connect(&dest)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    
        Ok(Box::new(HysteriaStream { send, recv }))
    }
}

impl Handler {
    pub fn new(
        server_ip: String,
        server_port: u16,
        auth: String,
    ) -> Result<Self> {
        let config = Config {
            server_ip,
            server_port,
            auth,
        };
        
        let client = HysteriaClient::new(config)?;
        tokio::runtime::Handle::current().block_on(async {
            client.connect_and_authenticate()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to authenticate: {}", e))
        })?;

        Ok(Handler { 
            client: Arc::new(client)
        })
    }
}

impl Drop for Handler {
    fn drop(&mut self) {
        tokio::runtime::Handle::current().block_on(async {
            self.client.shutdown().await;
        });
    }
}