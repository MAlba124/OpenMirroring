use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fcast_iced::{
    protocol_types::{
        PlayMessage, PlaybackErrorMessage, PlaybackState, PlaybackUpdateMessage, SeekMessage,
        SetSpeedMessage, SetVolumeMessage, VolumeUpdateMessage,
    },
    session::{Session, SessionId, SessionMessage},
    CreateSessionRequest, Message,
};
use iced::{
    futures::{channel::mpsc::Sender, SinkExt, Stream},
    widget::{center, text},
    Element, Subscription, Task,
};
use iced_video_player::{Video, VideoPlayer};
use log::{debug, error, info};
use url::Url;

fn new_session_stream() -> impl Stream<Item = Message> {
    iced::stream::channel(1, move |mut output: Sender<Message>| async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:46899")
            .await
            .unwrap();

        info!("Listening on {:?}", listener.local_addr());

        let mut id: SessionId = 0;

        loop {
            match listener.accept().await {
                Ok((net_stream, _)) => {
                    output
                        .send(Message::CreateSession(CreateSessionRequest {
                            net_stream_mutex: Arc::new(Mutex::new(Some(net_stream))),
                            id,
                        }))
                        .await
                        .unwrap();
                    id += 1;
                }
                Err(e) => panic!("{}", e.to_string()),
            }
        }
    })
}

fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

struct Fcast {
    video: Option<Video>,
    sessions: HashMap<SessionId, tokio::sync::mpsc::Sender<SessionMessage>>,
}

impl Fcast {
    pub fn new() -> (Self, Task<Message>) {
        (
            Self {
                video: None,
                sessions: HashMap::new(),
            },
            Task::none(),
        )
    }

    async fn send_message_to_session(
        tx: tokio::sync::mpsc::Sender<SessionMessage>,
        msg: SessionMessage,
    ) -> Message {
        debug!("Sending message to sessions: {msg:?}");
        if let Err(e) = tx.send(msg).await {
            error!("Error occured sending message to sessions: {e}");
        }

        Message::Nothing
    }

    fn send_to_all_sessions(&mut self, msg: SessionMessage) -> Task<Message> {
        let mut tasks: Vec<Task<Message>> = Vec::new();

        for session in self.sessions.clone() {
            tasks.push(Task::future(Self::send_message_to_session(
                session.1,
                msg.clone(),
            )));
        }

        Task::batch(tasks)
    }

    fn playback_state(&mut self) -> Option<PlaybackUpdateMessage> {
        self.video.as_ref().map(|v| PlaybackUpdateMessage {
            generation: current_time_millis(),
            time: v.position().as_secs_f64(),
            duration: v.duration().as_secs_f64(),
            state: v.paused().into(),
            speed: v.speed(),
        })
    }

    fn send_playback_state_to_sessions(&mut self) -> Task<Message> {
        if self.sessions.is_empty() {
            return Task::none();
        }

        match self.playback_state() {
            Some(state) => self.send_to_all_sessions(state.into()),
            None => self.send_to_all_sessions(
                PlaybackUpdateMessage {
                    generation: current_time_millis(),
                    time: 0.0,
                    duration: 0.0,
                    state: PlaybackState::Idle,
                    speed: 0.0,
                }
                .into(),
            ),
        }
    }

    fn send_error_to_sessions(&mut self, e: String) -> Task<Message> {
        self.send_to_all_sessions(PlaybackErrorMessage { message: e }.into())
    }

    fn seek(&mut self, msg: SeekMessage) -> Task<Message> {
        match &mut self.video {
            Some(v) => match v.seek(Duration::from_secs_f64(msg.time), false) {
                Ok(_) => {
                    debug!("Successfully seeked to {}", msg.time);
                    self.send_playback_state_to_sessions()
                }
                Err(e) => {
                    error!("Seek failed: {e}");
                    self.send_error_to_sessions(e.to_string())
                }
            },
            None => Task::none(),
        }
    }

    fn resume(&mut self) -> Task<Message> {
        match &mut self.video {
            Some(v) => {
                debug!("Resuming playback");
                v.set_paused(false);
                self.send_playback_state_to_sessions()
            }
            None => Task::none(),
        }
    }

    // Currently does not work when streaming over network
    fn set_speed(&mut self, msg: SetSpeedMessage) -> Task<Message> {
        match &mut self.video {
            Some(v) => match v.set_speed(msg.speed) {
                Ok(_) => {
                    debug!("Successfully set playback speed to {}", msg.speed);
                    self.send_playback_state_to_sessions()
                }
                Err(e) => {
                    error!("Failed to set playback speed: {e}");
                    Task::none()
                }
            },
            None => Task::none(),
        }
    }

    fn pause(&mut self) -> Task<Message> {
        match &mut self.video {
            Some(v) => {
                debug!("Pausing playback");
                v.set_paused(true);
                self.send_playback_state_to_sessions()
            }
            None => Task::none(),
        }
    }

    fn play(&mut self, msg: PlayMessage) -> Task<Message> {
        // TODO:
        let resource_location = Url::parse(&msg.url.unwrap()).unwrap();
        match Video::new(&resource_location) {
            Ok(v) => {
                debug!("Successfully created video src={resource_location:?}");
                self.video = Some(v);
                self.send_playback_state_to_sessions()
            }
            Err(e) => {
                error!("Failed to create video: {e}");
                Task::none()
            }
        }
    }

    fn end_of_stream_or_stop(&mut self) -> Task<Message> {
        debug!("Playback reached end of stream (EOS) or playback was stopped");
        self.video = None;
        if !self.sessions.is_empty() {
            self.send_to_all_sessions(
                PlaybackUpdateMessage {
                    generation: current_time_millis(),
                    time: 0.0,
                    duration: 0.0,
                    state: PlaybackState::Idle,
                    speed: 0.0,
                }
                .into(),
            )
        } else {
            Task::none()
        }
    }

    fn create_session(&mut self, req: CreateSessionRequest) -> Task<Message> {
        let mut net_stream = req.net_stream_mutex.lock().unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(10);

        if self.sessions.insert(req.id, tx).is_some() {
            error!(
                "Session ({}) already exists. Removing and adding the new anyways",
                req.id
            );
        }

        Task::stream(
            Session::new(
                net_stream
                    .take()
                    .expect("There should always be a stream here"),
                req.id,
            )
            .stream(rx),
        )
    }

    fn set_volume(&mut self, msg: SetVolumeMessage) -> Task<Message> {
        match &mut self.video {
            Some(v) => {
                debug!("Setting playback volume to {}", msg.volume);
                v.set_volume(msg.volume);

                let update = VolumeUpdateMessage {
                    generation: current_time_millis(),
                    volume: v.volume(),
                };

                if !self.sessions.is_empty() {
                    self.send_to_all_sessions(update.into())
                } else {
                    Task::none()
                }
            }
            None => Task::none(),
        }
    }

    fn playback_error(&mut self, e: String) -> Task<Message> {
        error!("Playback error: {e}");
        if !self.sessions.is_empty() {
            self.send_to_all_sessions(PlaybackErrorMessage { message: e }.into())
        } else {
            debug!("No sessions connected to send error to");
            Task::none()
        }
    }

    fn session_destroy(&mut self, id: SessionId) {
        if self.sessions.remove(&id).is_none() {
            error!("Session ({id}) does not exist so it can't be destroyed");
        }
    }

    fn tick(&mut self) -> Task<Message> {
        self.send_playback_state_to_sessions()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Play(play_message) => return self.play(play_message),
            Message::EndOfStream => return self.end_of_stream_or_stop(),
            Message::Pause => return self.pause(),
            Message::Resume => return self.resume(),
            Message::SetSpeed(set_speed_message) => return self.set_speed(set_speed_message),
            Message::Stop => return self.end_of_stream_or_stop(),
            Message::CreateSession(req) => return self.create_session(req),
            Message::Seek(seek_message) => return self.seek(seek_message),
            Message::SetVolume(set_volume_message) => return self.set_volume(set_volume_message),
            Message::PlaybackError(e) => return self.playback_error(e),
            Message::Tick => return self.tick(),
            Message::SessionDestroyed(id) => self.session_destroy(id),
            Message::Nothing => {}
        }

        Task::none()
    }

    pub fn view(&self) -> Element<Message> {
        match &self.video {
            Some(v) => VideoPlayer::new(v)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .content_fit(iced::ContentFit::Contain)
                .on_end_of_stream(Message::EndOfStream)
                .on_error(|e| Message::PlaybackError(e.to_string()))
                .into(),
            None => center(text("Listening on localhost:46899")).into(),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(new_session_stream),
            iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick),
        ])
    }
}

pub fn main() -> iced::Result {
    env_logger::Builder::from_default_env()
        .filter_module("fcast_iced", log::LevelFilter::Debug)
        .init();

    iced::application("FCast", Fcast::update, Fcast::view)
        .subscription(Fcast::subscription)
        .run_with(Fcast::new)
}
