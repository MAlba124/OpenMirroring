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

use anyhow::Result;
use log::{debug, info};
use tokio::net::TcpListener;

use crate::{Event, session::SessionId};

pub struct Dispatcher {
    listener: TcpListener,
    event_tx: tokio::sync::mpsc::Sender<Event>,
}

impl Dispatcher {
    pub async fn new(event_tx: tokio::sync::mpsc::Sender<Event>) -> Result<Self> {
        let listener = TcpListener::bind("0.0.0.0:46899").await?;

        Ok(Self { listener, event_tx })
    }

    // TODO: Websocket listener
    pub async fn run(self, mut fin_rx: tokio::sync::oneshot::Receiver<()>) -> Result<()> {
        info!("Listening on {:?}", self.listener.local_addr());

        let mut id: SessionId = 0;

        loop {
            tokio::select! {
                _ = &mut fin_rx => break,
                r = self.listener.accept() => {
                    let (stream, _) = r?;
                    self.event_tx
                        .send(Event::CreateSessionRequest { stream, id })
                        .await?;
                    id += 1;
                }
            }
        }

        debug!("Quitting");

        self.event_tx.send(Event::Quit).await?;

        Ok(())
    }
}
