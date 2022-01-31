use super::super::{
    add_bus_cycles, add_cycles, add_interlock, apply_reg_interlock_1, apply_reg_interlocks_2,
    apply_reg_interlocks_3, can_read, can_write, handle_data_abort, handle_undefined, prefetch_arm,
    reload_pipeline, restore_spsr, write_reg_clear_interlock_ab, write_reg_interlock,
};
use crate::{
    cpu::{
        arm9::bus,
        bus::CpuAccess,
        interpreter::{
            alu_utils::shifts,
            common::{MiscAddressing, ShiftTy, StateSource, WbAddressing, WbOffTy},
            Engine,
        },
        psr::Mode,
    },
    emu::Emu,
    utils::schedule::RawTimestamp,
};
use core::intrinsics::{likely, unlikely};

// TODO: Check if the scaled offset additional internal cycle applies to LSL #0 too (it's assumed
//       that it is at the moment).
// TODO: Check how postincrement interacts with timing (at the moment it's assumed to be the same).
// TODO: Check data abort timings.

macro_rules! wb_handler {
    (
        $ident: ident,
        |
            $emu: ident,
            $instr: ident,
            $off_ty: ident,
            $addressing: ident,
            $addr: ident
            $(, src = $src_reg: ident)?
            $(, dst = $dst_reg: ident)?$(,)?
        | $inner: block$(,)?
    ) => {
        pub fn $ident<const $off_ty: WbOffTy, const UPWARDS: bool, const $addressing: WbAddressing>(
            $emu: &mut Emu<Engine>,
            $instr: u32,
        ) {
            $( let $src_reg = ($instr >> 12 & 0xF) as u8; )*
            $( let $dst_reg = ($instr >> 12 & 0xF) as u8; )*

            let base_reg = ($instr >> 16 & 0xF) as u8;
            let offset = {
                let abs_off = match $off_ty {
                    WbOffTy::Imm => {
                        $( apply_reg_interlocks_2::<0, true>($emu, base_reg, $src_reg); )*
                        $( apply_reg_interlock_1::<false>($emu, base_reg); let _ = $dst_reg; )*
                        add_bus_cycles($emu, 1);
                        $instr & 0xFFF
                    }
                    WbOffTy::Reg(shift_ty) => {
                        let off_reg = ($instr & 0xF) as u8;
                        $( apply_reg_interlocks_3::<0, true>($emu, base_reg, off_reg, $src_reg); )*
                        $(
                            apply_reg_interlocks_2::<0, false>($emu, base_reg, off_reg);
                            let _ = $dst_reg;
                        )*
                        add_bus_cycles($emu, 2);
                        let value = reg!($emu.arm9, off_reg);
                        let shift = ($instr >> 7 & 0x1F) as u8;
                        match shift_ty {
                            ShiftTy::Lsl => shifts::lsl_imm(value, shift),
                            ShiftTy::Lsr => shifts::lsr_imm(value, shift),
                            ShiftTy::Asr => shifts::asr_imm(value, shift),
                            ShiftTy::Ror => shifts::ror_imm(
                                &$emu.arm9.engine_data.regs,
                                value,
                                shift,
                            ),
                        }
                    }
                } as i32;
                if UPWARDS {
                    abs_off
                } else {
                    abs_off.wrapping_neg()
                }
            };

            let $addr = if $addressing.preincrement() {
                reg!($emu.arm9, base_reg).wrapping_add(offset as u32)
            } else {
                reg!($emu.arm9, base_reg)
            };
            prefetch_arm::<false, true>($emu);
            if matches!($off_ty, WbOffTy::Reg(_)) {
                add_cycles($emu, 1);
            }

            $inner

            if $addressing.writeback() $(&& $dst_reg != base_reg)* {
                #[cfg(feature = "interp-r15-write-checks")]
                if unlikely(base_reg == 15) {
                    unimplemented!(concat!(stringify!($ident), " r15 writeback"));
                }
                write_reg_clear_interlock_ab($emu, base_reg, if $addressing.preincrement() {
                    $addr
                } else {
                    $addr.wrapping_add(offset as u32)
                });
            }
        }
    };
}

wb_handler! {
    ldr,
    |emu, instr, OFF_TY, ADDRESSING, addr, dst = dst_reg| {
        if unlikely(!can_read(
            emu,
            addr,
            ADDRESSING != WbAddressing::PostUser && emu.arm9.engine_data.regs.is_in_priv_mode(),
        )) {
            emu.arm9.engine_data.data_cycles = 1;
            if OFF_TY == WbOffTy::Imm {
                add_bus_cycles(emu, 1);
            }
            add_cycles(emu, 1);
            return handle_data_abort::<false>(emu, addr);
        }
        let result = bus::read_32::<CpuAccess, _, false>(emu, addr).rotate_right((addr & 3) << 3);
        let cycles = bus::timing_32::<_, true, false>(emu, addr);
        if dst_reg == 15 {
            emu.arm9.engine_data.data_cycles = 1;
            if OFF_TY == WbOffTy::Imm {
                add_bus_cycles(emu, 1);
            }
            add_cycles(emu, cycles as RawTimestamp + 1);
            reg!(emu.arm9, 15) = result;
            if emu.arm9.cp15.control().t_bit_load_disabled() {
                reload_pipeline::<{ StateSource::Arm }>(emu);
            } else {
                reload_pipeline::<{ StateSource::R15Bit0 }>(emu);
            }
        } else {
            emu.arm9.engine_data.data_cycles = cycles;
            write_reg_interlock(
                emu,
                dst_reg,
                result,
                1 + (addr & 3 != 0) as RawTimestamp,
                1,
            );
        }
    },
}

wb_handler! {
    str,
    |emu, instr, OFF_TY, ADDRESSING, addr, src = src_reg| {
        if unlikely(!can_write(
            emu,
            addr,
            ADDRESSING != WbAddressing::PostUser && emu.arm9.engine_data.regs.is_in_priv_mode(),
        )) {
            emu.arm9.engine_data.data_cycles = 1;
            if OFF_TY == WbOffTy::Imm {
                add_bus_cycles(emu, 1);
            }
            add_cycles(emu, 1);
            return handle_data_abort::<false>(emu, addr);
        }
        bus::write_32::<CpuAccess, _>(emu, addr, reg!(emu.arm9, src_reg));
        emu.arm9.engine_data.data_cycles = bus::timing_32::<_, false, false>(emu, addr);
    },
}

wb_handler! {
    ldrb,
    |emu, instr, OFF_TY, ADDRESSING, addr, dst = dst_reg| {
        if unlikely(!can_read(
            emu,
            addr,
            ADDRESSING != WbAddressing::PostUser && emu.arm9.engine_data.regs.is_in_priv_mode(),
        )) {
            emu.arm9.engine_data.data_cycles = 1;
            if OFF_TY == WbOffTy::Imm {
                add_bus_cycles(emu, 1);
            }
            add_cycles(emu, 1);
            return handle_data_abort::<false>(emu, addr);
        }
        let result = bus::read_8::<CpuAccess, _>(emu, addr) as u32;
        let cycles = bus::timing_16::<_, true>(emu, addr);
        if dst_reg == 15 {
            emu.arm9.engine_data.data_cycles = 1;
            if OFF_TY == WbOffTy::Imm {
                add_bus_cycles(emu, 1);
            }
            add_cycles(emu, cycles as RawTimestamp + 1);
            reg!(emu.arm9, 15) = result;
            if emu.arm9.cp15.control().t_bit_load_disabled() {
                reload_pipeline::<{ StateSource::Arm }>(emu);
            } else {
                reload_pipeline::<{ StateSource::R15Bit0 }>(emu);
            }
        } else {
            emu.arm9.engine_data.data_cycles = cycles;
            write_reg_interlock(emu, dst_reg, result, 2, 1);
        }
    },
}

wb_handler! {
    strb,
    |emu, instr, OFF_TY, ADDRESSING, addr, src = src_reg| {
        if unlikely(!can_write(
            emu,
            addr,
            ADDRESSING != WbAddressing::PostUser && emu.arm9.engine_data.regs.is_in_priv_mode(),
        )) {
            emu.arm9.engine_data.data_cycles = 1;
            if OFF_TY == WbOffTy::Imm {
                add_bus_cycles(emu, 1);
            }
            add_cycles(emu, 1);
            return handle_data_abort::<false>(emu, addr);
        }
        bus::write_8::<CpuAccess, _>(emu, addr, reg!(emu.arm9, src_reg) as u8);
        emu.arm9.engine_data.data_cycles = bus::timing_16::<_, false>(emu, addr);
    },
}

macro_rules! misc_handler {
    (
        $ident: ident,
        |
            $emu: ident,
            $instr: ident,
            $addr: ident
            $(, src = $src_reg: ident)?
            $(, dst = $dst_reg: ident)?$(,)?
        | $inner: block$(,)?
    ) => {
        pub fn $ident<const OFF_IMM: bool, const UPWARDS: bool, const ADDRESSING: MiscAddressing>(
            $emu: &mut Emu<Engine>,
            $instr: u32,
        ) {
            $( let $src_reg = ($instr >> 12 & 0xF) as u8; )*
            $( let $dst_reg = ($instr >> 12 & 0xF) as u8; )*

            let base_reg = ($instr >> 16 & 0xF) as u8;
            let offset = {
                let abs_off = if OFF_IMM {
                    $( apply_reg_interlocks_2::<0, true>($emu, base_reg, $src_reg); )*
                    $( apply_reg_interlock_1::<false>($emu, base_reg); let _ = $dst_reg; )*
                    ($instr & 0xF) | ($instr >> 4 & 0xF0)
                } else {
                    let off_reg = ($instr & 0xF) as u8;
                    $( apply_reg_interlocks_3::<0, true>($emu, base_reg, off_reg, $src_reg); )*
                    $(
                        apply_reg_interlocks_2::<0, false>($emu, base_reg, off_reg);
                        let _ = $dst_reg;
                    )*
                    reg!($emu.arm9, off_reg)
                } as i32;
                if UPWARDS {
                    abs_off
                } else {
                    abs_off.wrapping_neg()
                }
            };
            add_bus_cycles($emu, 1);

            let $addr = if ADDRESSING.preincrement() {
                reg!($emu.arm9, base_reg).wrapping_add(offset as u32)
            } else {
                reg!($emu.arm9, base_reg)
            };
            prefetch_arm::<false, true>($emu);

            $inner

            if ADDRESSING.writeback() $(&& $dst_reg != base_reg)* {
                #[cfg(feature = "interp-r15-write-checks")]
                if unlikely(base_reg == 15) {
                    unimplemented!(concat!(stringify!($ident), " r15 writeback"));
                }
                write_reg_clear_interlock_ab($emu, base_reg, if ADDRESSING.preincrement() {
                    $addr
                } else {
                    $addr.wrapping_add(offset as u32)
                });
            }
        }
    }
}

misc_handler! {
    ldrh,
    |emu, instr, addr, dst = dst_reg| {
        if unlikely(!can_read(
            emu,
            addr,
            emu.arm9.engine_data.regs.is_in_priv_mode(),
        )) {
            emu.arm9.engine_data.data_cycles = 1;
            add_bus_cycles(emu, 1);
            add_cycles(emu, 1);
            return handle_data_abort::<false>(emu, addr);
        }
        let result = bus::read_16::<CpuAccess, _>(emu, addr) as u32;
        let cycles = bus::timing_16::<_, true>(emu, addr);
        if dst_reg == 15 {
            emu.arm9.engine_data.data_cycles = 1;
            add_bus_cycles(emu, 1);
            add_cycles(emu, cycles as RawTimestamp + 1);
            reg!(emu.arm9, 15) = result;
            if emu.arm9.cp15.control().t_bit_load_disabled() {
                reload_pipeline::<{ StateSource::Arm }>(emu);
            } else {
                reload_pipeline::<{ StateSource::R15Bit0 }>(emu);
            }
        } else {
            emu.arm9.engine_data.data_cycles = cycles;
            write_reg_interlock(emu, dst_reg, result, 2, 1);
        }
    },
}

misc_handler! {
    strh,
    |emu, instr, addr, src = src_reg| {
        if unlikely(!can_write(
            emu,
            addr,
            emu.arm9.engine_data.regs.is_in_priv_mode(),
        )) {
            emu.arm9.engine_data.data_cycles = 1;
            add_bus_cycles(emu, 3);
            add_cycles(emu, 1);
            return handle_data_abort::<false>(emu, addr);
        }
        bus::write_16::<CpuAccess, _>(
            emu,
            addr,
            reg!(emu.arm9, src_reg) as u16,
        );
        emu.arm9.engine_data.data_cycles = bus::timing_16::<_, false>(emu, addr);
    },
}

pub fn ldrd<const OFF_IMM: bool, const UPWARDS: bool, const ADDRESSING: MiscAddressing>(
    emu: &mut Emu<Engine>,
    instr: u32,
) {
    let dst_base_reg = (instr >> 12 & 0xF) as u8;
    if dst_base_reg & 1 != 0 {
        return handle_undefined::<false>(emu);
    }

    let base_reg = (instr >> 16 & 0xF) as u8;

    let offset = {
        let abs_off = if OFF_IMM {
            apply_reg_interlock_1::<false>(emu, base_reg);
            (instr & 0xF) | (instr >> 4 & 0xF0)
        } else {
            let off_reg = (instr & 0xF) as u8;
            apply_reg_interlocks_2::<0, false>(emu, base_reg, off_reg);
            reg!(emu.arm9, off_reg)
        } as i32;
        if UPWARDS {
            abs_off
        } else {
            abs_off.wrapping_neg()
        }
    };
    add_bus_cycles(emu, 2);

    let start_addr = if ADDRESSING.preincrement() {
        reg!(emu.arm9, base_reg).wrapping_add(offset as u32)
    } else {
        reg!(emu.arm9, base_reg)
    };

    prefetch_arm::<false, true>(emu);

    macro_rules! do_read {
        (
            $i: expr, $is_r15: expr,
            $addr: expr => ($dst_reg: expr, $data_cycles: expr)
            $(, use $use_data_cycles: ident)?
        ) => {
            let addr = $addr;

            $(
                add_cycles(emu, $use_data_cycles as RawTimestamp);
            )*

            if unlikely(!can_read(emu, addr, emu.arm9.engine_data.regs.is_in_priv_mode())) {
                // Should behave in the same way as an LDM, see the corresponding comment
                emu.arm9.engine_data.data_cycles = 1;
                add_cycles(emu, (1 - $i) + 1);
                return handle_data_abort::<false>(emu, addr);
            }

            reg!(emu.arm9, $dst_reg) = bus::read_32::<CpuAccess, _, false>(emu, addr);
            let timings = emu.arm9.cp15.timings.get(addr);
            let cycles = if $i == 0 || addr & 0x3FC == 0 {
                timings.r_n32_data
            } else {
                timings.r_s32_data
            };
            if $is_r15 {
                add_cycles(emu, cycles as RawTimestamp + 1);
                if emu.arm9.cp15.control().t_bit_load_disabled() {
                    reload_pipeline::<{ StateSource::Arm }>(emu);
                } else {
                    reload_pipeline::<{ StateSource::R15Bit0 }>(emu);
                }
            } else {
                $data_cycles = cycles;
            }
        }
    }

    #[allow(clippy::needless_late_init)]
    let mut first_data_cycles = 0;
    do_read!(
        0, false,
        start_addr => (dst_base_reg, first_data_cycles)
    );
    do_read!(
        1, dst_base_reg == 14,
        start_addr.wrapping_add(4) => (dst_base_reg | 1, emu.arm9.engine_data.data_cycles),
        use first_data_cycles
    );

    if ADDRESSING.writeback() && dst_base_reg | 1 != base_reg {
        #[cfg(feature = "interp-r15-write-checks")]
        if unlikely(base_reg == 15) {
            unimplemented!("ldrd r15 writeback");
        }
        write_reg_clear_interlock_ab(
            emu,
            base_reg,
            if ADDRESSING.preincrement() {
                start_addr
            } else {
                start_addr.wrapping_add(offset as u32)
            },
        );
    }
}

pub fn strd<const OFF_IMM: bool, const UPWARDS: bool, const ADDRESSING: MiscAddressing>(
    emu: &mut Emu<Engine>,
    instr: u32,
) {
    let src_base_reg = (instr >> 12 & 0xF) as u8;
    if src_base_reg & 1 != 0 {
        return handle_undefined::<false>(emu);
    }

    let base_reg = (instr >> 16 & 0xF) as u8;

    let offset = {
        let abs_off = if OFF_IMM {
            apply_reg_interlock_1::<false>(emu, base_reg);
            (instr & 0xF) | (instr >> 4 & 0xF0)
        } else {
            let off_reg = (instr & 0xF) as u8;
            apply_reg_interlocks_2::<0, false>(emu, base_reg, off_reg);
            reg!(emu.arm9, off_reg)
        } as i32;
        if UPWARDS {
            abs_off
        } else {
            abs_off.wrapping_neg()
        }
    };

    let start_addr = if ADDRESSING.preincrement() {
        reg!(emu.arm9, base_reg).wrapping_add(offset as u32)
    } else {
        reg!(emu.arm9, base_reg)
    };

    prefetch_arm::<false, true>(emu);

    macro_rules! do_write {
        (
            $i: expr,
            $src_reg: expr => $addr: expr => $data_cycles: expr
            $(, use $use_data_cycles: ident)?
        ) => {
            let addr = $addr;

            if $i == 0 {
                apply_reg_interlock_1::<true>(emu, $src_reg);
            }

            $(
                add_cycles(emu, $use_data_cycles as RawTimestamp);
            )*

            if unlikely(!can_write(emu, addr, emu.arm9.engine_data.regs.is_in_priv_mode())) {
                // Should behave in the same way as an STM, see the corresponding comment
                emu.arm9.engine_data.data_cycles = 1;
                add_bus_cycles(emu, 2);
                add_cycles(emu, (1 - $i) + 1);
                return handle_data_abort::<false>(emu, addr);
            }

            bus::write_32::<CpuAccess, _>(emu, addr, reg!(emu.arm9, $src_reg));
            let timings = emu.arm9.cp15.timings.get(addr);
            $data_cycles = if $i == 0 || addr & 0x3FC == 0 {
                timings.w_n32_data
            } else {
                timings.w_s32_data
            };
        }
    }

    #[allow(clippy::needless_late_init)]
    let first_data_cycles;
    do_write!(
        0,
        src_base_reg => start_addr => first_data_cycles
    );
    do_write!(
        1,
        src_base_reg | 1 => start_addr.wrapping_add(4) => emu.arm9.engine_data.data_cycles,
        use first_data_cycles
    );

    add_bus_cycles(emu, 2);

    if ADDRESSING.writeback() {
        #[cfg(feature = "interp-r15-write-checks")]
        if unlikely(base_reg == 15) {
            unimplemented!("strd r15 writeback");
        }
        write_reg_clear_interlock_ab(
            emu,
            base_reg,
            if ADDRESSING.preincrement() {
                start_addr
            } else {
                start_addr.wrapping_add(offset as u32)
            },
        );
    }
}

misc_handler! {
    ldrsb,
    |emu, instr, addr, dst = dst_reg| {
        if unlikely(!can_read(
            emu,
            addr,
            emu.arm9.engine_data.regs.is_in_priv_mode(),
        )) {
            emu.arm9.engine_data.data_cycles = 1;
            add_bus_cycles(emu, 1);
            add_cycles(emu, 1);
            return handle_data_abort::<false>(emu, addr);
        }
        let result = bus::read_8::<CpuAccess, _>(emu, addr) as i8 as u32;
        let cycles = bus::timing_16::<_, true>(emu, addr);
        if dst_reg == 15 {
            emu.arm9.engine_data.data_cycles = 1;
            add_bus_cycles(emu, 1);
            add_cycles(emu, cycles as RawTimestamp + 1);
            reg!(emu.arm9, 15) = result;
            if emu.arm9.cp15.control().t_bit_load_disabled() {
                reload_pipeline::<{ StateSource::Arm }>(emu);
            } else {
                reload_pipeline::<{ StateSource::R15Bit0 }>(emu);
            }
        } else {
            emu.arm9.engine_data.data_cycles = cycles;
            write_reg_interlock(emu, dst_reg, result, 2, 1);
        }
    },
}

misc_handler! {
    ldrsh,
    |emu, instr, addr, dst = dst_reg| {
        if unlikely(!can_read(
            emu,
            addr,
            emu.arm9.engine_data.regs.is_in_priv_mode(),
        )) {
            emu.arm9.engine_data.data_cycles = 1;
            add_bus_cycles(emu, 1);
            add_cycles(emu, 1);
            return handle_data_abort::<false>(emu, addr);
        }
        let result = bus::read_16::<CpuAccess, _>(emu, addr) as i16 as u32;
        let cycles = bus::timing_16::<_, true>(emu, addr);
        if dst_reg == 15 {
            emu.arm9.engine_data.data_cycles = 1;
            add_bus_cycles(emu, 1);
            add_cycles(emu, cycles as RawTimestamp + 1);
            reg!(emu.arm9, 15) = result;
            if emu.arm9.cp15.control().t_bit_load_disabled() {
                reload_pipeline::<{ StateSource::Arm }>(emu);
            } else {
                reload_pipeline::<{ StateSource::R15Bit0 }>(emu);
            }
        } else {
            emu.arm9.engine_data.data_cycles = cycles;
            write_reg_interlock(emu, dst_reg, result, 2, 1);
        }
    },
}

pub fn swp(emu: &mut Emu<Engine>, instr: u32) {
    let addr_reg = (instr >> 16 & 0xF) as u8;
    apply_reg_interlock_1::<false>(emu, addr_reg);
    add_bus_cycles(emu, 2);
    let addr = reg!(emu.arm9, addr_reg);
    prefetch_arm::<false, true>(emu);
    // can_write implies can_read
    if unlikely(!can_write(
        emu,
        addr,
        emu.arm9.engine_data.regs.is_in_priv_mode(),
    )) {
        emu.arm9.engine_data.data_cycles = 1;
        add_cycles(emu, 1);
        return handle_data_abort::<false>(emu, addr);
    }
    let loaded_value = bus::read_32::<CpuAccess, _, false>(emu, addr).rotate_right((addr & 3) << 3);
    let load_cycles = bus::timing_32::<_, true, false>(emu, addr);
    add_cycles(emu, load_cycles as RawTimestamp);
    bus::write_32::<CpuAccess, _>(emu, addr, reg!(emu.arm9, instr & 0xF));
    emu.arm9.engine_data.data_cycles = bus::timing_32::<_, false, false>(emu, addr);
    let dst_reg = (instr >> 12 & 0xF) as u8;
    if likely(!cfg!(feature = "interp-r15-write-checks") || dst_reg != 15) {
        write_reg_interlock(
            emu,
            dst_reg,
            loaded_value,
            1 + (addr & 3 != 0) as RawTimestamp,
            1,
        );
    }
}

pub fn swpb(emu: &mut Emu<Engine>, instr: u32) {
    let addr_reg = (instr >> 16 & 0xF) as u8;
    apply_reg_interlock_1::<false>(emu, addr_reg);
    add_bus_cycles(emu, 2);
    let addr = reg!(emu.arm9, addr_reg);
    prefetch_arm::<false, true>(emu);
    // can_write implies can_read
    if unlikely(!can_write(
        emu,
        addr,
        emu.arm9.engine_data.regs.is_in_priv_mode(),
    )) {
        emu.arm9.engine_data.data_cycles = 1;
        add_cycles(emu, 1);
        return handle_data_abort::<false>(emu, addr);
    }
    let loaded_value = bus::read_8::<CpuAccess, _>(emu, addr) as u32;
    let load_cycles = bus::timing_16::<_, true>(emu, addr);
    add_cycles(emu, load_cycles as RawTimestamp);
    bus::write_8::<CpuAccess, _>(emu, addr, reg!(emu.arm9, instr & 0xF) as u8);
    emu.arm9.engine_data.data_cycles = bus::timing_16::<_, false>(emu, addr);
    let dst_reg = (instr >> 12 & 0xF) as u8;
    if likely(!cfg!(feature = "interp-r15-write-checks") || dst_reg != 15) {
        write_reg_interlock(emu, dst_reg, loaded_value, 2, 1);
    }
}

// NOTE: Here, `prefetch_arm` can be called before applying stored register interlocks, as they
//       happen in the execute stage, after the fetch has been initiated.
// TODO: Check timing after data aborts and with empty reg lists.
// TODO: Check what happens if both the S (bank switch, when not loading r15) and W (writeback) bits
//       are set at the same time (right now, the wrong register is updated).
// TODO: Check how bank switching interacts with timing.

pub fn ldm<const UPWARDS: bool, const PREINC: bool, const WRITEBACK: bool, const S_BIT: bool>(
    emu: &mut Emu<Engine>,
    instr: u32,
) {
    let base_reg = (instr >> 16 & 0xF) as u8;
    #[cfg(feature = "interp-r15-write-checks")]
    if unlikely(WRITEBACK && base_reg == 15) {
        unimplemented!("LDM r15 writeback");
    }
    apply_reg_interlock_1::<false>(emu, base_reg);
    add_bus_cycles(emu, 2);
    let base = reg!(emu.arm9, base_reg);
    prefetch_arm::<false, true>(emu);
    if unlikely(instr as u16 == 0) {
        add_cycles(emu, 1);
        if WRITEBACK {
            reg!(emu.arm9, base_reg) = if UPWARDS {
                base.wrapping_add(0x40)
            } else {
                base.wrapping_sub(0x40)
            };
        }
        emu.arm9.engine_data.data_cycles = 1;
        return;
    }
    let start_addr = if UPWARDS {
        base
    } else {
        base.wrapping_sub((instr as u16).count_ones() << 2)
    };
    let mut cur_addr = start_addr;
    if S_BIT && instr & 1 << 15 == 0 {
        emu.arm9
            .engine_data
            .regs
            .update_mode::<true>(emu.arm9.engine_data.regs.cpsr.mode(), Mode::User);
    }
    if PREINC {
        cur_addr = cur_addr.wrapping_add(4);
    }
    let mut not_first = false;
    let mut timings = emu.arm9.cp15.timings.get(cur_addr);
    let mut access_cycles = timings.r_n32_data;
    for reg in 0..15 {
        if instr & 1 << reg != 0 {
            if not_first {
                add_cycles(emu, emu.arm9.engine_data.data_cycles as RawTimestamp);
            }
            if unlikely(!can_read(
                emu,
                cur_addr,
                emu.arm9.engine_data.regs.is_in_priv_mode(),
            )) {
                // In case of a data abort, the instruction runs to completion before triggering
                // the exception (unclear what that means for timings, it's assumed all accesses
                // from that point on take 1 cycle)
                emu.arm9.engine_data.data_cycles = 1;
                if S_BIT && instr & 1 << 15 == 0 {
                    emu.arm9
                        .engine_data
                        .regs
                        .update_mode::<true>(Mode::User, emu.arm9.engine_data.regs.cpsr.mode());
                }
                for reg in reg + 1..16 {
                    if instr & 1 << reg != 0 {
                        add_cycles(emu, 1);
                    }
                }
                add_cycles(emu, 1);
                return handle_data_abort::<false>(emu, cur_addr);
            }
            reg!(emu.arm9, reg) = bus::read_32::<CpuAccess, _, false>(emu, cur_addr);
            emu.arm9.engine_data.data_cycles = access_cycles;
            cur_addr = cur_addr.wrapping_add(4);
            if cur_addr & 0x3FC == 0 {
                timings = emu.arm9.cp15.timings.get(cur_addr);
                access_cycles = timings.r_n32_data;
            } else {
                access_cycles = timings.r_s32_data;
            }
            not_first = true;
        }
    }
    if instr & 1 << 15 == 0 {
        if S_BIT {
            emu.arm9
                .engine_data
                .regs
                .update_mode::<true>(Mode::User, emu.arm9.engine_data.regs.cpsr.mode());
        }
        if instr as u16 & (instr as u16 - 1) == 0 {
            // Only one register present, add an internal cycle
            add_cycles(emu, emu.arm9.engine_data.data_cycles as RawTimestamp);
            emu.arm9.engine_data.data_cycles = 1;
        } else if !S_BIT {
            let last_reg = (15 - (instr as u16).leading_zeros()) as u8;
            add_interlock(emu, last_reg, 1, 1);
        }
    } else {
        if not_first {
            add_cycles(emu, emu.arm9.engine_data.data_cycles as RawTimestamp);
        }
        emu.arm9.engine_data.data_cycles = 1;
        if unlikely(!can_read(
            emu,
            cur_addr,
            emu.arm9.engine_data.regs.is_in_priv_mode(),
        )) {
            add_cycles(emu, 1);
            return handle_data_abort::<false>(emu, cur_addr);
        }
        reg!(emu.arm9, 15) = bus::read_32::<CpuAccess, _, false>(emu, cur_addr);
        add_cycles(emu, access_cycles as RawTimestamp + 1);
        if S_BIT {
            restore_spsr(emu);
            reload_pipeline::<{ StateSource::Cpsr }>(emu);
        } else if emu.arm9.cp15.control().t_bit_load_disabled() {
            reload_pipeline::<{ StateSource::Arm }>(emu);
        } else {
            reload_pipeline::<{ StateSource::R15Bit0 }>(emu);
        }
        cur_addr = cur_addr.wrapping_add(4);
    }
    if WRITEBACK
        && likely(
            instr & 1 << base_reg == 0
                || instr as u16 == 1 << base_reg
                || (instr & !((2 << base_reg) - 1)) as u16 != 0,
        )
    {
        reg!(emu.arm9, base_reg) = if UPWARDS {
            cur_addr.wrapping_sub((PREINC as u32) << 2)
        } else {
            start_addr
        };
    }
}

pub fn stm<
    const UPWARDS: bool,
    const PREINC: bool,
    const WRITEBACK: bool,
    const BANK_SWITCH: bool,
>(
    emu: &mut Emu<Engine>,
    instr: u32,
) {
    let base_reg = (instr >> 16 & 0xF) as u8;
    #[cfg(feature = "interp-r15-write-checks")]
    if unlikely(WRITEBACK && base_reg == 15) {
        unimplemented!("STM r15 writeback");
    }
    apply_reg_interlock_1::<false>(emu, base_reg);
    if BANK_SWITCH {
        add_bus_cycles(emu, 2);
    }
    let base = reg!(emu.arm9, base_reg);
    prefetch_arm::<false, true>(emu);
    if unlikely(instr as u16 == 0) {
        if !BANK_SWITCH {
            add_bus_cycles(emu, 2);
        }
        add_cycles(emu, 1);
        if WRITEBACK {
            reg!(emu.arm9, base_reg) = if UPWARDS {
                base.wrapping_add(0x40)
            } else {
                base.wrapping_sub(0x40)
            };
        }
        emu.arm9.engine_data.data_cycles = 1;
        return;
    }
    let start_addr = if UPWARDS {
        base
    } else {
        base.wrapping_sub((instr as u16).count_ones() << 2)
    };
    let mut cur_addr = start_addr;
    if BANK_SWITCH {
        emu.arm9
            .engine_data
            .regs
            .update_mode::<true>(emu.arm9.engine_data.regs.cpsr.mode(), Mode::User);
    }
    if PREINC {
        cur_addr = cur_addr.wrapping_add(4);
    }
    let mut not_first = false;
    let mut timings = emu.arm9.cp15.timings.get(cur_addr);
    let mut access_cycles = timings.w_n32_data;
    for reg in 0..16 {
        if instr & 1 << reg != 0 {
            if not_first {
                add_cycles(emu, emu.arm9.engine_data.data_cycles as RawTimestamp);
            } else if !BANK_SWITCH {
                apply_reg_interlock_1::<true>(emu, reg);
            }
            if unlikely(!can_write(
                emu,
                cur_addr,
                emu.arm9.engine_data.regs.is_in_priv_mode(),
            )) {
                // In case of a data abort, the instruction runs to completion before triggering
                // the exception (unclear what that means for timings, it's assumed all accesses
                // from that point on take 1 cycle).
                emu.arm9.engine_data.data_cycles = 1;
                if BANK_SWITCH {
                    emu.arm9
                        .engine_data
                        .regs
                        .update_mode::<true>(Mode::User, emu.arm9.engine_data.regs.cpsr.mode());
                } else {
                    add_bus_cycles(emu, 2);
                }
                for reg in reg + 1..16 {
                    if instr & 1 << reg != 0 {
                        add_cycles(emu, 1);
                    }
                }
                add_cycles(emu, 1);
                return handle_data_abort::<false>(emu, cur_addr);
            }
            bus::write_32::<CpuAccess, _>(emu, cur_addr, reg!(emu.arm9, reg));
            emu.arm9.engine_data.data_cycles = access_cycles;
            cur_addr = cur_addr.wrapping_add(4);
            if cur_addr & 0x3FC == 0 {
                timings = emu.arm9.cp15.timings.get(cur_addr);
                access_cycles = timings.w_n32_data;
            } else {
                access_cycles = timings.w_s32_data;
            }
            not_first = true;
        }
    }
    if BANK_SWITCH {
        emu.arm9
            .engine_data
            .regs
            .update_mode::<true>(Mode::User, emu.arm9.engine_data.regs.cpsr.mode());
    } else {
        add_bus_cycles(emu, 2);
    }
    if instr as u16 & (instr as u16 - 1) == 0 {
        // Only one register present, add an internal cycle
        add_cycles(emu, emu.arm9.engine_data.data_cycles as RawTimestamp);
        emu.arm9.engine_data.data_cycles = 1;
    }
    if WRITEBACK {
        reg!(emu.arm9, base_reg) = if UPWARDS {
            cur_addr.wrapping_sub((PREINC as u32) << 2)
        } else {
            start_addr
        };
    }
}

pub fn pld(_emu: &mut Emu<Engine>, _instr: u32) {
    todo!("PLD");
}
