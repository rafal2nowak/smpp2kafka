use std::time::Duration;

use crate::helper::ManualDelay;
use futures::SinkExt;
use smpp2kafka::server::ServerConfigBuilder;
use smpp_codec::pdu::{
    Address, Bind, Body, Header, Pdu, SmData, CMD_BIND_TRANSCEIVER, CMD_SUBMIT_SM, ESME_RBINDFAIL,
    ESME_ROK,
};
use tokio::time::sleep;
use tokio_stream::StreamExt;
mod helper;

const SYSTEM_ID: &str = "1234567890";
const PASSWORD: &str = "password";

#[tokio::test]
async fn shutdown_server_manually() {
    let shutdown = ManualDelay::new();
    let shutdown_completed =
        helper::start_server(ServerConfigBuilder::default(), shutdown.clone()).await;

    tokio::spawn(async {
        let mut framed = helper::connect_to_server().await;
        framed.next().await;
    });

    sleep(Duration::from_secs(1)).await;

    helper::gracefully_shutdown_server(shutdown, shutdown_completed).await;
}

#[tokio::test]
async fn bind_timeout() {
    let shutdown = ManualDelay::new();
    let shutdown_completed = helper::start_server(
        ServerConfigBuilder::default().bind_timeout_sec(1),
        shutdown.clone(),
    )
    .await;

    tokio::spawn(async {
        let mut framed = helper::connect_to_server().await;
        return framed.next().await;
    })
    .await
    .unwrap();

    helper::gracefully_shutdown_server(shutdown, shutdown_completed).await;
}

#[tokio::test]
async fn bind_transciever() {
    let shutdown = ManualDelay::new();
    let shutdown_completed =
        helper::start_server(ServerConfigBuilder::default(), shutdown.clone()).await;

    let mut framed = helper::connect_to_server().await;

    let bind = Pdu::new(
        Header::new(41, CMD_BIND_TRANSCEIVER, 0x00, 0x01),
        Body::Bind(Bind::new(
            String::from(SYSTEM_ID),
            String::from(PASSWORD),
            String::from(""),
            0x34,
            0x00,
            0x00,
            String::from(""),
        )),
    );
    framed.send(bind).await.unwrap();
    let pdu = framed
        .next()
        .await
        .expect("Expects Some(Pdu)")
        .expect("Expects Pdu");

    match pdu.body {
        Body::BindResp(bind_resp) => {
            assert_eq!(bind_resp.system_id, SYSTEM_ID);
        }
        b => panic!("Expects BindResp, got {:?}", b),
    }

    helper::gracefully_shutdown_server(shutdown, shutdown_completed).await;
}

#[tokio::test]
async fn bind_transciever_invalid_password() {
    let shutdown = ManualDelay::new();
    let shutdown_completed =
        helper::start_server(ServerConfigBuilder::default(), shutdown.clone()).await;

    let invalid_password = "abcsword";
    let mut framed = helper::connect_to_server().await;

    let bind = Pdu::new(
        Header::new(41, CMD_BIND_TRANSCEIVER, 0x00, 0x01),
        Body::Bind(Bind::new(
            String::from(SYSTEM_ID),
            String::from(invalid_password),
            String::from(""),
            0x34,
            0x00,
            0x00,
            String::from(""),
        )),
    );
    framed.send(bind).await.unwrap();
    let pdu = framed
        .next()
        .await
        .expect("Expects Some(Pdu)")
        .expect("Expects Pdu");
    match pdu.body {
        Body::BindResp(bind_resp) => {
            assert_eq!(bind_resp.system_id, SYSTEM_ID);
            assert_eq!(pdu.header.command_status, ESME_RBINDFAIL);
        }
        b => panic!("Expects BindResp, got {:?}", b),
    }
    helper::gracefully_shutdown_server(shutdown, shutdown_completed).await;
}

#[tokio::test]
async fn bind_transciever_account_not_found() {
    let shutdown = ManualDelay::new();
    let shutdown_completed =
        helper::start_server(ServerConfigBuilder::default(), shutdown.clone()).await;
    let invalid_system_id = "1004567890";
    let mut framed = helper::connect_to_server().await;

    let bind = Pdu::new(
        Header::new(41, CMD_BIND_TRANSCEIVER, 0x00, 0x01),
        Body::Bind(Bind::new(
            String::from(invalid_system_id),
            String::from(PASSWORD),
            String::from(""),
            0x34,
            0x00,
            0x00,
            String::from(""),
        )),
    );
    framed.send(bind).await.unwrap();
    let pdu = framed
        .next()
        .await
        .expect("Expects Some(Pdu)")
        .expect("Expects Pdu");

    match pdu.body {
        Body::BindResp(bind_resp) => {
            assert_eq!(bind_resp.system_id, invalid_system_id);
            assert_eq!(pdu.header.command_status, ESME_RBINDFAIL);
        }
        b => panic!("Expects BindResp, got {:?}", b),
    }

    framed.close().await.unwrap();

    helper::gracefully_shutdown_server(shutdown, shutdown_completed).await;
}

#[tokio::test]
async fn submit_sm() {
    let shutdown = ManualDelay::new();
    let shutdown_completed = helper::start_server(
        ServerConfigBuilder::default().enquire_link_interval_sec(4),
        shutdown.clone(),
    )
    .await;

    let mut framed = helper::connect_to_server().await;

    let bind = Pdu::new(
        Header::new(41, CMD_BIND_TRANSCEIVER, 0x00, 0x01),
        Body::Bind(Bind::new(
            String::from(SYSTEM_ID),
            String::from(PASSWORD),
            String::from(""),
            0x34,
            0x00,
            0x00,
            String::from(""),
        )),
    );
    framed.send(bind).await.unwrap();
    framed
        .next()
        .await
        .expect("Expects Some(Pdu)")
        .expect("Expects Pdu");

    let submit_sm = Pdu::new(
        Header::new(50, CMD_SUBMIT_SM, 0x00, 0x01),
        Body::SubmitSm(SmData {
            service_type: String::from(""),
            source_addr: Address::new(0x00, 0x01, String::from("100")),
            dest_addr: Address::new(0x01, 0x01, String::from("1234567890")),
            esm_class: 0x00,
            protocol_id: 0x00,
            priority_flag: 0x00,
            schedule_delivery_time: String::from(""),
            validity_period: String::from(""),
            registered_delivery: 0x00,
            replace_if_present_flag: 0x00,
            data_coding: 0x00,
            sm_default_msg_id: 0x00,
            sm_length: 0x04,
            short_message: helper::create_short_message(4),
            opt_params: None,
        }),
    );

    framed.send(submit_sm).await.unwrap();
    
    let resp_pdu = framed
        .next()
        .await
        .expect("Expects Some(Pdu)")
        .expect("Expects Pdu");

    match resp_pdu.body {
        Body::SubmitSmResp(message_id) => {
            assert!(message_id.len() > 0);
            assert_eq!(resp_pdu.header.command_status, ESME_ROK);
        }
        r => panic!("Expects SubmitSmResp, got {r:?}"),
    }
    helper::gracefully_shutdown_server(shutdown, shutdown_completed).await;
}
