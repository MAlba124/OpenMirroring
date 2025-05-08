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

// TODO: dma buf (https://docs.pipewire.org/page_dma_buf.html)

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameFormat {
    RGBx,
    XBGR,
    BGRx,
    BGRA,
    RGBA,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    pub format: FrameFormat,
    pub width: u32,
    pub height: u32,
    // TODO: rotation
}
