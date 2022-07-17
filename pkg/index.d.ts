/* tslint:disable */
/* eslint-disable */
/**
*/
export function run_worker(): void;
/**
* @param {Uint8Array | undefined} arm7_bios_arr
* @param {Uint8Array | undefined} arm9_bios_arr
* @param {Uint8Array | undefined} firmware_arr
* @param {Uint8Array} rom_arr
* @param {Uint8Array | undefined} save_contents_arr
* @param {number | undefined} save_type
* @param {boolean} has_ir
* @param {number} model
* @param {Function} audio_callback
* @returns {EmuState}
*/
export function create_emu_state(arm7_bios_arr: Uint8Array | undefined, arm9_bios_arr: Uint8Array | undefined, firmware_arr: Uint8Array | undefined, rom_arr: Uint8Array, save_contents_arr: Uint8Array | undefined, save_type: number | undefined, has_ir: boolean, model: number, audio_callback: Function): EmuState;
/**
* @returns {any}
*/
export function internal_get_module(): any;
/**
* @returns {any}
*/
export function internal_get_memory(): any;
/**
*/
export enum SaveType {
  None,
  Eeprom4k,
  EepromFram64k,
  EepromFram512k,
  EepromFram1m,
  Flash2m,
  Flash4m,
  Flash8m,
  Nand64m,
  Nand128m,
  Nand256m,
}
/**
*/
export enum WbgModel {
  Ds,
  Lite,
  Ique,
  IqueLite,
  Dsi,
}
/**
*/
export class EmuState {
  free(): void;
/**
*/
  reset(): void;
/**
* @param {Uint8Array} ram_arr
*/
  load_save(ram_arr: Uint8Array): void;
/**
* @returns {Uint8Array}
*/
  export_save(): Uint8Array;
/**
* @param {number} pressed
* @param {number} released
*/
  update_input(pressed: number, released: number): void;
/**
* @param {number | undefined} x
* @param {number | undefined} y
*/
  update_touch(x?: number, y?: number): void;
/**
* @returns {Uint32Array}
*/
  run_frame(): Uint32Array;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly run_worker: () => void;
  readonly __wbg_emustate_free: (a: number) => void;
  readonly emustate_reset: (a: number) => void;
  readonly emustate_load_save: (a: number, b: number) => void;
  readonly emustate_export_save: (a: number) => number;
  readonly emustate_update_input: (a: number, b: number, c: number) => void;
  readonly emustate_update_touch: (a: number, b: number, c: number) => void;
  readonly emustate_run_frame: (a: number) => number;
  readonly create_emu_state: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
  readonly internal_get_module: () => number;
  readonly internal_get_memory: () => number;
  readonly memory: WebAssembly.Memory;
  readonly __wbindgen_free: (a: number, b: number) => void;
  readonly __wbindgen_malloc: (a: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number) => number;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly __wbindgen_thread_destroy: () => void;
  readonly __wbindgen_start: () => void;
}

/**
* Synchronously compiles the given `bytes` and instantiates the WebAssembly module.
*
* @param {BufferSource} bytes
* @param {WebAssembly.Memory} maybe_memory
*
* @returns {InitOutput}
*/
export function initSync(bytes: BufferSource, maybe_memory?: WebAssembly.Memory): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {InitInput | Promise<InitInput>} module_or_path
* @param {WebAssembly.Memory} maybe_memory
*
* @returns {Promise<InitOutput>}
*/
export default function init (module_or_path?: InitInput | Promise<InitInput>, maybe_memory?: WebAssembly.Memory): Promise<InitOutput>;
