use std::sync::Arc;

use smpp2kafka::{
    account::FileBasedAccountService,
    kafka_message_store::{KafkaConfigBuilder, KafkaMessageStore},
    server::{self, ServerConfigBuilder},
};
use tokio::signal;

#[tokio::main]
async fn main() {
    tracing::subscriber::set_global_default(tracing_subscriber::FmtSubscriber::new()).unwrap();

    let message_store =
        Arc::new(KafkaMessageStore::new(KafkaConfigBuilder::default().build()).await);
    let server_config = Arc::new(ServerConfigBuilder::default().build());
    let account_service = Arc::new(FileBasedAccountService::new(String::from(
        "/Users/rnowak/Projects/rust/smpp2kafka/tests/accounts.json",
    )));

    let _ = server::run(server_config, account_service, message_store, signal::ctrl_c()).await;
}
