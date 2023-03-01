use bytes::{BufMut, Bytes, BytesMut};
use futures::Future;
use smpp2kafka::account::FileBasedAccountService;
use smpp2kafka::kafka_message_store::{KafkaConfigBuilder, KafkaMessageStore};
pub use smpp2kafka::server::Server;
use smpp2kafka::server::{self, ServerConfigBuilder, ACCOUNTS_FILE_NAME};
use smpp_codec::codec::PduCodec;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex},
    task::{Poll, Waker},
    time::Duration,
};
use tokio::{
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender},
    time::sleep,
};
use tokio_util::codec::{Decoder, Framed};

const SERVER_ADDR: &str = "127.0.0.1:2776";
static TRACING_STARTED: AtomicBool = AtomicBool::new(false);

pub async fn connect_to_server() -> Framed<TcpStream, PduCodec> {
    loop {
        let socket = TcpStream::connect(SERVER_ADDR).await;
        if socket.is_err() {
            sleep(Duration::from_millis(500)).await;
        } else {
            return PduCodec::default().framed(socket.ok().unwrap());
        }
    }
}

pub async fn start_server(
    server_config_builder: ServerConfigBuilder,
    shutdown: impl Future + Send + Clone + 'static,
) -> (Sender<()>, Receiver<()>) {
    start_tracing_if_nneded();
    let shutdown_complete = mpsc::channel::<()>(1);
    let server_config = Arc::new(server_config_builder.build());
    let account_service = Arc::new(FileBasedAccountService::new(String::from(ACCOUNTS_FILE_NAME)));
    let message_store =
        Arc::new(KafkaMessageStore::new(KafkaConfigBuilder::default().build()).await);
    tokio::spawn(async move {
        let _ = server::run(server_config, account_service, message_store, shutdown).await;
    });
    shutdown_complete
}

pub async fn gracefully_shutdown_server(
    mut shutdown: ManualDelay,
    mut shutdown_completed: (Sender<()>, Receiver<()>),
) {
    shutdown.complete();
    drop(shutdown_completed.0);
    shutdown_completed.1.recv().await;
}

pub fn create_short_message(len: usize) -> Bytes {
    let mut bytes = BytesMut::with_capacity(len);
    for i in 0..len {
        bytes.put_u8((130 + i).try_into().unwrap())
    }
    bytes.freeze()
}

fn start_tracing_if_nneded() {
    match TRACING_STARTED.compare_exchange(false, true, Ordering::Relaxed, Ordering::SeqCst) {
        Ok(false) => {
            tracing::subscriber::set_global_default(tracing_subscriber::FmtSubscriber::new())
                .unwrap();
        }
        _ => (),
    }
}

fn file_path(root_path: &PathBuf, file_name: &str) -> String {
    let mut path = root_path.clone();
    path.push(file_name);
    path.to_str()
        .expect(&format!(
            "Expecting file: {file_name} in directory: {:?}",
            root_path.as_os_str()
        ))
        .to_owned()
}

#[derive(Debug, Clone)]
pub struct State {
    completed: bool,
    waker: Option<Waker>,
}

#[derive(Debug, Clone)]
pub struct ManualDelay {
    state: Arc<Mutex<State>>,
}

impl ManualDelay {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                completed: false,
                waker: None,
            })),
        }
    }

    pub fn complete(&mut self) {
        let mut state = self.state.lock().unwrap();
        state.completed = true;

        if state.waker.is_some() {
            state.waker.as_ref().unwrap().wake_by_ref();
        }
    }
}
unsafe impl Send for ManualDelay {}

impl Future for ManualDelay {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut state = self.state.lock().unwrap();

        match &state.waker {
            Some(_) => (),
            None => state.waker = Some(cx.waker().clone()),
        }

        if state.completed {
            state.waker.as_mut().unwrap().wake_by_ref();
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
