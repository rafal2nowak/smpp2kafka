pub mod account;
pub mod data;
pub mod kafka_message_store;
pub mod server;
pub mod session;
pub mod shutdown;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub type Result<T> = std::result::Result<T, Error>;