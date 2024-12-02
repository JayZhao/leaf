use std::sync::Arc;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use anyhow::Result;
use std::{
    pin::Pin,
    task::{Context, Poll},
    io,
};

use crate::{proxy::*, session::Session};
use ::hysteria::{Config, HysteriaClient, quinn};

pub struct Handler {
    client: Arc<HysteriaClient>,
}

// 实现一个包装 QUIC streams 的 ProxyStream
struct HysteriaStream {
    send: quinn::SendStream, // 发送流
    recv: quinn::RecvStream, // 接收流
}

impl AsyncRead for HysteriaStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // 从接收流中读取数据
        println!("开始从接收流中读取数据");
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

impl AsyncWrite for HysteriaStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        // 向发送流中写入数据
        println!("开始向发送流中写入数据");
        match Pin::new(&mut self.send).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                println!("成功写入 {} 字节", n);
                Poll::Ready(Ok(n))
            },
            Poll::Ready(Err(e)) => {
                println!("写入失败: {}", e);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            },
            Poll::Pending => {
                println!("写入操作挂起");
                Poll::Pending
            },
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        // 刷新发送流
        println!("开始刷新发送流");
        match Pin::new(&mut self.send).poll_flush(cx) {
            Poll::Ready(Ok(())) => {
                println!("刷新成功");
                Poll::Ready(Ok(()))
            },
            Poll::Ready(Err(e)) => {
                println!("刷新失败: {}", e);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            },
            Poll::Pending => {
                println!("刷新操作挂起");
                Poll::Pending
            },
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        // 关闭发送流
        println!("开始关闭发送流");
        match Pin::new(&mut self.send).poll_shutdown(cx) {
            Poll::Ready(Ok(())) => {
                println!("关闭成功");
                Poll::Ready(Ok(()))
            },
            Poll::Ready(Err(e)) => {
                println!("关闭失败: {}", e);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            },
            Poll::Pending => {
                println!("关闭操作挂起");
                Poll::Pending
            },
        }
    }
}

#[async_trait]
impl OutboundStreamHandler for Handler {
    fn connect_addr(&self) -> OutboundConnect {
        // 作为终端协议，使用 Unknown 表示自己处理连接
        OutboundConnect::Unknown
    }

    async fn handle<'a>(
        &'a self,
        sess: &'a Session,
        _lhs: Option<&mut AnyStream>,
        _stream: Option<AnyStream>,
    ) -> io::Result<AnyStream> {
        // 获取目标地址字符串
        let dest = sess.destination.to_string();
        
        // 使用 hysteria client 建立 TCP 连接
        let (send, recv) = self.client
            .tcp_connect(&dest)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("connect failed: {}", e)))?;

        // 包装成 HysteriaStream
        Ok(Box::new(HysteriaStream { send, recv }))
    }
}

impl Handler {
    pub fn new(
        server: String,
        auth: String,
        server_name: String,
    ) -> Result<Self> {
        let config = Config::new(
            server.clone(),
            auth,
            server_name,
        );
        
        let mut client = HysteriaClient::new(config);
        
        // 在创建时就进行连接和认证
        tokio::runtime::Handle::current().block_on(async {
            client.connect_and_authenticate().await?;
            Ok::<_, anyhow::Error>(())
        })?;

        Ok(Handler {
            client: Arc::new(client)
        })
    }
}


impl Drop for Handler {
    fn drop(&mut self) {
        // 确保资源正确清理
        if Arc::strong_count(&self.client) == 1 {
            tokio::runtime::Handle::current().block_on(async {
                Arc::get_mut(&mut self.client).unwrap().shutdown().await;
            });
        }
    }
}