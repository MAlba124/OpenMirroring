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

use std::{num::NonZero, rc::Rc};

use anyhow::{Result, anyhow, bail};
use glow::HasContext;

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

pub struct VideoUnderlay {
    gl: Rc<glow::Context>,
    program: glow::Program,
    vbo: glow::Buffer,
    vao: glow::VertexArray,
    ebo: glow::Buffer,
    texture_location: glow::UniformLocation,
    transform_location: glow::UniformLocation,
}

impl VideoUnderlay {
    pub fn new(gl: Rc<glow::Context>) -> Result<Self> {
        unsafe {
            let program = gl.create_program().map_err(|err| anyhow!("{err}"))?;

            compile_shader(
                &gl,
                &program,
                include_str!("../shaders/video_vertex.glsl"),
                include_str!("../shaders/video_fragment.glsl"),
            )?;

            let transform_location = get_uniform_location!(gl, program, "transform")?;
            let texture_location = get_uniform_location!(gl, program, "texture")?;
            let position_location = get_attrib_location!(gl, program, "position")?;
            let texture_coords_location = get_attrib_location!(gl, program, "inTextureCoords")?;

            #[rustfmt::skip]
            let vertices: [f32; 16] = [
                -1.0, -1.0, 0.0, 1.0,
                1.0, -1.0, 1.0, 1.0,
                1.0, 1.0, 1.0, 0.0,
                -1.0, 1.0, 0.0, 0.0,
            ];
            let indices: [u32; 6] = [0, 1, 2, 2, 3, 0];

            let vao = gl.create_vertex_array().map_err(|err| anyhow!("{err}"))?;
            let vbo = gl.create_buffer().map_err(|err| anyhow!("{err}"))?;
            let ebo = gl.create_buffer().map_err(|err| anyhow!("{err}"))?;

            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                std::slice::from_raw_parts(
                    vertices.as_ptr() as *const u8,
                    vertices.len() * std::mem::size_of::<f32>(),
                ),
                glow::STATIC_DRAW,
            );

            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                std::slice::from_raw_parts(
                    indices.as_ptr() as *const u8,
                    indices.len() * std::mem::size_of::<u32>(),
                ),
                glow::STATIC_DRAW,
            );

            // note to self: the stride is related to how `vertecies` is laid out
            // attribute vec2 position;
            gl.enable_vertex_attrib_array(position_location);
            gl.vertex_attrib_pointer_f32(position_location, 2, glow::FLOAT, false, 4 * 4, 0);

            // attribute vec2 inTextureCoords;
            gl.enable_vertex_attrib_array(texture_coords_location);
            gl.vertex_attrib_pointer_f32(
                texture_coords_location,
                2,
                glow::FLOAT,
                false,
                4 * 4,
                2 * 4,
            );

            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_vertex_array(None);

            Ok(Self {
                gl,
                program,
                vbo,
                vao,
                ebo,
                transform_location,
                texture_location,
            })
        }
    }

    pub fn render(
        &mut self,
        texture_id: NonZero<u32>,
        window_width: u32,
        window_height: u32,
        texture_width: u32,
        texture_height: u32,
    ) {
        unsafe {
            let gl = &self.gl;

            // Transformation matrix for scaling the texture so the aspect ratio is correct
            let texture_transform = {
                let texture_aspect = texture_width as f32 / texture_height as f32;
                let window_aspect = window_width as f32 / window_height as f32;

                let (scale_x, scale_y) = if texture_aspect > window_aspect {
                    (1.0, window_aspect / texture_aspect)
                } else {
                    (texture_aspect / window_aspect, 1.0)
                };

                [
                    scale_x, 0.0, 0.0, 0.0, 0.0, scale_y, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
                    0.0, 1.0,
                ]
            };

            gl.viewport(0, 0, window_width as i32, window_height as i32);

            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            gl.use_program(Some(self.program));

            gl.uniform_matrix_4_f32_slice(
                Some(&self.transform_location),
                false,
                &texture_transform,
            );

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(glow::NativeTexture(texture_id)));
            gl.uniform_1_i32(Some(&self.texture_location), 0);

            gl.bind_vertex_array(Some(self.vao));
            gl.draw_elements(glow::TRIANGLES, 6, glow::UNSIGNED_INT, 0);

            gl.use_program(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_vertex_array(None);
        }
    }
}

impl Drop for VideoUnderlay {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_buffer(self.vbo);
            self.gl.delete_buffer(self.ebo);
        }
    }
}
