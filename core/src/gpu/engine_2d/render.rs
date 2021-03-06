mod all;
#[cfg(target_arch = "x86_64")]
mod avx2;
mod common;

use super::{
    AffineBgIndex, BgIndex, BgObjPixel, Engine2d, OamAttr0, OamAttr1, OamAttr2, ObjPixel, Role,
    WindowPixel,
};
use crate::{
    gpu::{engine_3d, vram::Vram, Scanline, SCREEN_HEIGHT, SCREEN_WIDTH},
    utils::make_zero,
};

pub struct FnPtrs<R: Role> {
    render_scanline_bg_text: fn(&mut Engine2d<R>, bg_index: BgIndex, line: u8, vram: &Vram),
}

impl<R: Role> FnPtrs<R> {
    #[allow(unused_labels)]
    pub fn new() -> Self {
        macro_rules! fn_ptr {
            ($ident: ident $($generics: tt)*) => {
                'get_fn_ptr: {
                    #[cfg(target_arch = "x86_64")]
                    if is_x86_feature_detected!("avx2") {
                        break 'get_fn_ptr avx2::$ident$($generics)*;
                    }
                    all::$ident$($generics)*
                }
            }
        }
        FnPtrs {
            render_scanline_bg_text: fn_ptr!(render_scanline_bg_text::<R>),
        }
    }
}

const fn rgb_15_to_18(value: u32) -> u32 {
    (value << 1 & 0x3E) | (value << 2 & 0xF80) | (value << 3 & 0x3_E000)
}

const fn rgb_18_to_rgba_32(value: u32) -> u32 {
    let rgb_6_8 = (value & 0x3F) | (value << 2 & 0x3F00) | (value << 4 & 0x3F_0000);
    0xFF00_0000 | rgb_6_8 << 2 | (rgb_6_8 >> 4 & 0x0003_0303)
}

impl<R: Role> Engine2d<R> {
    fn apply_color_effects<const EFFECT: u8>(&mut self) {
        #[inline]
        fn blend(color_a: u32, color_b: u32, coeff_a: u32, coeff_b: u32) -> u32 {
            let r = ((color_a & 0x3F) * coeff_a + (color_b & 0x3F) * coeff_b).min(0x3F0);
            let g =
                ((color_a & 0xFC0) * coeff_a + (color_b & 0xFC0) * coeff_b).min(0xFC00) & 0xFC00;
            let b = ((color_a & 0x3_F000) * coeff_a + (color_b & 0x3_F000) * coeff_b)
                .min(0x3F_0000)
                & 0x3F_0000;
            (r | g | b) >> 4
        }

        #[inline]
        fn blend_5bit_coeff(color_a: u32, color_b: u32, coeff_a: u32, coeff_b: u32) -> u32 {
            let r = ((color_a & 0x3F) * coeff_a + (color_b & 0x3F) * coeff_b).min(0x7E0);
            let g =
                ((color_a & 0xFC0) * coeff_a + (color_b & 0xFC0) * coeff_b).min(0x1F800) & 0x1F800;
            let b = ((color_a & 0x3_F000) * coeff_a + (color_b & 0x3_F000) * coeff_b)
                .min(0x7E_0000)
                & 0x7E_0000;
            (r | g | b) >> 5
        }

        let target_1_mask = self.color_effects_control.target_1_mask();
        let target_2_mask = self.color_effects_control.target_2_mask();
        let a_coeff = self.blend_coeffs.0 as u32;
        let b_coeff = self.blend_coeffs.1 as u32;
        let brightness_coeff = self.brightness_coeff as u32;
        for i in 0..SCREEN_WIDTH {
            let pixel = self.bg_obj_scanline.0[i];
            let top = BgObjPixel(pixel as u32);
            self.bg_obj_scanline.0[i] = if self.window.0[i].color_effects_enabled() {
                let bot = BgObjPixel((pixel >> 32) as u32);
                let top_mask = top.color_effects_mask();
                let bot_matches = bot.color_effects_mask() & target_2_mask != 0;
                if top.is_3d() && bot_matches {
                    let a_coeff = (top.alpha() + 1) as u32;
                    let b_coeff = (32 - a_coeff) as u32;
                    blend_5bit_coeff(top.0, bot.0, a_coeff, b_coeff)
                } else if top.force_blending() && bot_matches {
                    let (a_coeff, b_coeff) = if top.custom_alpha() {
                        (top.alpha() as u32, 16 - top.alpha() as u32)
                    } else {
                        (a_coeff, b_coeff)
                    };
                    blend(top.0, bot.0, a_coeff, b_coeff)
                } else if EFFECT != 0 && top_mask & target_1_mask != 0 {
                    match EFFECT {
                        1 => {
                            if bot_matches {
                                blend(top.0, bot.0, a_coeff, b_coeff)
                            } else {
                                top.0
                            }
                        }

                        2 => {
                            let increment = {
                                let complement = 0x3_FFFF ^ top.0;
                                ((((complement & 0x3_F03F) * brightness_coeff) & 0x3F_03F0)
                                    | (((complement & 0xFC0) * brightness_coeff) & 0xFC00))
                                    >> 4
                            };
                            top.0 + increment
                        }

                        _ => {
                            let decrement = {
                                ((((top.0 & 0x3_F03F) * brightness_coeff) & 0x3F_03F0)
                                    | (((top.0 & 0xFC0) * brightness_coeff) & 0xFC00))
                                    >> 4
                            };
                            top.0 - decrement
                        }
                    }
                } else {
                    top.0
                }
            } else {
                top.0
            } as u64;
        }
    }

    pub(in super::super) fn update_windows(&mut self, vcount: u16) {
        for i in 0..2 {
            if self.control.win01_enabled() & 1 << i == 0 {
                self.windows_active[i] = false;
                continue;
            }

            let y_range = &self.window_ranges[i].y;
            let y_start = y_range.0;
            let mut y_end = y_range.1;
            if y_end < y_start {
                y_end = 192;
            }
            if vcount as u8 == y_start {
                self.windows_active[i] = true;
            }
            if vcount as u8 == y_end {
                self.windows_active[i] = false;
            }
        }
    }

    pub(in super::super) fn render_scanline(
        &mut self,
        vcount: u16,
        scanline_buffer: &mut Scanline<u32>,
        vram: &mut Vram,
        renderer_3d: &mut dyn engine_3d::Renderer,
    ) {
        // According to melonDS, if vcount falls outside the drawing range or 2D engine B is
        // disabled, the scanline is filled with pure white.
        if vcount >= SCREEN_HEIGHT as u16 || (!R::IS_A && !self.enabled) {
            if R::IS_A && self.engine_3d_enabled_in_frame {
                renderer_3d.skip_scanline();
            }
            // TODO: Display capture interaction?

            scanline_buffer.0.fill(0xFFFF_FFFF);
            return;
        }

        let vcount = vcount as u8;

        let display_mode = if R::IS_A {
            self.control.display_mode_a()
        } else {
            self.control.display_mode_b()
        };

        let scanline_3d = if R::IS_A && self.engine_3d_enabled_in_frame {
            let enabled_in_bg_obj = self.bgs[0].priority != 4 && self.control.bg0_3d();
            if (self.capture_enabled_in_frame
                && (self.capture_control.src_a_3d_only() || enabled_in_bg_obj))
                || (display_mode == 1 && enabled_in_bg_obj)
            {
                Some(renderer_3d.read_scanline())
            } else {
                renderer_3d.skip_scanline();
                None
            }
        } else {
            None
        };

        if display_mode == 1
            || (R::IS_A && self.capture_enabled_in_frame && !self.capture_control.src_a_3d_only())
        {
            self.window.0[..SCREEN_WIDTH].fill(WindowPixel(if self.control.wins_enabled() == 0 {
                0x3F
            } else {
                self.window_control[2].0
            }));

            if self.control.obj_win_enabled() {
                let obj_window_pixel = WindowPixel(self.window_control[3].0);
                for (i, window_pixel) in self.window.0[..SCREEN_WIDTH].iter_mut().enumerate() {
                    if self.obj_window[i >> 3] & 1 << (i & 7) != 0 {
                        *window_pixel = obj_window_pixel;
                    }
                }
            }

            for i in (0..2).rev() {
                if !self.windows_active[i] {
                    continue;
                }

                let x_range = &self.window_ranges[i].x;
                let x_start = x_range.0 as usize;
                let mut x_end = x_range.1 as usize;
                if x_end < x_start {
                    x_end = 256;
                }
                self.window.0[x_start..x_end].fill(WindowPixel(self.window_control[i].0));
            }

            let backdrop = BgObjPixel(rgb_15_to_18(
                vram.palette.read_le::<u16>((!R::IS_A as usize) << 10) as u32,
            ))
            .with_color_effects_mask(1 << 5)
            .0;
            self.bg_obj_scanline
                .0
                .fill(backdrop as u64 | (backdrop as u64) << 32);

            [
                Self::render_scanline_bgs_and_objs::<0>,
                Self::render_scanline_bgs_and_objs::<1>,
                Self::render_scanline_bgs_and_objs::<2>,
                Self::render_scanline_bgs_and_objs::<3>,
                Self::render_scanline_bgs_and_objs::<4>,
                Self::render_scanline_bgs_and_objs::<5>,
                Self::render_scanline_bgs_and_objs::<6>,
                Self::render_scanline_bgs_and_objs::<7>,
            ][self.control.bg_mode() as usize](self, vcount, vram, scanline_3d);
            [
                Self::apply_color_effects::<0>,
                Self::apply_color_effects::<1>,
                Self::apply_color_effects::<2>,
                Self::apply_color_effects::<3>,
            ][self.color_effects_control.color_effect() as usize](self);
        }

        #[allow(clippy::match_same_arms)]
        match display_mode {
            0 => {
                scanline_buffer.0.fill(0xFFFF_FFFF);
                return;
            }

            1 => {}

            2 => {
                // The bank must be mapped as LCDC VRAM to be used
                let bank_index = self.control.a_vram_bank();
                let bank_control = vram.bank_control()[bank_index as usize];
                if bank_control.enabled() && bank_control.mst() == 0 {
                    let bank = match bank_index {
                        0 => &vram.banks.a,
                        1 => &vram.banks.b,
                        2 => &vram.banks.c,
                        _ => &vram.banks.d,
                    };
                    let line_base = (vcount as usize) << 9;
                    for (i, pixel) in scanline_buffer.0.iter_mut().enumerate() {
                        let src =
                            unsafe { bank.read_le_aligned_unchecked::<u16>(line_base | i << 1) };
                        *pixel = rgb_15_to_18(src as u32);
                    }
                } else {
                    scanline_buffer.0.fill(0);
                }
            }

            _ => {
                // TODO: Main memory display mode
            }
        }

        #[allow(clippy::similar_names)]
        if R::IS_A && self.capture_enabled_in_frame && vcount < self.capture_height {
            let dst_bank_index = self.capture_control.dst_bank();
            let dst_bank_control = vram.bank_control()[dst_bank_index as usize];
            if dst_bank_control.enabled() && dst_bank_control.mst() == 0 {
                let capture_width_shift = 7 + (self.capture_control.size() != 0) as u8;

                let dst_bank = match dst_bank_index {
                    0 => vram.banks.a.as_ptr(),
                    1 => vram.banks.b.as_ptr(),
                    2 => vram.banks.c.as_ptr(),
                    _ => vram.banks.d.as_ptr(),
                };

                let dst_offset = (((self.capture_control.dst_offset_raw() as usize) << 15)
                    + ((vcount as usize) << (1 + capture_width_shift)))
                    & 0x1_FFFE;

                let dst_line = unsafe { dst_bank.add(dst_offset) as *mut u16 };

                let capture_source = self.capture_control.src();
                let factor_a = self.capture_control.factor_a().min(16) as u16;
                let factor_b = self.capture_control.factor_b().min(16) as u16;

                let src_b_line = if capture_source != 0
                    && (factor_b != 0 || capture_source & 2 == 0)
                {
                    if self.capture_control.src_b_display_fifo() {
                        todo!("Display capture display FIFO source");
                    } else {
                        let src_bank_index = self.control.a_vram_bank();
                        let src_bank_control = vram.bank_control()[src_bank_index as usize];
                        if src_bank_control.enabled() && src_bank_control.mst() == 0 {
                            let src_bank = match src_bank_index {
                                0 => vram.banks.a.as_ptr(),
                                1 => vram.banks.b.as_ptr(),
                                2 => vram.banks.c.as_ptr(),
                                _ => vram.banks.d.as_ptr(),
                            };

                            let src_offset = if self.control.display_mode_a() == 2 {
                                (vcount as usize) << 9
                            } else {
                                (((self.capture_control.src_b_vram_offset_raw() as usize) << 15)
                                    + ((vcount as usize) << 9))
                                    & 0x1_FFFE
                            };

                            Some(unsafe { src_bank.add(src_offset) as *const u16 })
                        } else {
                            None
                        }
                    }
                } else {
                    None
                };

                unsafe {
                    if capture_source == 1
                        || (capture_source & 2 != 0 && factor_a == 0)
                        || (self.capture_control.src_a_3d_only()
                            && !self.engine_3d_enabled_in_frame)
                    {
                        if let Some(src_b_line) = src_b_line {
                            if src_b_line != dst_line {
                                dst_line
                                    .copy_from_nonoverlapping(src_b_line, 1 << capture_width_shift);
                            }
                        } else {
                            dst_line.write_bytes(0, 1 << capture_width_shift);
                        }
                    } else if self.capture_control.src_a_3d_only() {
                        let scanline_3d = scanline_3d.unwrap_unchecked();
                        if let Some(src_b_line) = src_b_line {
                            for x in 0..1 << capture_width_shift {
                                let a_pixel = scanline_3d.0[x];
                                let a_r = (a_pixel >> 1) as u16 & 0x1F;
                                let a_g = (a_pixel >> 7) as u16 & 0x1F;
                                let a_b = (a_pixel >> 13) as u16 & 0x1F;
                                let a_a = (a_pixel >> 18 & 0x1F != 0) as u16;

                                let b_pixel = src_b_line.add(x).read();
                                let b_r = b_pixel & 0x1F;
                                let b_g = (b_pixel >> 5) & 0x1F;
                                let b_b = (b_pixel >> 10) & 0x1F;
                                let b_a = b_pixel >> 15;

                                let r = (((a_r * a_a * factor_a) + (b_r * b_a * factor_b)) >> 4)
                                    .min(0x1F);
                                let g = (((a_g * a_a * factor_a) + (b_g * b_a * factor_b)) >> 4)
                                    .min(0x1F);
                                let b = (((a_b * a_a * factor_a) + (b_b * b_a * factor_b)) >> 4)
                                    .min(0x1F);
                                let a = a_a | b_a;

                                dst_line.add(x).write(r | g << 5 | b << 10 | a << 15);
                            }
                        } else {
                            for x in 0..1 << capture_width_shift {
                                let pixel = scanline_3d.0[x];
                                let r = (pixel >> 1) as u16 & 0x1F;
                                let g = (pixel >> 7) as u16 & 0x1F;
                                let b = (pixel >> 13) as u16 & 0x1F;
                                let a = (pixel >> 18 & 0x1F != 0) as u16;
                                dst_line.add(x).write(r | g << 5 | b << 10 | a << 15);
                            }
                        }
                    } else if let Some(src_b_line) = src_b_line {
                        for x in 0..1 << capture_width_shift {
                            let a_pixel = self.bg_obj_scanline.0[x];
                            let a_r = (a_pixel >> 1) as u16 & 0x1F;
                            let a_g = (a_pixel >> 7) as u16 & 0x1F;
                            let a_b = (a_pixel >> 13) as u16 & 0x1F;

                            let b_pixel = src_b_line.add(x).read();
                            let b_r = b_pixel & 0x1F;
                            let b_g = (b_pixel >> 5) & 0x1F;
                            let b_b = (b_pixel >> 10) & 0x1F;
                            let b_a = b_pixel >> 15;

                            let r = (((a_r * factor_a) + (b_r * b_a * factor_b)) >> 4).min(0x1F);
                            let g = (((a_g * factor_a) + (b_g * b_a * factor_b)) >> 4).min(0x1F);
                            let b = (((a_b * factor_a) + (b_b * b_a * factor_b)) >> 4).min(0x1F);

                            dst_line.add(x).write(r | g << 5 | b << 10 | 0x8000);
                        }
                    } else {
                        for x in 0..1 << capture_width_shift {
                            let pixel = self.bg_obj_scanline.0[x];
                            let r = (pixel >> 1) as u16 & 0x1F;
                            let g = (pixel >> 7) as u16 & 0x1F;
                            let b = (pixel >> 13) as u16 & 0x1F;
                            dst_line.add(x).write(r | g << 5 | b << 10 | 0x8000);
                        }
                    }
                }
            }
        }

        match self.master_brightness_control.mode() {
            1 if self.master_brightness_factor != 0 => {
                for (dst, src) in scanline_buffer
                    .0
                    .iter_mut()
                    .zip(self.bg_obj_scanline.0.iter())
                {
                    let src = *src as u32;
                    let increment = {
                        let complement = 0x3_FFFF ^ src;
                        ((((complement & 0x3_F03F) * self.master_brightness_factor) & 0x3F_03F0)
                            | (((complement & 0xFC0) * self.master_brightness_factor) & 0xFC00))
                            >> 4
                    };
                    *dst = rgb_18_to_rgba_32(src + increment);
                }
            }

            2 if self.master_brightness_factor != 0 => {
                for (dst, src) in scanline_buffer
                    .0
                    .iter_mut()
                    .zip(self.bg_obj_scanline.0.iter())
                {
                    let src = *src as u32;
                    let decrement = {
                        ((((src & 0x3_F03F) * self.master_brightness_factor) & 0x3F_03F0)
                            | (((src & 0xFC0) * self.master_brightness_factor) & 0xFC00))
                            >> 4
                    };
                    *dst = rgb_18_to_rgba_32(src - decrement);
                }
            }

            3 => unimplemented!("Unknown 2D engine brightness mode 3"),

            _ => {
                for (dst, src) in scanline_buffer
                    .0
                    .iter_mut()
                    .zip(self.bg_obj_scanline.0.iter())
                {
                    *dst = rgb_18_to_rgba_32(*src as u32);
                }
            }
        }
    }

    fn render_scanline_bgs_and_objs<const BG_MODE: u8>(
        &mut self,
        line: u8,
        vram: &Vram,
        scanline_3d: Option<&Scanline<u32, SCREEN_WIDTH>>,
    ) {
        let render_affine = [
            Self::render_scanline_bg_affine::<false>,
            Self::render_scanline_bg_affine::<true>,
        ];

        let render_extended = [
            Self::render_scanline_bg_extended::<false>,
            Self::render_scanline_bg_extended::<true>,
        ];

        for priority in (0..4).rev() {
            if self.bgs[3].priority == priority {
                match BG_MODE {
                    0 => {
                        (self.render_fns.render_scanline_bg_text)(
                            self,
                            BgIndex::new(3),
                            line,
                            vram,
                        );
                    }
                    1..=2 => {
                        render_affine[self.bgs[3].control.affine_display_area_overflow() as usize](
                            self,
                            AffineBgIndex::new(1),
                            vram,
                        );
                    }
                    3..=5 => {
                        render_extended
                            [self.bgs[3].control.affine_display_area_overflow() as usize](
                            self,
                            AffineBgIndex::new(1),
                            vram,
                        );
                    }
                    _ => {}
                }
            }

            if self.bgs[2].priority == priority {
                match BG_MODE {
                    0..=1 | 3 => {
                        (self.render_fns.render_scanline_bg_text)(
                            self,
                            BgIndex::new(2),
                            line,
                            vram,
                        );
                    }
                    2 | 4 => {
                        render_affine[self.bgs[2].control.affine_display_area_overflow() as usize](
                            self,
                            AffineBgIndex::new(0),
                            vram,
                        );
                    }
                    5 => {
                        render_extended
                            [self.bgs[2].control.affine_display_area_overflow() as usize](
                            self,
                            AffineBgIndex::new(0),
                            vram,
                        );
                    }
                    6 => {
                        if self.bgs[2].control.affine_display_area_overflow() {
                            self.render_scanline_bg_large::<true>(vram);
                        } else {
                            self.render_scanline_bg_large::<false>(vram);
                        }
                    }
                    _ => {}
                }
            }

            if self.bgs[1].priority == priority && BG_MODE != 6 {
                (self.render_fns.render_scanline_bg_text)(self, BgIndex::new(1), line, vram);
            }

            if self.bgs[0].priority == priority {
                if R::IS_A && self.control.bg0_3d() {
                    if self.engine_3d_enabled_in_frame {
                        let scanline_3d = unsafe { scanline_3d.unwrap_unchecked() };
                        let pixel_attrs = BgObjPixel(0).with_color_effects_mask(1).with_is_3d(true);
                        // TODO: 3D layer scrolling
                        for i in 0..SCREEN_WIDTH {
                            let pixel = scanline_3d.0[i];
                            if pixel >> 19 != 0 {
                                self.bg_obj_scanline.0[i] = (self.bg_obj_scanline.0[i] as u64)
                                    << 32
                                    | ((pixel & 0x3_FFFF)
                                        | pixel_attrs.with_alpha((pixel >> 18) as u8 & 0x1F).0)
                                        as u64;
                            }
                        }
                    }
                } else if BG_MODE != 6 {
                    (self.render_fns.render_scanline_bg_text)(self, BgIndex::new(0), line, vram);
                }
            }

            for i in 0..SCREEN_WIDTH {
                if self.window.0[i].0 & 1 << 4 == 0 {
                    continue;
                }

                let obj_pixel = self.obj_scanline.0[i];
                if obj_pixel.priority() == priority {
                    let pixel_attrs = BgObjPixel(obj_pixel.0 & 0x03F8_0000)
                        .with_color_effects_mask(1 << 4)
                        .0;
                    let color = unsafe {
                        rgb_15_to_18(if obj_pixel.use_raw_color() {
                            obj_pixel.raw_color()
                        } else if obj_pixel.use_ext_pal() {
                            (if R::IS_A {
                                vram.a_obj_ext_pal.as_ptr()
                            } else {
                                vram.b_obj_ext_pal_ptr
                            } as *const u16)
                                .add(obj_pixel.pal_color() as usize)
                                .read()
                        } else {
                            vram.palette.read_le_aligned_unchecked::<u16>(
                                (!R::IS_A as usize) << 10
                                    | 0x200
                                    | (obj_pixel.pal_color() as usize) << 1,
                            )
                        } as u32)
                    };
                    self.bg_obj_scanline.0[i] =
                        self.bg_obj_scanline.0[i] << 32 | (color | pixel_attrs) as u64;
                }
            }
        }
    }

    #[allow(clippy::similar_names)]
    fn render_scanline_bg_affine<const DISPLAY_AREA_OVERFLOW: bool>(
        &mut self,
        bg_index: AffineBgIndex,
        vram: &Vram,
    ) {
        let bg_control = self.bgs[bg_index.get() as usize | 2].control;
        let affine = &mut self.affine_bg_data[bg_index.get() as usize];

        let map_base = if R::IS_A {
            self.control.a_map_base() | bg_control.map_base()
        } else {
            bg_control.map_base()
        };
        let tile_base = if R::IS_A {
            self.control.a_tile_base() + bg_control.tile_base()
        } else {
            bg_control.tile_base()
        };

        let bg_mask = 4 << bg_index.get();
        let pixel_attrs = BgObjPixel(0).with_color_effects_mask(bg_mask);

        let display_area_overflow_mask = !((0x8000 << bg_control.size_key()) - 1);

        let map_row_shift = 4 + bg_control.size_key();
        let pos_map_mask = ((1 << map_row_shift) - 1) << 11;
        let pos_y_to_map_y_shift = 11 - map_row_shift;

        let mut pos = affine.pos;

        for i in 0..SCREEN_WIDTH {
            if self.window.0[i].0 & bg_mask != 0
                && (DISPLAY_AREA_OVERFLOW || (pos[0] | pos[1]) & display_area_overflow_mask == 0)
            {
                let tile_addr = map_base
                    + ((pos[1] as u32 & pos_map_mask) >> pos_y_to_map_y_shift
                        | (pos[0] as u32 & pos_map_mask) >> 11);
                let tile = if R::IS_A {
                    vram.read_a_bg::<u8>(tile_addr)
                } else {
                    vram.read_b_bg::<u8>(tile_addr)
                };
                let pixel_addr = tile_base
                    + ((tile as u32) << 6 | (pos[1] as u32 >> 5 & 0x38) | (pos[0] as u32 >> 8 & 7));
                let color_index = if R::IS_A {
                    vram.read_a_bg::<u8>(pixel_addr)
                } else {
                    vram.read_b_bg::<u8>(pixel_addr)
                };
                if color_index != 0 {
                    let color = unsafe {
                        vram.palette.read_le_aligned_unchecked::<u16>(
                            (!R::IS_A as usize) << 10 | (color_index as usize) << 1,
                        )
                    };
                    self.bg_obj_scanline.0[i] = (self.bg_obj_scanline.0[i] as u64) << 32
                        | (rgb_15_to_18(color as u32) | pixel_attrs.0) as u64;
                }
            }

            pos[0] = pos[0].wrapping_add(affine.params[0] as i32);
            pos[1] = pos[1].wrapping_add(affine.params[2] as i32);
        }

        affine.pos[0] = affine.pos[0].wrapping_add(affine.params[1] as i32);
        affine.pos[1] = affine.pos[1].wrapping_add(affine.params[3] as i32);
    }

    #[allow(clippy::similar_names)]
    fn render_scanline_bg_extended<const DISPLAY_AREA_OVERFLOW: bool>(
        &mut self,
        bg_index: AffineBgIndex,
        vram: &Vram,
    ) {
        let bg_control = self.bgs[bg_index.get() as usize | 2].control;

        let bg_mask = 4 << bg_index.get();
        let pixel_attrs = BgObjPixel(0).with_color_effects_mask(bg_mask);

        if bg_control.use_bitmap_extended_bg() {
            let data_base = bg_control.map_base() << 3;

            let (x_shift, y_shift) = match bg_control.size_key() {
                0 => (0, 0),
                1 => (1, 1),
                2 => (2, 1),
                _ => (2, 2),
            };

            let display_area_x_overflow_mask = !((0x8000 << x_shift) - 1);
            let display_area_y_overflow_mask = !((0x8000 << y_shift) - 1);

            let pos_x_map_mask = ((0x80 << x_shift) - 1) << 8;
            let pos_y_map_mask = ((0x80 << y_shift) - 1) << 8;

            let affine = &self.affine_bg_data[bg_index.get() as usize];
            let mut pos = affine.pos;

            if bg_control.use_direct_color_extended_bg() {
                for i in 0..SCREEN_WIDTH {
                    if self.window.0[i].0 & bg_mask != 0
                        && (DISPLAY_AREA_OVERFLOW
                            || (pos[0] & display_area_x_overflow_mask)
                                | (pos[1] & display_area_y_overflow_mask)
                                == 0)
                    {
                        let pixel_addr = data_base
                            + ((pos[1] as u32 & pos_y_map_mask) << x_shift
                                | (pos[0] as u32 & pos_x_map_mask) >> 7);
                        let color = if R::IS_A {
                            vram.read_a_bg::<u16>(pixel_addr)
                        } else {
                            vram.read_b_bg::<u16>(pixel_addr)
                        };
                        if color & 0x8000 != 0 {
                            self.bg_obj_scanline.0[i] = (self.bg_obj_scanline.0[i] as u64) << 32
                                | (rgb_15_to_18(color as u32) | pixel_attrs.0) as u64;
                        }
                    }

                    pos[0] = pos[0].wrapping_add(affine.params[0] as i32);
                    pos[1] = pos[1].wrapping_add(affine.params[2] as i32);
                }
            } else {
                for i in 0..SCREEN_WIDTH {
                    if self.window.0[i].0 & bg_mask != 0
                        && (DISPLAY_AREA_OVERFLOW
                            || (pos[0] & display_area_x_overflow_mask)
                                | (pos[1] & display_area_y_overflow_mask)
                                == 0)
                    {
                        let pixel_addr = data_base
                            + ((pos[1] as u32 & pos_y_map_mask) >> 1 << x_shift
                                | (pos[0] as u32 & pos_x_map_mask) >> 8);
                        let color_index = if R::IS_A {
                            vram.read_a_bg::<u8>(pixel_addr)
                        } else {
                            vram.read_b_bg::<u8>(pixel_addr)
                        };
                        if color_index != 0 {
                            let color = unsafe {
                                vram.palette.read_le_aligned_unchecked::<u16>(
                                    (!R::IS_A as usize) << 10 | (color_index as usize) << 1,
                                )
                            };
                            self.bg_obj_scanline.0[i] = (self.bg_obj_scanline.0[i] as u64) << 32
                                | (rgb_15_to_18(color as u32) | pixel_attrs.0) as u64;
                        }
                    }

                    pos[0] = pos[0].wrapping_add(affine.params[0] as i32);
                    pos[1] = pos[1].wrapping_add(affine.params[2] as i32);
                }
            }
        } else {
            let map_base = if R::IS_A {
                self.control.a_map_base() | bg_control.map_base()
            } else {
                bg_control.map_base()
            };
            let tile_base = if R::IS_A {
                self.control.a_tile_base() + bg_control.tile_base()
            } else {
                bg_control.tile_base()
            };

            let display_area_overflow_mask = !((0x8000 << bg_control.size_key()) - 1);

            let map_row_shift = 4 + bg_control.size_key();
            let pos_map_mask = ((1 << map_row_shift) - 1) << 11;
            let pos_y_to_map_y_shift = 10 - map_row_shift;

            let (palette, pal_base_mask) = if self.control.bg_ext_pal_enabled() {
                (
                    unsafe {
                        if R::IS_A {
                            vram.a_bg_ext_pal.as_ptr()
                        } else {
                            vram.b_bg_ext_pal_ptr
                        }
                        .add((bg_index.get() as usize | 2) << 13)
                            as *const u16
                    },
                    0xF,
                )
            } else {
                (
                    unsafe { vram.palette.as_ptr().add((!R::IS_A as usize) << 10) as *const u16 },
                    0,
                )
            };

            let affine = &self.affine_bg_data[bg_index.get() as usize];
            let mut pos = affine.pos;

            for i in 0..SCREEN_WIDTH {
                if self.window.0[i].0 & bg_mask != 0
                    && (DISPLAY_AREA_OVERFLOW
                        || (pos[0] | pos[1]) & display_area_overflow_mask == 0)
                {
                    let tile_addr = map_base
                        + ((pos[1] as u32 & pos_map_mask) >> pos_y_to_map_y_shift
                            | (pos[0] as u32 & pos_map_mask) >> 10);
                    let tile = if R::IS_A {
                        vram.read_a_bg::<u16>(tile_addr)
                    } else {
                        vram.read_b_bg::<u16>(tile_addr)
                    };

                    let x_offset = if tile & 1 << 10 == 0 {
                        pos[0] as u32 >> 8 & 7
                    } else {
                        !pos[0] as u32 >> 8 & 7
                    };
                    let y_offset = if tile & 1 << 11 == 0 {
                        pos[1] as u32 >> 5 & 0x38
                    } else {
                        !pos[1] as u32 >> 5 & 0x38
                    };

                    let pixel_addr = tile_base + ((tile as u32 & 0x3FF) << 6 | y_offset | x_offset);
                    let color_index = if R::IS_A {
                        vram.read_a_bg::<u8>(pixel_addr)
                    } else {
                        vram.read_b_bg::<u8>(pixel_addr)
                    };

                    if color_index != 0 {
                        let pal_base = ((tile >> 12 & pal_base_mask) << 8) as usize;
                        let color = unsafe { palette.add(pal_base | color_index as usize).read() };
                        self.bg_obj_scanline.0[i] = (self.bg_obj_scanline.0[i] as u64) << 32
                            | (rgb_15_to_18(color as u32) | pixel_attrs.0) as u64;
                    }
                }

                pos[0] = pos[0].wrapping_add(affine.params[0] as i32);
                pos[1] = pos[1].wrapping_add(affine.params[2] as i32);
            }
        }

        let affine = &mut self.affine_bg_data[bg_index.get() as usize];
        affine.pos[0] = affine.pos[0].wrapping_add(affine.params[1] as i32);
        affine.pos[1] = affine.pos[1].wrapping_add(affine.params[3] as i32);
    }

    #[allow(clippy::similar_names)]
    fn render_scanline_bg_large<const DISPLAY_AREA_OVERFLOW: bool>(&mut self, vram: &Vram) {
        let bg_control = self.bgs[2].control;

        let pixel_attrs = BgObjPixel(0).with_color_effects_mask(1 << 2);

        let (x_shift, y_shift) = match bg_control.size_key() {
            0 => (1, 2),
            1 => (2, 1),
            2 => (1, 0),
            _ => (1, 1),
        };

        let display_area_x_overflow_mask = !((0x1_0000 << x_shift) - 1);
        let display_area_y_overflow_mask = !((0x1_0000 << y_shift) - 1);

        let pos_x_map_mask = ((0x100 << x_shift) - 1) << 8;
        let pos_y_map_mask = ((0x100 << y_shift) - 1) << 8;

        let affine = &mut self.affine_bg_data[0];
        let mut pos = affine.pos;

        for i in 0..SCREEN_WIDTH {
            if self.window.0[i].0 & 1 << 2 != 0
                && (DISPLAY_AREA_OVERFLOW
                    || (pos[0] & display_area_x_overflow_mask)
                        | (pos[1] & display_area_y_overflow_mask)
                        == 0)
            {
                let pixel_addr = (pos[1] as u32 & pos_y_map_mask) << x_shift
                    | (pos[0] as u32 & pos_x_map_mask) >> 8;
                let color_index = if R::IS_A {
                    vram.read_a_bg::<u8>(pixel_addr)
                } else {
                    vram.read_b_bg::<u8>(pixel_addr)
                };
                if color_index != 0 {
                    let color = unsafe {
                        vram.palette.read_le_aligned_unchecked::<u16>(
                            (!R::IS_A as usize) << 10 | (color_index as usize) << 1,
                        )
                    };
                    self.bg_obj_scanline.0[i] = (self.bg_obj_scanline.0[i] as u64) << 32
                        | (rgb_15_to_18(color as u32) | pixel_attrs.0) as u64;
                }
            }

            pos[0] = pos[0].wrapping_add(affine.params[0] as i32);
            pos[1] = pos[1].wrapping_add(affine.params[2] as i32);
        }

        affine.pos[0] = affine.pos[0].wrapping_add(affine.params[1] as i32);
        affine.pos[1] = affine.pos[1].wrapping_add(affine.params[3] as i32);
    }

    pub(in super::super) fn prerender_sprites(&mut self, scanline: u32, vram: &Vram) {
        // Arisotura confirmed that shape 3 just forces 8 pixels of size
        #[rustfmt::skip]
        static OBJ_SIZE_SHIFT: [(u8, u8); 16] = [
            (0, 0), (1, 0), (0, 1), (0, 0),
            (1, 1), (2, 0), (0, 2), (0, 0),
            (2, 2), (2, 1), (1, 2), (0, 0),
            (3, 3), (3, 2), (2, 3), (0, 0),
        ];

        #[inline]
        fn obj_size_shift(attr_0: OamAttr0, attr_1: OamAttr1) -> (u8, u8) {
            OBJ_SIZE_SHIFT[((attr_1.0 >> 12 & 0xC) | attr_0.0 >> 14) as usize]
        }

        self.obj_scanline.0.fill(ObjPixel(0).with_priority(4));
        make_zero(&mut self.obj_window);
        if !self.control.objs_enabled() {
            return;
        }
        for priority in (0..4).rev() {
            for obj_i in (0..128).rev() {
                let oam_start = (!R::IS_A as usize) << 10 | obj_i << 3;
                let attrs = unsafe {
                    let attr_2 = OamAttr2(vram.oam.read_le_aligned_unchecked::<u16>(oam_start | 4));
                    if attr_2.bg_priority() != priority {
                        continue;
                    }
                    (
                        OamAttr0(vram.oam.read_le_aligned_unchecked::<u16>(oam_start)),
                        OamAttr1(vram.oam.read_le_aligned_unchecked::<u16>(oam_start | 2)),
                        attr_2,
                    )
                };
                if attrs.0.rot_scale() {
                    let (width_shift, height_shift) = obj_size_shift(attrs.0, attrs.1);
                    let y_in_obj = (scanline as u8).wrapping_sub(attrs.0.y_start()) as u32;
                    let (bounds_width_shift, bounds_height_shift) = if attrs.0.double_size() {
                        (width_shift + 1, height_shift + 1)
                    } else {
                        (width_shift, height_shift)
                    };
                    if y_in_obj as u32 >= 8 << bounds_height_shift {
                        continue;
                    }
                    let x_start = attrs.1.x_start() as i32;
                    if x_start <= -(8 << bounds_width_shift) {
                        continue;
                    }
                    self.prerender_sprite_rot_scale(
                        attrs,
                        x_start,
                        y_in_obj as i32 - (4 << bounds_height_shift),
                        width_shift,
                        height_shift,
                        bounds_width_shift,
                        vram,
                    );
                } else {
                    if attrs.0.disabled() {
                        continue;
                    }
                    let (width_shift, height_shift) = obj_size_shift(attrs.0, attrs.1);
                    let y_in_obj = (scanline as u8).wrapping_sub(attrs.0.y_start()) as u32;
                    if y_in_obj >= 8 << height_shift {
                        continue;
                    }
                    let x_start = attrs.1.x_start() as i32;
                    if x_start <= -(8 << width_shift) {
                        continue;
                    }
                    let y_in_obj = if attrs.1.y_flip() {
                        y_in_obj ^ ((8 << height_shift) - 1)
                    } else {
                        y_in_obj
                    };
                    (if attrs.1.x_flip() {
                        Self::prerender_sprite_normal::<true>
                    } else {
                        Self::prerender_sprite_normal::<false>
                    })(
                        self,
                        (attrs.0, (), attrs.2),
                        x_start,
                        y_in_obj,
                        width_shift,
                        vram,
                    );
                }
            }
        }
    }

    #[allow(clippy::similar_names, clippy::too_many_arguments)]
    fn prerender_sprite_rot_scale(
        &mut self,
        attrs: (OamAttr0, OamAttr1, OamAttr2),
        bounds_x_start: i32,
        rel_y_in_square_obj: i32,
        width_shift: u8,
        height_shift: u8,
        bounds_width_shift: u8,
        vram: &Vram,
    ) {
        let (start_x, end_x, start_rel_x_in_square_obj) = {
            let bounds_width = 8 << bounds_width_shift;
            if bounds_x_start < 0 {
                (
                    0,
                    (bounds_x_start + bounds_width) as usize,
                    -(bounds_width >> 1) - bounds_x_start,
                )
            } else {
                (
                    bounds_x_start as usize,
                    (bounds_x_start + bounds_width).min(256) as usize,
                    -(bounds_width >> 1),
                )
            }
        };

        let params = unsafe {
            let start =
                (!R::IS_A as usize) << 10 | (attrs.1.rot_scale_params_index() as usize) << 5;
            [
                vram.oam.read_le_aligned_unchecked::<i16>(start | 0x06),
                vram.oam.read_le_aligned_unchecked::<i16>(start | 0x0E),
                vram.oam.read_le_aligned_unchecked::<i16>(start | 0x16),
                vram.oam.read_le_aligned_unchecked::<i16>(start | 0x1E),
            ]
        };

        let mut pos = [
            (0x400 << width_shift)
                + start_rel_x_in_square_obj * params[0] as i32
                + rel_y_in_square_obj * params[1] as i32,
            (0x400 << height_shift)
                + start_rel_x_in_square_obj * params[2] as i32
                + rel_y_in_square_obj * params[3] as i32,
        ];

        let obj_x_outside_mask = !((0x800 << width_shift) - 1);
        let obj_y_outside_mask = !((0x800 << height_shift) - 1);

        if attrs.0.mode() == 3 {
            let alpha = match attrs.2.palette_number() {
                0 => return,
                value => value + 1,
            };

            let tile_number = attrs.2.tile_number() as u32;

            let (tile_base, y_shift) = if self.control.obj_bitmap_1d_mapping() {
                if self.control.bitmap_objs_256x256() {
                    return;
                }
                (
                    tile_number
                        << if R::IS_A {
                            7 + self.control.a_obj_bitmap_1d_boundary()
                        } else {
                            7
                        },
                    width_shift + 1,
                )
            } else if self.control.bitmap_objs_256x256() {
                (
                    ((tile_number & 0x1F) << 4) + ((tile_number & !0x1F) << 7),
                    9,
                )
            } else {
                (((tile_number & 0xF) << 4) + ((tile_number & !0xF) << 7), 8)
            };

            let pixel_attrs = ObjPixel(0)
                .with_priority(attrs.2.bg_priority())
                .with_force_blending(true)
                .with_use_raw_color(true)
                .with_custom_alpha(true)
                .with_alpha(alpha);

            for x in start_x..end_x {
                if (pos[0] & obj_x_outside_mask) | (pos[1] & obj_y_outside_mask) == 0 {
                    let pixel_addr =
                        tile_base + (pos[0] as u32 >> 8) + (pos[1] as u32 >> 8 << y_shift);
                    let color = if R::IS_A {
                        vram.read_a_obj::<u16>(pixel_addr)
                    } else {
                        vram.read_b_obj::<u16>(pixel_addr)
                    };
                    if color & 0x8000 != 0 {
                        unsafe {
                            *self.obj_scanline.0.get_unchecked_mut(x) =
                                pixel_attrs.with_raw_color(color);
                        }
                    }
                }

                pos[0] = pos[0].wrapping_add(params[0] as i32);
                pos[1] = pos[1].wrapping_add(params[2] as i32);
            }
        } else {
            let tile_base = if R::IS_A {
                self.control.a_tile_base()
            } else {
                0
            } + {
                let tile_number = attrs.2.tile_number() as u32;
                if self.control.obj_tile_1d_mapping() {
                    tile_number << (5 + self.control.obj_tile_1d_boundary())
                } else {
                    tile_number << 5
                }
            };

            let mut pixel_attrs = ObjPixel(0)
                .with_priority(attrs.2.bg_priority())
                .with_force_blending(attrs.0.mode() == 1)
                .with_use_raw_color(false);

            if attrs.0.use_256_colors() {
                let pal_base = if self.control.obj_ext_pal_enabled() {
                    pixel_attrs.set_use_ext_pal(true);
                    (attrs.2.palette_number() as u16) << 8
                } else {
                    0
                };

                macro_rules! render {
                    ($window: expr, $y_off: expr) => {
                        for x in start_x..end_x {
                            if (pos[0] & obj_x_outside_mask) | (pos[1] & obj_y_outside_mask) == 0 {
                                let pixel_addr = {
                                    let x_off =
                                        (pos[0] as u32 >> 11 << 6) | (pos[0] as u32 >> 8 & 7);
                                    tile_base + ($y_off | x_off)
                                };
                                let color_index = if R::IS_A {
                                    vram.read_a_obj::<u8>(pixel_addr)
                                } else {
                                    vram.read_b_obj::<u8>(pixel_addr)
                                };
                                if color_index != 0 {
                                    if $window {
                                        self.obj_window[x >> 3] |= 1 << (x & 7);
                                    } else {
                                        unsafe {
                                            *self.obj_scanline.0.get_unchecked_mut(x) = pixel_attrs
                                                .with_pal_color(pal_base | color_index as u16);
                                        }
                                    }
                                }
                            }

                            pos[0] = pos[0].wrapping_add(params[0] as i32);
                            pos[1] = pos[1].wrapping_add(params[2] as i32);
                        }
                    };
                    ($window: expr) => {
                        if self.control.obj_tile_1d_mapping() {
                            render!(
                                $window,
                                (pos[1] as u32 >> 11 << (width_shift + 3)
                                    | (pos[1] as u32 >> 8 & 7))
                                    << 3
                            );
                        } else {
                            render!(
                                $window,
                                (pos[1] as u32 >> 11 << 10) | (pos[1] as u32 >> 8 & 7) << 3
                            );
                        }
                    };
                }

                if attrs.0.mode() == 2 {
                    render!(true);
                } else {
                    render!(false);
                }
            } else {
                let pal_base = (attrs.2.palette_number() as u16) << 4;

                macro_rules! render {
                    ($window: expr, $y_off: expr) => {
                        for x in start_x..end_x {
                            if (pos[0] & obj_x_outside_mask) | (pos[1] & obj_y_outside_mask) == 0 {
                                let pixel_addr = {
                                    let x_off =
                                        (pos[0] as u32 >> 11 << 5) | (pos[0] as u32 >> 9 & 3);
                                    tile_base + ($y_off | x_off)
                                };
                                let color_index = if R::IS_A {
                                    vram.read_a_obj::<u8>(pixel_addr)
                                } else {
                                    vram.read_b_obj::<u8>(pixel_addr)
                                } >> (pos[0] as u32 >> 6 & 4)
                                    & 0xF;
                                if color_index != 0 {
                                    if $window {
                                        self.obj_window[x >> 3] |= 1 << (x & 7);
                                    } else {
                                        unsafe {
                                            *self.obj_scanline.0.get_unchecked_mut(x) = pixel_attrs
                                                .with_pal_color(pal_base | color_index as u16);
                                        }
                                    }
                                }
                            }

                            pos[0] = pos[0].wrapping_add(params[0] as i32);
                            pos[1] = pos[1].wrapping_add(params[2] as i32);
                        }
                    };
                    ($window: expr) => {
                        if self.control.obj_tile_1d_mapping() {
                            render!(
                                $window,
                                (pos[1] as u32 >> 11 << (width_shift + 3)
                                    | (pos[1] as u32 >> 8 & 7))
                                    << 2
                            );
                        } else {
                            render!(
                                $window,
                                (pos[1] as u32 >> 11 << 10) | (pos[1] as u32 >> 8 & 7) << 2
                            );
                        }
                    };
                }

                if attrs.0.mode() == 2 {
                    render!(true);
                } else {
                    render!(false);
                }
            }
        }
    }

    fn prerender_sprite_normal<const X_FLIP: bool>(
        &mut self,
        attrs: (OamAttr0, (), OamAttr2),
        x_start: i32,
        y_in_obj: u32,
        width_shift: u8,
        vram: &Vram,
    ) {
        let (start_x, end_x, mut x_in_obj, x_in_obj_incr) = {
            let width = 8 << width_shift;
            let (start_x, end_x, mut x_in_obj) = if x_start < 0 {
                (0, (width + x_start) as usize, -x_start as u32)
            } else {
                (x_start as usize, (x_start + width).min(256) as usize, 0)
            };
            let x_in_obj_incr = if X_FLIP {
                x_in_obj = width as u32 - 1 - x_in_obj;
                -1_i32
            } else {
                1
            };
            (start_x, end_x, x_in_obj, x_in_obj_incr)
        };

        if attrs.0.mode() == 3 {
            let alpha = match attrs.2.palette_number() {
                0 => return,
                value => value + 1,
            };

            let tile_number = attrs.2.tile_number() as u32;

            let mut tile_base = if self.control.obj_bitmap_1d_mapping() {
                if self.control.bitmap_objs_256x256() {
                    return;
                }
                (tile_number
                    << if R::IS_A {
                        7 + self.control.a_obj_bitmap_1d_boundary()
                    } else {
                        7
                    })
                    + (y_in_obj << (width_shift + 1))
            } else if self.control.bitmap_objs_256x256() {
                ((tile_number & 0x1F) << 4) + ((tile_number & !0x1F) << 7) + (y_in_obj << 9)
            } else {
                ((tile_number & 0xF) << 4) + ((tile_number & !0xF) << 7) + (y_in_obj << 8)
            };

            let pixel_attrs = ObjPixel(0)
                .with_priority(attrs.2.bg_priority())
                .with_force_blending(true)
                .with_use_raw_color(true)
                .with_custom_alpha(true)
                .with_alpha(alpha);

            let x_in_obj_new_tile_compare = if X_FLIP { 3 } else { 0 };

            let tile_base_incr = if X_FLIP { -8_i32 } else { 8 };
            tile_base += (x_in_obj >> 3) << 4;
            let mut pixels = 0;

            macro_rules! read_pixels {
                () => {
                    pixels = if R::IS_A {
                        vram.read_a_obj::<u64>(tile_base)
                    } else {
                        vram.read_b_obj::<u64>(tile_base)
                    };
                    tile_base = tile_base.wrapping_add(tile_base_incr as u32);
                };
            }

            if x_in_obj & 3 != x_in_obj_new_tile_compare {
                read_pixels!();
            }

            for x in start_x..end_x {
                if x_in_obj & 3 == x_in_obj_new_tile_compare {
                    read_pixels!();
                }
                let color = pixels.wrapping_shr(x_in_obj << 4) as u16;
                if color & 0x8000 != 0 {
                    unsafe {
                        *self.obj_scanline.0.get_unchecked_mut(x) =
                            pixel_attrs.with_raw_color(color);
                    }
                }
                x_in_obj = x_in_obj.wrapping_add(x_in_obj_incr as u32);
            }
        } else {
            let mut tile_base = if R::IS_A {
                self.control.a_tile_base()
            } else {
                0
            } + {
                let tile_number = attrs.2.tile_number() as u32;
                if self.control.obj_tile_1d_mapping() {
                    let tile_number_off = tile_number << (5 + self.control.obj_tile_1d_boundary());
                    let y_off = ((y_in_obj & !7) << width_shift | (y_in_obj & 7))
                        << (2 | attrs.0.use_256_colors() as u8);
                    tile_number_off + y_off
                } else {
                    let tile_number_off = tile_number << 5;
                    let y_off = (y_in_obj >> 3 << 10)
                        | ((y_in_obj & 7) << (2 | attrs.0.use_256_colors() as u8));
                    tile_number_off + y_off
                }
            };

            let mut pixel_attrs = ObjPixel(0)
                .with_priority(attrs.2.bg_priority())
                .with_force_blending(attrs.0.mode() == 1)
                .with_use_raw_color(false);

            let x_in_obj_new_tile_compare = if X_FLIP { 7 } else { 0 };

            if attrs.0.use_256_colors() {
                let pal_base = if self.control.obj_ext_pal_enabled() {
                    pixel_attrs.set_use_ext_pal(true);
                    (attrs.2.palette_number() as u16) << 8
                } else {
                    0
                };

                let tile_base_incr = if X_FLIP { -64_i32 } else { 64 };
                tile_base += x_in_obj >> 3 << 6;
                let mut pixels = 0;

                macro_rules! read_pixels {
                    () => {
                        pixels = if R::IS_A {
                            vram.read_a_obj::<u64>(tile_base)
                        } else {
                            vram.read_b_obj::<u64>(tile_base)
                        };
                        tile_base = tile_base.wrapping_add(tile_base_incr as u32);
                    };
                }

                if x_in_obj & 7 != x_in_obj_new_tile_compare {
                    read_pixels!();
                }

                macro_rules! render {
                    ($window: expr) => {
                        for x in start_x..end_x {
                            if x_in_obj & 7 == x_in_obj_new_tile_compare {
                                read_pixels!();
                            }
                            let color_index = pixels.wrapping_shr(x_in_obj << 3) as u16 & 0xFF;
                            if color_index != 0 {
                                if $window {
                                    self.obj_window[x >> 3] |= 1 << (x & 7);
                                } else {
                                    unsafe {
                                        *self.obj_scanline.0.get_unchecked_mut(x) =
                                            pixel_attrs.with_pal_color(pal_base | color_index);
                                    }
                                }
                            }
                            x_in_obj = x_in_obj.wrapping_add(x_in_obj_incr as u32);
                        }
                    };
                }

                if attrs.0.mode() == 2 {
                    render!(true);
                } else {
                    render!(false);
                }
            } else {
                let pal_base = (attrs.2.palette_number() as u16) << 4;
                let tile_base_incr = if X_FLIP { -32_i32 } else { 32 };
                tile_base += x_in_obj >> 3 << 5;
                let mut pixels = 0;

                macro_rules! read_pixels {
                    () => {
                        pixels = if R::IS_A {
                            vram.read_a_obj::<u32>(tile_base)
                        } else {
                            vram.read_b_obj::<u32>(tile_base)
                        };
                        tile_base = tile_base.wrapping_add(tile_base_incr as u32);
                    };
                }

                if x_in_obj & 7 != x_in_obj_new_tile_compare {
                    read_pixels!();
                }

                macro_rules! render {
                    ($window: expr) => {
                        for x in start_x..end_x {
                            if x_in_obj & 7 == x_in_obj_new_tile_compare {
                                read_pixels!();
                            }
                            let color_index = pixels.wrapping_shr(x_in_obj << 2) as u16 & 0xF;
                            if color_index != 0 {
                                if $window {
                                    self.obj_window[x >> 3] |= 1 << (x & 7);
                                } else {
                                    unsafe {
                                        *self.obj_scanline.0.get_unchecked_mut(x) =
                                            pixel_attrs.with_pal_color(pal_base | color_index);
                                    }
                                }
                            }
                            x_in_obj = x_in_obj.wrapping_add(x_in_obj_incr as u32);
                        }
                    };
                }

                if attrs.0.mode() == 2 {
                    render!(true);
                } else {
                    render!(false);
                }
            }
        }
    }
}
