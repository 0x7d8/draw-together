mod data;

use axum::{
    body::{Body, Bytes},
    extract::{
        ws::{Message, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::HeaderMap,
    response::Response,
    routing::{any, get},
    Router,
};
use futures_util::{stream::StreamExt, SinkExt};
use std::{net::SocketAddr, path::Path, sync::Arc};
use tokio::sync::Mutex;

const INDEX_HTML: &str = include_str!("../static/index.html");
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    let nosave = std::env::args().nth(1) == Some("--nosave".to_string());
    let port = std::env::var("PORT")
        .unwrap_or("3000".to_string())
        .parse::<u16>()
        .expect("invalid port, 0-65535");

    let data = Arc::new(Mutex::new(
        data::Data::new(
            match Path::new("history_2.raw").exists() {
                true => Some("history_2.raw".to_string()),
                false => match nosave {
                    true => None,
                    false => Some("history_2.raw".to_string()),
                },
            },
            !nosave,
        )
        .await,
    ));

    let app = Router::new()
        .route(
            "/history_2.raw",
            get(|state: State<Arc<Mutex<data::Data>>>| async move {
                let data = state.lock().await;

                let mut headers = HeaderMap::new();

                headers.insert("Content-Type", "robert/history-2".parse().unwrap());

                let data = data.data.as_ref().lock().await;
                let body = Body::from(data.clone());

                (headers, body)
            }),
        )
        .route("/ws", any(handle_ws))
        .route(
            "/",
            get(|| async {
                let mut headers = HeaderMap::new();

                headers.insert("Content-Type", "text/html".parse().unwrap());

                (headers, Body::from(INDEX_HTML))
            }),
        )
        .with_state(data);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .unwrap();

    println!(
        "listening on {} (v{})",
        listener.local_addr().unwrap(),
        VERSION
    );
    if nosave {
        println!("not saving history");
    } else {
        println!("saving history to history_2.raw");
    }

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

async fn handle_ws(
    ws: WebSocketUpgrade,
    data: State<Arc<Mutex<data::Data>>>,
    ConnectInfo(who): ConnectInfo<SocketAddr>,
) -> Response {
    println!("{} connected to ws", who);

    let data = Arc::clone(&data);

    ws.on_upgrade(move |socket| async move {
        let (sender, mut reciever) = socket.split();
        let sender = Arc::new(Mutex::new(sender));

        let writer_data = Arc::clone(&data);
        let writer = tokio::spawn(async move {
            loop {
                let ws_data = reciever.next().await;
                if ws_data.is_none() {
                    break;
                }

                let ws_data = ws_data.unwrap();
                if ws_data.is_err() {
                    break;
                }

                let ws_data = ws_data.unwrap().into_data();

                let mut parsed = Vec::with_capacity(ws_data.len() / 7);
                for chunk in ws_data.chunks(7) {
                    let data = data::ClientMessage::decode(chunk);

                    if data.is_none() {
                        continue;
                    }

                    parsed.push(data.unwrap());
                }

                writer_data.lock().await.write(&parsed).await;
            }
        });

        let (send, mut recieve): (
            tokio::sync::mpsc::Sender<Vec<u8>>,
            tokio::sync::mpsc::Receiver<Vec<u8>>,
        ) = tokio::sync::mpsc::channel(7);
        data.lock().await.add_listener(send);

        let reader_sender = Arc::clone(&sender);
        let reader = tokio::spawn(async move {
            loop {
                let data = recieve.recv().await.unwrap_or_default();

                reader_sender
                    .lock()
                    .await
                    .send(Message::binary(data))
                    .await
                    .unwrap_or_default();
            }
        });

        let pinger_sender = Arc::clone(&sender);
        let pinger = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;

                let ping = pinger_sender
                    .lock()
                    .await
                    .send(Message::Ping(Bytes::from_static(&[1, 2, 3])))
                    .await;

                if ping.is_err() {
                    break;
                }
            }
        });

        writer.await.unwrap_or_default();
        pinger.await.unwrap_or_default();

        println!("{} disconnected", who);

        reader.abort();
        data.lock().await.sync_listeners();
    })
}
