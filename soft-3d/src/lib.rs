#![feature(portable_simd, const_mut_refs, const_trait_impl)]

mod render;
pub use render::Renderer as RawRenderer;

use dust_core::{
    gpu::{
        engine_3d::RenderingState,
        engine_3d::{Color, Polygon, RenderingControl, ScreenVertex},
    },
    utils::{Bytes, Zero},
};

#[repr(C)]
pub struct RenderingData {
    pub control: RenderingControl,

    pub w_buffering: bool,

    pub alpha_test_ref: u8,

    pub clear_poly_id: u8,
    pub clear_image_offset: [u8; 2],
    pub clear_depth: u16,

    pub fog_offset: u16,
    pub fog_densities: [u8; 0x20],
    pub rear_plane_fog_enabled: bool,

    pub clear_color: Color,
    pub fog_color: Color,
    pub edge_colors: [Color; 8],
    pub toon_colors: [Color; 0x20],

    pub texture: Bytes<0x8_0000>,
    // TODO: How is the texture palette region mirrored?
    pub tex_pal: Bytes<0x2_0000>,

    pub vert_ram: [ScreenVertex; 6144],

    pub poly_ram: [Polygon; 2048],
    pub poly_ram_level: u16,
}

unsafe impl Zero for RenderingData {}

impl RenderingData {
    fn copy_rendering_data(
        &mut self,
        texture: &Bytes<0x8_0000>,
        tex_pal: &Bytes<0x1_8000>,
        state: &RenderingState,
    ) {
        self.control = state.control;

        self.alpha_test_ref = if state.control.alpha_test_enabled() {
            state.alpha_test_ref
        } else {
            0
        };

        self.clear_poly_id = state.clear_poly_id;
        self.clear_image_offset = state.clear_image_offset;
        self.clear_depth = state.clear_depth;

        self.fog_offset = state.fog_offset;
        self.fog_densities = state.fog_densities;
        self.rear_plane_fog_enabled = state.rear_plane_fog_enabled;

        self.fog_color = state.fog_color;
        self.edge_colors = state.edge_colors;
        self.clear_color = state.clear_color;
        self.toon_colors = state.toon_colors;

        for i in 0..4 {
            if state.texture_dirty & 1 << i == 0 {
                continue;
            }
            let range = i << 17..(i + 1) << 17;
            self.texture[range.clone()].copy_from_slice(&texture[range]);
        }

        for i in 0..6 {
            if state.tex_pal_dirty & 1 << i == 0 {
                continue;
            }
            let range = i << 14..(i + 1) << 14;
            self.tex_pal[range.clone()].copy_from_slice(&tex_pal[range]);
        }
    }

    #[inline]
    pub fn prepare(
        &mut self,
        texture: &Bytes<0x8_0000>,
        tex_pal: &Bytes<0x1_8000>,
        vert_ram: &[ScreenVertex],
        poly_ram: &[Polygon],
        state: &RenderingState,
        w_buffering: bool,
    ) {
        self.w_buffering = w_buffering;

        self.vert_ram[..vert_ram.len()].copy_from_slice(vert_ram);
        self.poly_ram[..poly_ram.len()].copy_from_slice(poly_ram);
        self.poly_ram_level = poly_ram.len() as u16;

        self.copy_rendering_data(texture, tex_pal, state);
    }

    pub fn repeat_last_frame(
        &mut self,
        texture: &Bytes<0x8_0000>,
        tex_pal: &Bytes<0x1_8000>,
        state: &RenderingState,
    ) {
        self.copy_rendering_data(texture, tex_pal, state);
    }
}
