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
        info!(target: "hysteria", "[Hysteria客户端] 开始创建新的Handler，服务器地址: {}:{}", server_ip, server_port);
        
        let config = Config {
            server_ip,
            server_port,
            auth,
        };
        
        debug!(target: "hysteria", "[Hysteria客户端] 使用配置创建新的HysteriaClient实例");
        let client = HysteriaClient::new(config)?;
        
        let runtime = tokio::runtime::Handle::current();
        info!(
            target: "hysteria",
            "[Hysteria客户端] Runtime诊断信息 - Runtime类型: {:?}, 是否在Tokio线程内: {}, 当前线程名称: {}",
            runtime.runtime_flavor(),
            tokio::runtime::Handle::try_current().is_ok(),
            std::thread::current().name().unwrap_or("unnamed"),
        );
        
        info!(target: "hysteria", "[Hysteria客户端] 准备进行认证，将创建新的Runtime执行认证");
        let result = {
            debug!(target: "hysteria", "[Hysteria客户端] 开始创建新的Runtime");
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow::anyhow!("创建Runtime失败: {}", e))?;
            
            debug!(target: "hysteria", "[Hysteria客户端] 新Runtime创建成功，准备执行认证");
            let auth_future = async {
                debug!(target: "hysteria", "[Hysteria客户端] 进入block_on块，开始认证");
                let auth_result = client.connect_and_authenticate().await;
                debug!(target: "hysteria", "[Hysteria客户端] 认证过程完成");
                auth_result.map_err(|e| anyhow::anyhow!("认证失败: {}", e))
            };

            debug!(target: "hysteria", "[Hysteria客户端] 调用block_on执行认证");
            rt.block_on(auth_future)
        };

        match &result {
            Ok(_) => info!(target: "hysteria", "[Hysteria客户端] 认证成功"),
            Err(e) => error!(target: "hysteria", "[Hysteria客户端] 认证失败: {}", e),
        }

        result?;

        info!(target: "hysteria", "[Hysteria客户端] Handler创建完成");
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