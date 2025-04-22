// SPDX-License-Identifier: MPL-2.0
// This file is taken from the gst-plugin-rs project licensed under the MPL-2.0 and modified

use async_tungstenite::tungstenite::{Message as WsMessage, Utf8Bytes};
use futures::channel::mpsc;
use futures::prelude::*;
use log::{debug, error, trace, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::task;

use super::protocol::OutgoingMessage;

struct Peer {
    receive_task_handle: task::JoinHandle<()>,
    send_task_handle: task::JoinHandle<Result<(), String>>,
    sender: mpsc::Sender<String>,
}

struct State {
    tx: Option<mpsc::Sender<(String, Option<Utf8Bytes>)>>,
    peers: HashMap<String, Peer>,
}

#[derive(Clone)]
pub struct Server {
    state: Arc<Mutex<State>>,
}

pub enum ServerError {
    Handshake,
}

impl Server {
    pub fn spawn<
        I: for<'a> Deserialize<'a>,
        Factory: FnOnce(Pin<Box<dyn Stream<Item = (String, Option<I>)> + Send>>) -> St,
        St: Stream<Item = (String, OutgoingMessage)> + Send + Unpin + 'static,
    >(
        factory: Factory,
        out_peer_id: Arc<Mutex<Option<String>>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<(String, Option<Utf8Bytes>)>(1000);
        let mut handler = factory(Box::pin(rx.filter_map(|(peer_id, msg)| async move {
            if let Some(msg) = msg {
                match serde_json::from_str::<I>(&msg) {
                    Ok(msg) => Some((peer_id, Some(msg))),
                    Err(err) => {
                        warn!("Failed to parse incoming message: {} ({})", err, msg);
                        None
                    }
                }
            } else {
                Some((peer_id, None))
            }
        })));

        let state = Arc::new(Mutex::new(State {
            tx: Some(tx),
            peers: HashMap::new(),
        }));

        let state_clone = state.clone();
        task::spawn(async move {
            while let Some((peer_id, msg)) = handler.next().await {
                // Handle the first producer that connects
                match msg {
                    OutgoingMessage::Welcome { ref peer_id } => {
                        debug!("Got producer: {peer_id}");
                        let mut out_peer_id = out_peer_id.lock().unwrap();
                        if (*out_peer_id).is_none() {
                            *out_peer_id = Some(peer_id.clone());
                        }
                    }
                    _ => (),
                }

                match serde_json::to_string(&msg) {
                    Ok(msg_str) => {
                        let sender = {
                            let mut state = state_clone.lock().unwrap();
                            if let Some(peer) = state.peers.get_mut(&peer_id) {
                                Some(peer.sender.clone())
                            } else {
                                None
                            }
                        };

                        if let Some(mut sender) = sender {
                            trace!("Sending {}", msg_str);
                            let _ = sender.send(msg_str).await;
                        }
                    }
                    Err(err) => {
                        warn!("Failed to serialize outgoing message: {}", err);
                    }
                }
            }
        });

        Self { state }
    }

    fn remove_peer(state: Arc<Mutex<State>>, peer_id: &str) {
        if let Some(mut peer) = state.lock().unwrap().peers.remove(peer_id) {
            let peer_id = peer_id.to_string();
            task::spawn(async move {
                peer.sender.close_channel();
                if let Err(err) = peer.send_task_handle.await {
                    error!("Error while joining send task: {err} peer_id={peer_id}");
                }

                if let Err(err) = peer.receive_task_handle.await {
                    error!("Error while joining receive task: {err} peer_id={peer_id}");
                }
            });
        }
    }

    pub async fn accept_async<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
        &mut self,
        stream: S,
    ) -> Result<String, ServerError> {
        let ws = match async_tungstenite::tokio::accept_async(stream).await {
            Ok(ws) => ws,
            Err(err) => {
                warn!("Error during the websocket handshake: {}", err);
                return Err(ServerError::Handshake);
            }
        };

        let this_id = uuid::Uuid::new_v4().to_string();
        debug!("New WebSocket connection: this_id={this_id}");

        let (websocket_sender, mut websocket_receiver) = mpsc::channel::<String>(10);

        let this_id_clone = this_id.clone();
        let (mut ws_sink, mut ws_stream) = ws.split();
        let send_task_handle = task::spawn(async move {
            let mut res = Ok(());
            loop {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    websocket_receiver.next(),
                )
                .await
                {
                    Ok(Some(msg)) => {
                        trace!("sending {msg} this_id={this_id_clone}");
                        res = ws_sink.send(WsMessage::text(msg)).await;
                    }
                    Ok(None) => {
                        break;
                    }
                    Err(_) => {
                        trace!("timeout, sending ping this_id={this_id_clone}");
                        res = ws_sink.send(WsMessage::Ping(Default::default())).await;
                    }
                }

                if let Err(ref err) = res {
                    error!("Quitting send loop: {err} this_id={this_id_clone}");
                    break;
                }
            }

            debug!("Done sending this_id={this_id_clone}");

            let _ = ws_sink.close().await;

            res.map_err(|err| err.to_string())
        });

        let mut tx = self.state.lock().unwrap().tx.clone();
        let this_id_clone = this_id.clone();
        let state_clone = self.state.clone();
        let receive_task_handle = task::spawn(async move {
            if let Some(tx) = tx.as_mut() {
                if let Err(err) = tx
                    .send((
                        this_id_clone.clone(),
                        Some(
                            serde_json::json!({
                                "type": "newPeer",
                            })
                            .to_string()
                            .into(),
                        ),
                    ))
                    .await
                {
                    warn!("Error handling message: {err:?} this={this_id_clone}");
                }
            }
            while let Some(msg) = ws_stream.next().await {
                debug!("Received message {msg:?}");
                match msg {
                    Ok(WsMessage::Text(msg)) => {
                        if let Some(tx) = tx.as_mut() {
                            if let Err(err) = tx.send((this_id_clone.clone(), Some(msg))).await {
                                warn!("Error handling message: {err:?} this={this_id_clone}");
                            }
                        }
                    }
                    Ok(WsMessage::Close(reason)) => {
                        debug!("connection closed: {reason:?} this_id={this_id_clone}");
                        break;
                    }
                    Ok(WsMessage::Pong(_)) => {
                        continue;
                    }
                    Ok(_) => warn!("Unsupported message type this_id={this_id_clone}"),
                    Err(err) => {
                        warn!("recv error: {err} this_id={this_id_clone}");
                        break;
                    }
                }
            }

            if let Some(tx) = tx.as_mut() {
                let _ = tx.send((this_id_clone.clone(), None)).await;
            }

            Self::remove_peer(state_clone, &this_id_clone);
        });

        self.state.lock().unwrap().peers.insert(
            this_id.clone(),
            Peer {
                receive_task_handle,
                send_task_handle,
                sender: websocket_sender,
            },
        );

        Ok(this_id)
    }
}
