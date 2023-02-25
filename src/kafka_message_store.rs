use crate::data::Message;
use bytes::BytesMut;
use prost;
use rskafka::{
    client::{
        error::Error,
        partition::{Compression, PartitionClient},
        ClientBuilder,
    },
    record::Record,
    time::OffsetDateTime,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Debug;
use tracing::info;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KafkaConfig {
    pub bootstrap_servers: String,
    pub create_topic_if_not_exists: bool,
    pub topic_name: String,
    pub topic_partitions_num: i32,
    pub replication_factor: i16,
    pub timeout_ms: i32,
}

pub struct KafkaConfigBuilder {
    pub bootstrap_servers: String,
    pub create_topic_if_not_exists: bool,
    pub topic_name: String,
    pub topic_partitions_num: i32,
    pub replication_factor: i16,
    pub timeout_ms: i32,
}

impl Default for KafkaConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl KafkaConfigBuilder {
    pub fn new() -> KafkaConfigBuilder {
        KafkaConfigBuilder {
            bootstrap_servers: String::from("localhost:9092"),
            create_topic_if_not_exists: true,
            topic_name: String::from("incoming_messages"),
            topic_partitions_num: 2,
            replication_factor: 1,
            timeout_ms: 1000,
        }
    }

    pub fn build(self) -> KafkaConfig {
        KafkaConfig {
            bootstrap_servers: self.bootstrap_servers,
            create_topic_if_not_exists: self.create_topic_if_not_exists,
            topic_name: self.topic_name,
            topic_partitions_num: self.topic_partitions_num,
            replication_factor: self.replication_factor,
            timeout_ms: self.timeout_ms,
        }
    }

    pub fn bootstrap_servers(mut self, bootstrap_servers: String) -> KafkaConfigBuilder {
        self.bootstrap_servers = bootstrap_servers;
        self
    }

    pub fn create_topic_if_not_exists(
        mut self,
        create_topic_if_not_exists: bool,
    ) -> KafkaConfigBuilder {
        self.create_topic_if_not_exists = create_topic_if_not_exists;
        self
    }

    pub fn topic_name(mut self, topic_name: String) -> KafkaConfigBuilder {
        self.topic_name = topic_name;
        self
    }

    pub fn topic_partitions_num(mut self, topic_partitions_num: i32) -> KafkaConfigBuilder {
        self.topic_partitions_num = topic_partitions_num;
        self
    }

    pub fn replication_factor(mut self, replication_factor: i16) -> KafkaConfigBuilder {
        self.replication_factor = replication_factor;
        self
    }

    pub fn timeout_ms(mut self, timeout_ms: i32) -> KafkaConfigBuilder {
        self.timeout_ms = timeout_ms;
        self
    }
}

pub struct KafkaMessageStore {
    partition_client: PartitionClient,
}

impl KafkaMessageStore {
    pub async fn new(config: KafkaConfig) -> Self {
        let client = ClientBuilder::new(vec![config.bootstrap_servers])
            .build()
            .await
            .unwrap();
        if config.create_topic_if_not_exists {
            let topic = client
                .list_topics()
                .await
                .expect("Failed to list topics")
                .into_iter()
                .find(|t| t.name == config.topic_name);

            match topic {
                Some(t) => info!("KafkaMessageStore [topic={:?}] started", t),
                None => {
                    client
                        .controller_client()
                        .unwrap()
                        .create_topic(
                            config.topic_name.as_str(),
                            config.topic_partitions_num,
                            config.replication_factor,
                            config.timeout_ms,
                        )
                        .await
                        .unwrap();
                    info!(
                        "Topic {:?} with {:?} partitions created",
                        config.topic_name, config.topic_partitions_num
                    );
                }
            };
        }

        Self {
            partition_client: client
                .partition_client(config.topic_name.as_str(), 0)
                .unwrap(),
        }
    }

    pub async fn persist(&self, msg: &Message) -> Result<(), Error> {
        self.partition_client
            .produce(vec![into_record(msg)], Compression::default())
            .await
            .map(|_| ())
    }
}

fn into_record(msg: &Message) -> Record {
    let mut buf = BytesMut::new();
    prost::Message::encode(msg, &mut buf)
        .unwrap_or_else(|_| panic!("Protubuf encoding failed for {msg:?}"));

    Record {
        key: None,
        value: Some(buf.to_vec()),
        headers: BTreeMap::from([("foo".to_owned(), b"bar".to_vec())]),
        timestamp: OffsetDateTime::now_utc(),
    }
}
