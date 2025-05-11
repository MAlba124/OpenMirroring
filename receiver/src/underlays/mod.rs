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

use anyhow::{Result, anyhow, bail};
use glow::HasContext;

pub mod background;
pub mod video;

unsafe fn compile_shader(
    gl: &glow::Context,
    program: &glow::Program,
    vert_source: &str,
    frag_source: &str,
) -> Result<()> {
    unsafe {
        let shader_sources = [
            (glow::VERTEX_SHADER, vert_source),
            (glow::FRAGMENT_SHADER, frag_source),
        ];

        let mut shaders = Vec::with_capacity(shader_sources.len());

        for (shader_type, shader_source) in shader_sources.iter() {
            let shader = gl
                .create_shader(*shader_type)
                .map_err(|err| anyhow!("{err}"))?;
            gl.shader_source(shader, shader_source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                bail!("{}", gl.get_shader_info_log(shader));
            }
            gl.attach_shader(*program, shader);
            shaders.push(shader);
        }

        gl.link_program(*program);
        if !gl.get_program_link_status(*program) {
            bail!("{}", gl.get_program_info_log(*program));
        }

        for shader in shaders {
            gl.detach_shader(*program, shader);
            gl.delete_shader(shader);
        }
    }

    Ok(())
}

macro_rules! get_uniform_location {
    ($gl:expr, $program:expr, $name:expr) => {
        $gl.get_uniform_location($program, $name)
            .ok_or(anyhow!("Failed to get uniform location of `{}`", $name))
    };
}

macro_rules! get_attrib_location {
    ($gl:expr, $program:expr, $name:expr) => {
        $gl.get_attrib_location($program, $name)
            .ok_or(anyhow!("Failed to get attribute location of `{}`", $name))
    };
}

pub(crate) use get_attrib_location;
pub(crate) use get_uniform_location;
