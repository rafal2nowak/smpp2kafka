use futures::SinkExt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;
use tokio::task::{self, JoinHandle};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::{interval_at, sleep};
use tokio::time::{Duration, Instant};

use crate::account::Account;
use crate::data;
use crate::kafka_message_store::KafkaMessageStore;
use crate::shutdown::Shutdown;
use crate::{account::AccountService, server::ServerConfig};
use smpp_codec::codec::{CodecError as SmppCodecError, PduCodec};
use smpp_codec::pdu::{
    Address, Bind, BindResp, Body, Header, Pdu, SmData, CMD_BIND_TRANSMITTER, CMD_ENQUIRE_LINK,
    ESME_RALYBND, ESME_RBINDFAIL, ESME_RINVBNDSTS, ESME_RINVDCS, ESME_ROK, PDU_CMD_ID_RESP_MASK,
    PDU_HEADER_LENGTH,
};

const UNBOUND: &str = "UNBOUND";

#[derive(Debug, Clone)]
enum Event {
    Received(Pdu),
    ToSend(Pdu),
    Error(Error),
    Shutdown,
}

impl From<SmppCodecError> for Event {
    fn from(err: SmppCodecError) -> Self {
        Self::Error(Error::from(err))
    }
}

impl From<SmppCodecError> for Error {
    fn from(err: SmppCodecError) -> Self {
        match err {
            SmppCodecError::IoError(err_str) => Error::Io(err_str),
            parse_err => Error::InvalidPdu(parse_err),
        }
    }
}

#[derive(Debug, Clone)]
enum Error {
    EnquireLinkWithoutResp(i32),
    InvalidBindRequest(BindType), //ESME requested bind with type different than was provisioned
    BindTimeout,
    Io(String),
    InvalidPdu(SmppCodecError),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum BindType {
    Transmitter,
    Transceiver,
    Receiver,
}

impl BindType {
    fn from_cmd_id(command_id: u32) -> Self {
        match command_id {
            CMD_BIND_TRANSMITTER => BindType::Transmitter,
            CMD_BIND_TRANSCEIVER => BindType::Transceiver,
            CMD_BIND_RECEIVER => BindType::Receiver,
        }
    }
    fn to_string(bind_type: &BindType) -> String {
        match bind_type {
            BindType::Transmitter => String::from("TR"),
            BindType::Transceiver => String::from("TRX"),
            BindType::Receiver => String::from("RX"),
        }
    }
}

struct BindHealthCheck {
    addr: String,
    session_id: String,
    el_pending_count: Arc<AtomicI32>,
    el_task: Option<Arc<Mutex<JoinHandle<()>>>>,
    config: Arc<ServerConfig>,
}

impl BindHealthCheck {
    fn new(addr: String, session_id: String, config: Arc<ServerConfig>) -> Self {
        Self {
            addr,
            session_id,
            el_pending_count: Arc::new(AtomicI32::new(0)),
            el_task: None,
            config,
        }
    }

    fn start(&mut self, tx: UnboundedSender<Event>, addr: String, session_id: String) {
        self.session_id = session_id.clone();
        let el_pdu = |seq: u32| -> Pdu {
            Pdu::new(
                Header::new(
                    PDU_HEADER_LENGTH.try_into().unwrap(),
                    CMD_ENQUIRE_LINK,
                    0,
                    seq,
                ),
                Body::EnquireLink,
            )
        };
        let error = Error::EnquireLinkWithoutResp;

        let el_pending_count = self.el_pending_count.clone();
        let el_pending_max = self.config.enquire_link_pending_max;
        let el_interval_sec = self.config.enquire_link_interval_sec.try_into().unwrap();
        let el_task = task::spawn(async move {
            let mut sequence_number: u32 = 0;
            let mut interval = interval_at(
                Instant::now() + Duration::from_secs(el_interval_sec),
                Duration::from_secs(el_interval_sec),
            );

            loop {
                interval.tick().await;
                sequence_number += 1;

                let e = if el_pending_count.fetch_add(1, Ordering::SeqCst) <= (el_pending_max + 1) {
                    Event::ToSend(el_pdu(sequence_number))
                } else {
                    Event::Error(error(el_pending_count.load(Ordering::SeqCst)))
                };
                send_to_pdu_to_be_send_channel(addr.clone(), session_id.clone(), tx.clone(), e);
            }
        });
        self.el_task = Some(Arc::new(Mutex::new(el_task)));
    }

    fn stop(&mut self) {
        debug!(
            "[{}|{}] stopping bind healtcheck task",
            self.addr, self.session_id
        );
        if let Some(h) = self.el_task.as_ref() {
            h.lock().unwrap().abort()
        }
    }
}

pub struct Session {
    addr: String,
    session_id: String,
    bind_type: Option<BindType>,
    account: Option<Account>,
    bind_healtcheck: BindHealthCheck,
    pdu_to_be_send: (UnboundedSender<Event>, UnboundedReceiver<Event>),
    account_service: Arc<dyn AccountService>,
    message_store: Arc<KafkaMessageStore>,
    config: Arc<ServerConfig>,
    bind_timout_task: Option<JoinHandle<()>>,
    shutdown: Shutdown,
    _notify_shutdown: broadcast::Sender<()>,
}

impl Session {
    pub fn new(
        addr: String,
        account_service: Arc<dyn AccountService>,
        message_store: Arc<KafkaMessageStore>,
        config: Arc<ServerConfig>,
        shutdown: Shutdown,
        _notify_shutdown: broadcast::Sender<()>,
    ) -> Self {
        let addr_cloned = addr.clone();
        Self {
            addr,
            session_id: String::from(UNBOUND),
            bind_type: None,
            account: None,
            bind_healtcheck: BindHealthCheck::new(
                addr_cloned,
                String::from(UNBOUND),
                config.clone(),
            ),
            pdu_to_be_send: mpsc::unbounded_channel(),
            account_service,
            message_store,
            config,
            bind_timout_task: None,
            shutdown,
            _notify_shutdown,
        }
    }

    pub async fn run(&mut self, socket: TcpStream) {
        self.start_bind_timeout_task();
        let mut pdu_frames = Framed::new(socket, PduCodec::default());

        let to_event = |r: Option<Result<Pdu, SmppCodecError>>| {
            r.map(|x| match x {
                Ok(pdu) => Event::Received(pdu),
                Err(e) => e.into(),
            })
        };

        loop {
            if let Some(event) = tokio::select! {
                r = pdu_frames.next() => to_event(r),
                r = self.pdu_to_be_send.1.recv() => r,
                _ = self.shutdown.recv() => Some(Event::Shutdown),
            } {
                match event {
                    Event::Received(pdu) => {
                        if let Some(pdu_resp) = self.process_received_pdu(&pdu).await {
                            if (pdu_frames.send(pdu_resp).await).is_err() {
                                break;
                            }
                        }
                    }
                    Event::ToSend(pdu) => {
                        if (pdu_frames.send(pdu).await).is_err() {
                            break;
                        }
                    }
                    Event::Error(e) => {
                        error!(
                            "[{}|{}] {:?}, closing connection",
                            self.addr, self.session_id, e
                        );
                        break;
                    }
                    Event::Shutdown => break,
                }
            }
        }
        self.bind_healtcheck.stop();
        let _ = pdu_frames.close().await;

        info!("[{}|{}] Session closed", self.addr, self.session_id);
    }

    async fn process_received_pdu(&mut self, pdu: &Pdu) -> Option<Pdu> {
        match &pdu.body {
            Body::Bind(bind) => Some(self.on_bind(&pdu.header, bind)),

            Body::EnquireLink => Some(self.on_enquire_link(&pdu.header)),
            Body::EnquireLinkResp => {
                self.on_enquire_link_resp();
                None
            }
            Body::SubmitSm(sm_data) => Some(self.on_submit_sm(&pdu.header, sm_data).await),

            Body::DeliverSmResp(_) => {
                None // TODO: implement
            }
            Body::Unbind => todo!(),
            Body::UnbindResp => todo!(),
            Body::Nack => todo!(),
            _ => {
                warn!(
                    "[{}|{}] dropping {:?} sent by ESME ",
                    self.addr, self.session_id, pdu
                );
                None
            }
        }
    }

    fn on_enquire_link(&mut self, header: &Header) -> Pdu {
        self.bind_healtcheck
            .el_pending_count
            .fetch_add(1, Ordering::SeqCst);
        Pdu::new(
            Header::new(
                (PDU_HEADER_LENGTH) as u32,
                header.command_id | PDU_CMD_ID_RESP_MASK,
                ESME_ROK,
                header.sequence_number,
            ),
            Body::EnquireLinkResp,
        )
    }

    fn on_enquire_link_resp(&mut self) {
        self.bind_healtcheck
            .el_pending_count
            .fetch_sub(1, Ordering::SeqCst);
    }

    async fn on_submit_sm(&mut self, header: &Header, sm_data: &SmData) -> Pdu {
        match (&self.bind_type, self.submit_sm_to_message(sm_data)) {
            (Some(_), Ok(message)) => match self.message_store.persist(&message).await {
                Ok(_) => Pdu::new(
                    Header::new(
                        (message.id.len() + 1 + PDU_HEADER_LENGTH) as u32,
                        header.command_id | PDU_CMD_ID_RESP_MASK,
                        ESME_ROK,
                        header.sequence_number,
                    ),
                    Body::SubmitSmResp(message.id),
                ),
                Err(_) => {
                    //Err(err_code.try_into().unwrap_or(self.config.default_err_code))
                    todo!()
                }
            },
            (Some(_), Err(_)) => todo!(), // datacoding issue
            (None, _) =>
            // FIXME: it shoud be submitsmrep with ESME_RINVBNDSTS
            {
                header.create_nack(ESME_RINVBNDSTS)
            } // Incorrect BIND Status for given command,
        }
    }

    fn on_bind(&mut self, header: &Header, bind: &Bind) -> Pdu {
        let response_pdu = |command_status: u32| {
            Pdu::new(
                Header::new(
                    (bind.system_id.len() + 1 + PDU_HEADER_LENGTH) as u32,
                    header.command_id | PDU_CMD_ID_RESP_MASK,
                    command_status,
                    header.sequence_number,
                ),
                Body::BindResp(BindResp::new(bind.system_id.to_owned(), None)),
            )
        };

        if self.bind_type.is_some() {
            return response_pdu(ESME_RALYBND); // ESME Already in Bound State
        }

        match self.account_service.find(&bind.system_id) {
            Err(_) => todo!(),
            Ok(None) => response_pdu(ESME_RBINDFAIL), // account not found
            Ok(Some(account)) => {
                trace!("found {:?}", account);
                let bind_type = BindType::from_cmd_id(header.command_id); //IncorrectBindRequest
                if !account.bind_type.eq(&bind_type) {
                    send_to_pdu_to_be_send_channel(
                        self.addr.clone(),
                        self.session_id.clone(),
                        self.pdu_to_be_send.0.clone(),
                        Event::Error(Error::InvalidBindRequest(bind_type)),
                    );
                    response_pdu(ESME_RBINDFAIL) // Binding failed
                } else if account.password != bind.password {
                    response_pdu(ESME_RBINDFAIL) // Auth failed
                } else {
                    self.abort_bind_timeout_task();
                    self.bind_type = Some(bind_type);
                    self.account = Some(account);
                    self.session_id = create_session_id(
                        self.account.as_ref().unwrap(),
                        self.bind_type.as_ref().unwrap(),
                    );
                    self.bind_healtcheck.start(
                        self.pdu_to_be_send.0.clone(),
                        self.addr.clone(),
                        self.session_id.clone(),
                    );
                    info!("[{}|{}] Session created", self.addr, self.session_id);
                    response_pdu(ESME_ROK)
                }
            }
        }
    }

    fn submit_sm_to_message(&self, sm_data: &SmData) -> Result<data::Message, u32> {
        let mut msg = data::Message::default();
        if let Some(charset) = self.config.dc_to_charset(sm_data.data_coding) {
            msg.set_charset(charset);
        } else {
            return Err(ESME_RINVDCS);
        }
        msg.id = Uuid::new_v4().to_string();
        msg.dest = Some(data::Address::from(&sm_data.dest_addr));
        msg.src = Some(data::Address::from(&sm_data.source_addr));
        msg.text = sm_data.short_message.to_vec();
        Ok(msg)
    }

    fn start_bind_timeout_task(&mut self) {
        let session_id = self.session_id.clone();
        let addr = self.addr.clone();
        let bind_timeout_sec = self.config.bind_timeout_sec;
        let tx = self.pdu_to_be_send.0.clone();
        let task = tokio::spawn(async move {
            sleep(Duration::from_secs(bind_timeout_sec)).await;
            send_to_pdu_to_be_send_channel(addr, session_id, tx, Event::Error(Error::BindTimeout));
        });
        self.bind_timout_task = Some(task);
    }

    fn abort_bind_timeout_task(&self) {
        if let Some(bind_timout_task) = &self.bind_timout_task {
            bind_timout_task.abort();
        }
    }
}

fn create_session_id(account: &Account, bind_type: &BindType) -> String {
    let mut session_id = BindType::to_string(bind_type).to_uppercase();
    session_id.push('@');
    session_id.push_str(&account.system_id.clone());
    session_id
}

impl From<&Address> for data::Address {
    fn from(addr: &Address) -> Self {
        data::Address {
            npi: addr.npi.into(),
            ton: addr.ton.into(),
            value: addr.value.clone(),
        }
    }
}

fn send_to_pdu_to_be_send_channel<T: std::fmt::Debug + Clone>(
    addr: String,
    session_id: String,
    tx: UnboundedSender<T>,
    msg: T,
) {
    if let Err(e) = tx.send(msg.clone()) {
        error!(
            "[{}|{}] failed to send {:?} to unbounded channel, {:?}",
            addr, session_id, msg, e
        )
    }
}
