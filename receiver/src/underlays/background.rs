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

use anyhow::{Result, anyhow};
use glow::HasContext;

use super::{get_attrib_location, get_uniform_location};

// Modified from https://github.com/slint-ui/slint/blob/master/examples/opengl_underlay/main.rs
pub struct BackgroundUnderlay {
    gl: std::rc::Rc<glow::Context>,
    program: glow::Program,
    effect_time_location: glow::UniformLocation,
    width_location: glow::UniformLocation,
    height_location: glow::UniformLocation,
    vbo: glow::Buffer,
    vao: glow::VertexArray,
    start_time: std::time::Instant,
}

impl BackgroundUnderlay {
    pub fn new(gl: std::rc::Rc<glow::Context>) -> Result<Self> {
        unsafe {
            let program = gl.create_program().map_err(|err| anyhow!("{err}"))?;

            super::compile_shader(
                &gl,
                &program,
                include_str!("../../shaders/background_vertex.glsl"),
                include_str!("../../shaders/background_fragment.glsl"),
            )?;

            let effect_time_location = get_uniform_location!(gl, program, "effect_time")?;
            let width_location = get_uniform_location!(gl, program, "win_width")?;
            let height_location = get_uniform_location!(gl, program, "win_height")?;
            let position_location = get_attrib_location!(gl, program, "position")?;

            let vbo = gl.create_buffer().map_err(|err| anyhow!("{err}"))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));

            let vertices: [f32; 8] = [
                -1.0, 1.0, //
                -1.0, -1.0, //
                1.0, 1.0, //
                1.0, -1.0, //
            ];

            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertices.align_to().1, glow::STATIC_DRAW);

            let vao = gl.create_vertex_array().map_err(|err| anyhow!("{err}"))?;
            gl.bind_vertex_array(Some(vao));

            gl.enable_vertex_attrib_array(position_location);
            gl.vertex_attrib_pointer_f32(position_location, 2, glow::FLOAT, false, 8, 0);

            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);

            Ok(Self {
                gl,
                program,
                effect_time_location,
                vbo,
                vao,
                start_time: std::time::Instant::now(),
                width_location,
                height_location,
            })
        }
    }

    pub fn render(&mut self, width: f32, height: f32) {
        unsafe {
            let gl = &self.gl;

            gl.use_program(Some(self.program));

            let old_buffer =
                std::num::NonZeroU32::new(gl.get_parameter_i32(glow::ARRAY_BUFFER_BINDING) as u32)
                    .map(glow::NativeBuffer);

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            let old_vao =
                std::num::NonZeroU32::new(gl.get_parameter_i32(glow::VERTEX_ARRAY_BINDING) as u32)
                    .map(glow::NativeVertexArray);

            gl.bind_vertex_array(Some(self.vao));

            let elapsed = self.start_time.elapsed().as_secs_f32() / 1.5;
            gl.uniform_1_f32(Some(&self.effect_time_location), elapsed);

            gl.uniform_1_f32(Some(&self.width_location), width);
            gl.uniform_1_f32(Some(&self.height_location), height);

            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

            gl.bind_buffer(glow::ARRAY_BUFFER, old_buffer);
            gl.bind_vertex_array(old_vao);
            gl.use_program(None);
        }
    }
}

impl Drop for BackgroundUnderlay {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_buffer(self.vbo);
        }
    }
}
