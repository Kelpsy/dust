#![feature(
    core_intrinsics,
    generic_const_exprs,
    generic_arg_infer,
    // rustc_attrs,
    adt_const_params,
    doc_cfg,
    maybe_uninit_uninit_array,
    maybe_uninit_slice,
    label_break_value,
    portable_simd,
    const_mut_refs,
    const_trait_impl,
    const_convert
)]
#![warn(clippy::pedantic)]
#![allow(
    incomplete_features,
    clippy::cast_lossless,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::struct_excessive_bools,
    clippy::used_underscore_binding,
    clippy::too_many_lines,
    clippy::missing_panics_doc,
    clippy::cast_ptr_alignment,
    clippy::ptr_as_ptr,
    clippy::option_if_let_else,
    clippy::module_name_repetitions,
    clippy::verbose_bit_mask,
    clippy::wildcard_imports,
    clippy::must_use_candidate,
    clippy::unused_self,
    clippy::missing_errors_doc,
    clippy::if_same_then_else, // False positives
)]

pub extern crate emu_utils as utils;

pub mod audio;
pub mod cpu;
pub mod ds_slot;
pub mod emu;
pub mod flash;
pub mod gpu;
pub mod ipc;
pub mod rtc;
pub mod spi;
pub mod wifi;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    Ds,
    Lite,
    Ique,
    IqueLite,
    Dsi,
}

impl Default for Model {
    fn default() -> Self {
        Model::Lite
    }
}

#[derive(Clone)]
pub enum SaveContents {
    Existing(utils::BoxedByteSlice),
    New(usize),
}

impl From<utils::BoxedByteSlice> for SaveContents {
    #[inline]
    fn from(other: utils::BoxedByteSlice) -> Self {
        Self::Existing(other)
    }
}

impl SaveContents {
    pub(crate) fn get_or_create(
        self,
        f: impl FnOnce(usize) -> utils::BoxedByteSlice,
    ) -> utils::BoxedByteSlice {
        match self {
            Self::Existing(data) => data,
            Self::New(len) => f(len),
        }
    }

    #[inline]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        match self {
            Self::Existing(data) => data.len(),
            Self::New(len) => *len,
        }
    }
}
