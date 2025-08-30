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

use fcast_lib::models::{PlayMessage, SeekMessage, SetSpeedMessage, SetVolumeMessage};

pub mod pipeline;
pub mod session;
pub mod video_underlay;

#[derive(Debug)]
pub enum Event {
    Pause,
    Play(PlayMessage),
    Resume,
    Stop,
    SetSpeed(SetSpeedMessage),
    Seek(SeekMessage),
    SetVolume(SetVolumeMessage),
    Quit,
    PipelineEos,
    PipelineError,
    SessionFinished,
    ResumeOrPause,
    SeekPercent(f32),
    PipelineStateChanged(gst::State),
    ToggleDebug,
}

#[macro_export]
macro_rules! log_if_err {
    ($res:expr) => {
        if let Err(err) = $res {
            error!("{err}");
        }
    };
}
