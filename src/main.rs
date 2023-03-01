use std::sync::Arc;

use smpp2kafka::{
    account::FileBasedAccountService,
    kafka_message_store::{KafkaConfigBuilder, KafkaMessageStore},
    server::{self, ServerConfigBuilder, ACCOUNTS_FILE_NAME},
};
use tokio::signal;

#[tokio::main]
async fn main() {
    tracing::subscriber::set_global_default(tracing_subscriber::FmtSubscriber::new()).unwrap();
    let accounts_file = std::env::args()
        .nth(1)
        .get_or_insert(String::from(ACCOUNTS_FILE_NAME))
        .to_string();
    let message_store =
        Arc::new(KafkaMessageStore::new(KafkaConfigBuilder::default().build()).await);
    let server_config = Arc::new(ServerConfigBuilder::default().build());
    let account_service = Arc::new(FileBasedAccountService::new(accounts_file));

    let _ = server::run(
        server_config,
        account_service,
        message_store,
        signal::ctrl_c(),
    )
    .await;
}
