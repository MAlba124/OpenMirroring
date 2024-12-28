use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use fcast_iced::{
    protocol_types::{
        PlayMessage, PlaybackErrorMessage, PlaybackUpdateMessage, SeekMessage, SetSpeedMessage,
        SetVolumeMessage, VolumeUpdateMessage,
    },
    session::{Session, SessionMessage},
    Message,
};
use iced::{
    futures::{channel::mpsc::Sender, SinkExt, Stream},
    widget::{center, text},
    Element, Subscription, Task,
};
use iced_video_player::{Video, VideoPlayer};
use log::{debug, error, info};
use tokio::net::TcpStream;
use url::Url;

fn create_message_stream() -> impl Stream<Item = Message> {
    iced::stream::channel(1, move |mut output: Sender<Message>| async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:46899")
            .await
            .unwrap();

        info!("Listening on {:?}", listener.local_addr());

        loop {
            match listener.accept().await {
                Ok((net_stream, _)) => {
                    output
                        .send(Message::CreateSession(Arc::new(Mutex::new(Some(
                            net_stream,
                        )))))
                        .await
                        .unwrap();
                }
                Err(e) => panic!("{}", e.to_string()),
            }
        }
    })
}

fn current_time_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64
}

struct Fcast {
    video: Option<Video>,
    session_tx: async_channel::Sender<SessionMessage>,
    session_rx: async_channel::Receiver<SessionMessage>,
    n_sessions: usize,
    last_position_update: Instant,
}

impl Fcast {
    pub fn new() -> (Self, Task<Message>) {
        let (session_tx, session_rx) = async_channel::unbounded();

        (
            Self {
                video: None,
                session_tx,
                session_rx,
                last_position_update: Instant::now(),
                n_sessions: 0,
            },
            Task::none(),
        )
    }

    async fn send_message_to_sessions(
        tx: async_channel::Sender<SessionMessage>,
        msg: SessionMessage,
    ) -> Message {
        debug!("Sending message to sessions: {msg:?}");
        if let Err(e) = tx.send(msg).await {
            error!("Error occured sending message to sessions: {e}");
        }

        Message::Nothing
    }

    fn playback_state(&mut self) -> Option<PlaybackUpdateMessage> {
        self.video.as_ref().map(|v| PlaybackUpdateMessage {
            generation: current_time_millis(),
            time: v.position().as_secs_f64(),
            duration: v.duration().as_secs_f64(),
            state: if v.paused() { 2.0 } else { 1.0 },
            speed: v.speed(),
        })
    }

    fn send_playback_state_to_sessions(&mut self) -> Task<Message> {
        if self.n_sessions < 1 {
            return Task::none();
        }

        match self.playback_state() {
            Some(state) => Task::future(Self::send_message_to_sessions(
                self.session_tx.clone(),
                state.into(),
            )),
            None => {
                error!("Could not get playback state");
                Task::none()
            }
        }
    }

    fn send_error_to_sessions(&mut self, e: String) -> Task<Message> {
        Task::future(Self::send_message_to_sessions(
            self.session_tx.clone(),
            PlaybackErrorMessage { message: e }.into(),
        ))
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
        if self.n_sessions > 0 {
            Task::future(Self::send_message_to_sessions(
                self.session_tx.clone(),
                PlaybackUpdateMessage {
                    generation: current_time_millis(),
                    time: 0.0,
                    duration: 0.0,
                    state: 0.0,
                    speed: 0.0,
                }
                .into(),
            ))
        } else {
            Task::none()
        }
    }

    fn create_session(&mut self, net_stream_mutex: Arc<Mutex<Option<TcpStream>>>) -> Task<Message> {
        let mut net_stream = net_stream_mutex.lock().unwrap();
        Task::stream(
            Session::new(
                net_stream
                    .take()
                    .expect("There should always be a stream here"),
            )
            .stream(self.session_rx.clone()),
        )
    }

    fn set_volume(&mut self, msg: SetVolumeMessage) -> Task<Message> {
        match &mut self.video {
            Some(v) => {
                debug!("Setting playback volume to {}", msg.volume);
                v.set_volume(msg.volume);

                if self.n_sessions > 0 {
                    Task::future(Self::send_message_to_sessions(
                        self.session_tx.clone(),
                        VolumeUpdateMessage {
                            generation: current_time_millis(),
                            volume: v.volume(),
                        }
                        .into(),
                    ))
                } else {
                    Task::none()
                }
            }
            None => Task::none(),
        }
    }

    fn playback_error(&mut self, e: String) -> Task<Message> {
        error!("Playback error: {e}");
        if self.n_sessions > 0 {
            Task::future(Self::send_message_to_sessions(
                self.session_tx.clone(),
                PlaybackErrorMessage { message: e }.into(),
            ))
        } else {
            debug!("No sessions connected to send error to");
            Task::none()
        }
    }

    fn tick(&mut self) -> Task<Message> {
        let now = Instant::now();
        if now - self.last_position_update >= Duration::from_secs(1) {
            self.last_position_update = now;
            return self.send_playback_state_to_sessions();
        }

        Task::none()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Play(play_message) => return self.play(play_message),
            Message::EndOfStream => return self.end_of_stream_or_stop(),
            Message::Pause => return self.pause(),
            Message::Resume => return self.resume(),
            Message::SetSpeed(set_speed_message) => return self.set_speed(set_speed_message),
            Message::Stop => return self.end_of_stream_or_stop(),
            Message::CreateSession(net_stream_mutex) => {
                return self.create_session(net_stream_mutex)
            }
            Message::Seek(seek_message) => return self.seek(seek_message),
            Message::SetVolume(set_volume_message) => return self.set_volume(set_volume_message),
            Message::PlaybackError(e) => return self.playback_error(e),
            // Message::PlaybackNewFrame => return self.new_frame(),
            Message::Tick => return self.tick(),
            Message::Nothing => {}
            Message::SessionCreated => self.n_sessions += 1,
            Message::SessionDestroyed => self.n_sessions -= 1,
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
                // .on_new_frame(Message::PlaybackNewFrame)
                .on_error(|e| Message::PlaybackError(e.to_string()))
                .into(),
            None => center(text("Listening on localhost:46899")).into(),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run(create_message_stream),
            iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
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
