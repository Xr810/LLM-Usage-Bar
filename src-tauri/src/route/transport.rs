//! Cancel HTTP handlers on peer close even when an unread request body has
//! backpressured Hyper. A held handler must not read that body to notice EOF.
use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use axum::serve::Listener;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use tokio::{
    io::{AsyncRead, ReadBuf},
    net::{tcp::OwnedReadHalf, TcpListener},
};

struct SharedRead(Arc<OwnedReadHalf>);

impl AsyncRead for SharedRead {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            std::task::ready!(self.0.as_ref().as_ref().poll_read_ready(cx))?;
            match self.0.try_read(buf.initialize_unfilled()) {
                Ok(count) => {
                    buf.advance(count);
                    return Poll::Ready(Ok(()));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
    }
}

async fn peer_closed(read: &OwnedReadHalf) {
    loop {
        match read.ready(tokio::io::Interest::READABLE).await {
            Ok(ready) if ready.is_read_closed() || ready.is_error() => return,
            Err(_) => return,
            _ => {}
        }
        // Non-consuming EOF probe for platforms that report EOF as readable.
        // Never poll the HTTP body or consume bytes before a Hold is released.
        match read.as_ref().peek(&mut [0]).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        // Unread body bytes are also readable; don't busy-loop on that level.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

pub(super) async fn serve(mut listener: TcpListener, router: axum::Router) -> io::Result<()> {
    loop {
        let (socket, _) = Listener::accept(&mut listener).await;
        let (read, write) = socket.into_split();
        let read = Arc::new(read);
        let io = tokio::io::join(SharedRead(read.clone()), write);
        let service = TowerToHyperService::new(router.clone());
        tokio::spawn(async move {
            // Keep axum::serve's protocol negotiation and upgrade support.
            let builder = Builder::new(TokioExecutor::new());
            let connection = builder.serve_connection_with_upgrades(TokioIo::new(io), service);
            tokio::select! {
                result = connection => {
                    if let Err(error) = result {
                        log::debug!("router connection ended: {error}");
                    }
                }
                _ = peer_closed(&read) => {
                    // Drop the entire connection, including its service future.
                    // PendingDisconnectAudit runs before PendingTicket dequeues
                    // and publishes the new snapshot. No HTTP 499 is written.
                }
            }
        });
    }
}
