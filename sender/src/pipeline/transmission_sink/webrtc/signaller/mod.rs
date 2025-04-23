// Copyright (C) 2025 Marcus L. Hanestad <marlhan@proton.me>
//
// This file is part of OpenMirroring.
//
// OpenMirroring is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// OpenMirroring is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with OpenMirroring.  If not, see <https://www.gnu.org/licenses/>.

use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

pub mod handlers;
pub mod protocol;
pub mod server;

pub async fn run_server(
    peer_id: Arc<Mutex<Option<String>>>,
    mut quit_signal: oneshot::Receiver<()>,
) {
    let server = server::Server::spawn(handlers::Handler::new, peer_id);

    // TODO: use random port
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8443").await.unwrap();

    log::debug!("WebRTC signaller running");

    loop {
        tokio::select! {
            _ = &mut quit_signal => {
                break;
            }
            res = listener.accept() => {
                let Ok((stream, address)) = res else {
                    break;
                };
                let mut server_clone = server.clone();
                log::info!("Accepting connection from {address}");
                tokio::task::spawn(async move { server_clone.accept_async(stream).await });
            }
        }
    }
}
