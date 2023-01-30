use std::{
    net::SocketAddr,
    task::{ready, Context, Poll},
    time::Duration,
};

use futures::{future::BoxFuture, FutureExt};
use snafu::{ResultExt, Snafu};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpSocket, TcpStream},
    sync::oneshot,
    time::sleep,
};
use tower::Service;

use vector_config::configurable_component;

use crate::{
    dns,
    internal_events::{TcpSocketConnectionEstablished, TcpSocketOutgoingConnectionError},
    sinks::{util::retries::ExponentialBackoff, Healthcheck},
};

use super::HostAndPort;

#[derive(Debug, Snafu)]
pub enum TcpError {
    #[snafu(display("Address was invalid: {}", reason))]
    InvalidAddress { reason: String },

    #[snafu(display("Failed to configure TCP socket: {}.", source))]
    FailedToConfigure { source: std::io::Error },

    #[snafu(display("Failed to send TCP message: {}", source))]
    FailedToSend { source: std::io::Error },

    #[snafu(display("Failed to connect to TCP endpoint: {}", source))]
    FailedToConnect { source: std::io::Error },

    #[snafu(display("No addresses returned."))]
    NoAddresses,

    #[snafu(display("Failed to resolve address: {}", source))]
    FailedToResolve { source: crate::dns::DnsError },

    #[snafu(display("Failed to get TCP stream back after send as channel closed unexpectedly."))]
    ServiceStreamChannelClosed,
}

/// `TcpConnector` configuration.
#[configurable_component]
#[derive(Clone, Debug)]
pub struct TcpConnectorConfig {
    /// The address to connect to.
    ///
    /// The address _must_ include a port.
    address: HostAndPort,

    /// The size of the socket's send buffer, in bytes.
    ///
    /// If set, the value of the setting is passed via the `SO_SNDBUF` option.
    send_buffer_size: Option<u32>,
}

impl TcpConnectorConfig {
    pub const fn from_address(host: String, port: u16) -> Self {
        Self {
            address: HostAndPort { host, port },
            send_buffer_size: None,
        }
    }

    pub fn as_connector(&self) -> TcpConnector {
        TcpConnector {
            address: self.address.clone(),
            send_buffer_size: self.send_buffer_size,
        }
    }
}

#[derive(Clone)]
pub struct TcpConnector {
    address: HostAndPort,
    send_buffer_size: Option<u32>,
}

impl TcpConnector {
    async fn connect(&self) -> Result<(SocketAddr, TcpStream), TcpError> {
        let ip = dns::Resolver
            .lookup_ip(self.address.host.clone())
            .await
            .context(FailedToResolveSnafu)?
            .next()
            .ok_or(TcpError::NoAddresses)?;

        let addr = SocketAddr::new(ip, self.address.port);

        let socket = if addr.is_ipv4() {
            TcpSocket::new_v4().context(FailedToConfigureSnafu)?
        } else {
            TcpSocket::new_v6().context(FailedToConfigureSnafu)?
        };

        if let Some(send_buffer_size) = self.send_buffer_size {
            if let Err(error) = socket.set_send_buffer_size(send_buffer_size) {
                warn!(%error, "Failed configuring send buffer size on TCP socket.");
            }
        }

        let stream = socket.connect(addr).await.context(FailedToConnectSnafu)?;

        Ok((addr, stream))
    }

    async fn connect_backoff(&self) -> TcpStream {
        // TODO: Make this configurable.
        let mut backoff = ExponentialBackoff::from_millis(2)
            .factor(250)
            .max_delay(Duration::from_secs(60));

        loop {
            match self.connect().await {
                Ok((addr, stream)) => {
                    emit!(TcpSocketConnectionEstablished {
                        peer_addr: Some(addr)
                    });
                    return stream;
                }
                Err(error) => {
                    emit!(TcpSocketOutgoingConnectionError { error });
                    sleep(backoff.next().unwrap()).await;
                }
            }
        }
    }

    /// Gets a `Healthcheck` based on the configured destination of this connector.
    pub fn healthcheck(&self) -> Healthcheck {
        let connector = self.clone();
        Box::pin(async move { connector.connect().await.map(|_| ()).map_err(Into::into) })
    }

    /// Gets a `Service` suitable for sending data to the configured destination of this connector.
    pub fn service(&self) -> TcpService {
        TcpService::new(self.clone())
    }
}

enum TcpServiceState {
    /// The service is currently disconnected.
    Disconnected,

    /// The service is currently attempting to connect to the endpoint.
    Connecting(BoxFuture<'static, TcpStream>),

    /// The service is connected and idle.
    Connected(TcpStream),

    /// The service has an in-flight send to the stream.
    ///
    /// If the stream experiences an unrecoverable error during the send, `None` will be returned
    /// over the channel to signal the need to establish a new connection rather than reusing the
    /// existing connection.
    Sending(oneshot::Receiver<Option<TcpStream>>),
}

pub struct TcpService {
    connector: TcpConnector,
    state: TcpServiceState,
}

impl TcpService {
    const fn new(connector: TcpConnector) -> Self {
        Self {
            connector,
            state: TcpServiceState::Disconnected,
        }
    }
}

impl Service<Vec<u8>> for TcpService {
    type Response = usize;
    type Error = TcpError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        loop {
            self.state = match &mut self.state {
                TcpServiceState::Disconnected => {
                    let connector = self.connector.clone();
                    TcpServiceState::Connecting(Box::pin(async move {
                        connector.connect_backoff().await
                    }))
                }
                TcpServiceState::Connecting(fut) => {
                    let stream = ready!(fut.poll_unpin(cx));
                    TcpServiceState::Connected(stream)
                }
                TcpServiceState::Connected(_) => break,
                TcpServiceState::Sending(fut) => {
                    match ready!(fut.poll_unpin(cx)) {
                        // When a send concludes, and there's an error, the request future sends
                        // back `None`. Otherwise, it'll send back `Some(...)` with the stream.
                        Ok(maybe_stream) => match maybe_stream {
                            Some(stream) => TcpServiceState::Connected(stream),
                            None => TcpServiceState::Disconnected,
                        },
                        Err(_) => return Poll::Ready(Err(TcpError::ServiceStreamChannelClosed)),
                    }
                }
            };
        }
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, buf: Vec<u8>) -> Self::Future {
        let (tx, rx) = oneshot::channel();

        let mut stream = match std::mem::replace(&mut self.state, TcpServiceState::Sending(rx)) {
            TcpServiceState::Connected(stream) => stream,
            _ => panic!("poll_ready must be called first"),
        };

        Box::pin(async move {
            let buf_len = buf.len();

            match stream.write_all(&buf).await.context(FailedToSendSnafu) {
                Ok(_) => {
                    // Send the stream back to the service.
                    let _ = tx.send(Some(stream));

                    Ok(buf_len)
                }
                Err(e) => {
                    // We need to signal back to the service that it needs to create a fresh stream
                    // since this one could be tainted.
                    let _ = tx.send(None);

                    Err(e)
                }
            }
        })
    }
}