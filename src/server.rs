use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::account::AccountService;
use crate::kafka_message_store::KafkaMessageStore;
use crate::shutdown::Shutdown;
use crate::{data, session};
use futures::Future;
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::Semaphore;
use tokio::time;
use tracing::info;

pub const ACCOUNTS_FILE_NAME: &str = "accounts.json";

pub async fn run(
    config: Arc<ServerConfig>,
    account_service: Arc<dyn AccountService>,
    message_store: Arc<KafkaMessageStore>,
    shutdown: impl Future + Send + 'static,
    shutdown_complete: Sender<()>,
) -> crate::Result<()> {
    let (notify_shutdown, _) = broadcast::channel::<()>(1);
    let mut server = Server::new(config, account_service, message_store, notify_shutdown);
    tokio::select! {
        _ = server.run(shutdown_complete.clone()) => {}
        _ = shutdown => info!("Shutting down")
    }

    let Server {
        config: _,
        account_service: _,
        message_store: _,
        connections_limit: _,
        notify_shutdown,
        listener: _,
    } = server;

    //TODO: 
    //drop(notify_shutdown);
    //drop(shutdown_complete);

    info!("Shutdown completed");
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Listener {
    pub host: String,
    pub port: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ServerConfig {
    dc_to_charset_map: HashMap<u8, String>,
    max_connections: u32,
    pub listener: Listener,
    pub enquire_link_interval_sec: u32,
    pub enquire_link_pending_max: i32,
    pub default_err_code: u32,
    pub bind_timeout_sec: u64,
}

impl ServerConfig {
    pub fn dc_to_charset(&self, dc: u8) -> Option<data::Charset> {
        let str_to_charset = |str: &str| match str {
            "ISO_8859_1" => Some(data::Charset::Iso88591),
            "ISO_8859_15" => Some(data::Charset::Iso885915),
            "GSM7" => Some(data::Charset::Gsm7),
            "GSM8" => Some(data::Charset::Gsm8),
            "UTF8" => Some(data::Charset::Gsm8),
            "UCS2" => Some(data::Charset::Ucs2),
            _ => None,
        };
        self.dc_to_charset_map
            .get(&dc)
            .and_then(|s| str_to_charset(s))
    }
}

pub struct ServerConfigBuilder {
    host: String,
    port: i32,
    dc_to_charset_map: HashMap<u8, String>,
    max_connections: u32,
    enquire_link_interval_sec: u32,
    enquire_link_pending_max: i32,
    default_err_code: u32,
    bind_timeout_sec: u64,
}

impl Default for ServerConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerConfigBuilder {
    pub fn new() -> ServerConfigBuilder {
        ServerConfigBuilder {
            host: String::from("127.0.0.1"),
            port: 2776,
            dc_to_charset_map: HashMap::from([
                (0, String::from("GSM7")),
                (1, String::from("ISO_8859_1")),
                (2, String::from("ISO_8859_15")),
                (3, String::from("GSM8")),
                (4, String::from("UTF8")),
                (8, String::from("UCS2")),
            ]),
            max_connections: 10,
            enquire_link_interval_sec: 5,
            enquire_link_pending_max: 3,
            default_err_code: 1024,
            bind_timeout_sec: 5,
        }
    }

    pub fn build(self) -> ServerConfig {
        ServerConfig {
            dc_to_charset_map: self.dc_to_charset_map,
            max_connections: self.max_connections,
            listener: Listener {
                host: self.host,
                port: self.port,
            },
            enquire_link_interval_sec: self.enquire_link_interval_sec,
            enquire_link_pending_max: self.enquire_link_pending_max,
            default_err_code: self.default_err_code,
            bind_timeout_sec: self.bind_timeout_sec,
        }
    }

    pub fn host(mut self, host: String) -> ServerConfigBuilder {
        self.host = host;
        self
    }

    pub fn port(mut self, port: i32) -> ServerConfigBuilder {
        self.port = port;
        self
    }

    pub fn max_connections(mut self, max_connections: u32) -> ServerConfigBuilder {
        self.max_connections = max_connections;
        self
    }

    pub fn enquire_link_interval_sec(
        mut self,
        enquire_link_interval_sec: u32,
    ) -> ServerConfigBuilder {
        self.enquire_link_interval_sec = enquire_link_interval_sec;
        self
    }

    pub fn enquire_link_pending_max(
        mut self,
        enquire_link_pending_max: i32,
    ) -> ServerConfigBuilder {
        self.enquire_link_pending_max = enquire_link_pending_max;
        self
    }

    pub fn default_err_code(mut self, default_err_code: u32) -> ServerConfigBuilder {
        self.default_err_code = default_err_code;
        self
    }

    pub fn bind_timeout_sec(mut self, bind_timeout_sec: u64) -> ServerConfigBuilder {
        self.bind_timeout_sec = bind_timeout_sec;
        self
    }

    pub fn dc_to_charset_map(
        mut self,
        dc_to_charset_map: HashMap<u8, String>,
    ) -> ServerConfigBuilder {
        self.dc_to_charset_map = dc_to_charset_map;
        self
    }

    pub fn with_clear_dc_to_charset_map(mut self) -> ServerConfigBuilder {
        self.dc_to_charset_map.clear();
        self
    }

    pub fn with_dc_to_charset(mut self, dc: u8, charset: String) -> ServerConfigBuilder {
        self.dc_to_charset_map.insert(dc, charset);
        self
    }
}

pub struct Server {
    config: Arc<ServerConfig>,
    account_service: Arc<dyn AccountService>,
    message_store: Arc<KafkaMessageStore>,
    connections_limit: Arc<Semaphore>,
    notify_shutdown: broadcast::Sender<()>,
    listener: Option<TcpListener>,
}

impl Server {
    pub fn new(
        config: Arc<ServerConfig>,
        account_service: Arc<dyn AccountService>,
        message_store: Arc<KafkaMessageStore>,
        notify_shutdown: broadcast::Sender<()>,
    ) -> Server {
        let max_connections = config.max_connections as usize;
        Server {
            config,
            account_service,
            message_store,
            connections_limit: Arc::new(Semaphore::new(max_connections)),
            notify_shutdown,
            listener: None,
        }
    }

    pub async fn run(&mut self, shutdown_complete_tx: mpsc::Sender<()>) -> crate::Result<()> {
        self.bind_listener().await?;

        loop {
            let permit = self
                .connections_limit
                .clone()
                .acquire_owned()
                .await
                .unwrap();

            let (socket, addr) = self.accept().await?;
            info!("Connection accepted: {}", addr);

            let mut session = session::Session::new(
                addr.to_string(),
                self.account_service.clone(),
                self.message_store.clone(),
                self.config.clone(),
                Shutdown::new(self.notify_shutdown.subscribe()),
                shutdown_complete_tx.clone(),
            );
            tokio::spawn(async move {
                session.run(socket).await;
                drop(permit);
            });
        }
    }

    async fn bind_listener(&mut self) -> crate::Result<()> {
        let host = self.config.listener.host.clone();
        let port = self.config.listener.port;
        let addr = host + ":" + &port.to_string();
        let tcp_listener = TcpListener::bind(addr).await?;
        info!(
            "Listening on port: {}. Connections limit: {}",
            port, self.config.max_connections
        );
        self.listener = Some(tcp_listener);
        Ok(())
    }

    /// Method copied from: https://github.com/tokio-rs/mini-redis
    async fn accept(&mut self) -> crate::Result<(TcpStream, SocketAddr)> {
        let mut backoff = 1;

        loop {
            match self.listener.as_ref().unwrap().accept().await {
                Ok(tuple) => return Ok(tuple),
                Err(err) => {
                    if backoff > 64 {
                        return Err(err.into());
                    }
                }
            }
            time::sleep(Duration::from_secs(backoff)).await;
            backoff *= 2;
        }
    }
}
