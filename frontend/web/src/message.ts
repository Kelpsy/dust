export const enum SaveType {
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

export const saveTypes = {
    none: SaveType.None,
    "eeprom-4k": SaveType.Eeprom4k,
    "eeprom-fram-64k": SaveType.EepromFram64k,
    "eeprom-fram-512k": SaveType.EepromFram512k,
    "eeprom-fram-1m": SaveType.EepromFram1m,
    "flash-2m": SaveType.Flash2m,
    "flash-4m": SaveType.Flash4m,
    "flash-8m": SaveType.Flash8m,
    "nand-64m": SaveType.Nand64m,
    "nand-128m": SaveType.Nand128m,
    "nand-256m": SaveType.Nand256m,
};

export const enum InputBits {
    A = 1 << 0,
    B = 1 << 1,
    Select = 1 << 2,
    Start = 1 << 3,
    Right = 1 << 4,
    Left = 1 << 5,
    Up = 1 << 6,
    Down = 1 << 7,
    R = 1 << 8,
    L = 1 << 9,
    X = 1 << 16,
    Y = 1 << 17,
    Debug = 1 << 19,
}

export namespace UiToEmu {
    export const enum MessageType {
        Start,
        Reset,
        Stop,
        LoadSave,
        ExportSave,
        UpdateInput,
        UpdatePlaying,
        UpdateFramerateLimit,
    }

    export interface StartMessage {
        type: MessageType.Start;
        rom: Uint8Array;
        bios7: Uint8Array | undefined;
        bios9: Uint8Array | undefined;
        firmware: Uint8Array | undefined;
        saveType: SaveType | undefined;
        hasIR: boolean;
    }

    export interface RawMessage {
        type: MessageType.Reset | MessageType.ExportSave | MessageType.Stop;
    }

    export interface LoadSaveMessage {
        type: MessageType.LoadSave;
        buffer: ArrayBuffer;
    }

    export interface UpdateInputMessage {
        type: MessageType.UpdateInput;
        pressed: number;
        released: number;
        touchPos: [number, number] | null | undefined;
    }

    export interface UpdateFlagMessage {
        type: MessageType.UpdatePlaying | MessageType.UpdateFramerateLimit;
        value: boolean;
    }

    export type Message =
        | StartMessage
        | RawMessage
        | LoadSaveMessage
        | UpdateInputMessage
        | UpdateFlagMessage;
}

export namespace EmuToUi {
    export const enum MessageType {
        Loaded,
        StartRenderer,
        ExportSave,
        RenderFrame,
        Stopped,
        PlayAudioChunk,
    }

    export interface LoadedMessage {
        type: MessageType.Loaded;
    }

    export interface StartRendererMessage {
        type: MessageType.StartRenderer;
        module: WebAssembly.Module;
        memory: WebAssembly.Memory;
    }

    export interface StopMessage {
        type: MessageType.Stopped;
        buffer: Uint8Array;
    }

    export interface ExportSaveMessage {
        type: MessageType.ExportSave;
        buffer: Uint8Array;
        triggerDownload: boolean;
    }

    export interface RenderFrameMessage {
        type: MessageType.RenderFrame;
        buffer: Uint32Array;
    }

    export interface PlayAudioChunkMessage {
        type: MessageType.PlayAudioChunk;
        l: Float32Array;
        r: Float32Array;
    }

    export type Message =
        | LoadedMessage
        | StartRendererMessage
        | StopMessage
        | ExportSaveMessage
        | RenderFrameMessage
        | PlayAudioChunkMessage;
}
