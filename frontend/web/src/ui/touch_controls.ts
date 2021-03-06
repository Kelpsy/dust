import { InputBits } from "../message";

export interface ControlLayoutData {
    x?: number;
    y?: number;
    scale?: number;
    interactionScale?: number;
}

export const enum TouchArea {
    Controls,
    BottomScreen,
    None,
}

export interface Touch {
    area: TouchArea;
    startX: number;
    startY: number;
    x: number;
    y: number;
}

export class Control {
    public readonly interactionElement: HTMLElement;

    private halfWidth_: number;
    private halfHeight_: number;
    private x_!: number;
    private y_!: number;
    private scale_!: number;
    private interactionScale_!: number;
    private editing_: boolean = false;

    get halfWidth(): number {
        return this.halfWidth_;
    }

    get halfHeight(): number {
        return this.halfHeight_;
    }

    get x(): number {
        return this.x_;
    }

    set x(x: number) {
        this.x_ = x;
        this.element.style.left = `${x - this.halfWidth_}px`;
    }

    get y(): number {
        return this.y_;
    }

    set y(y: number) {
        this.y_ = y;
        this.element.style.top = `${y - this.halfHeight_}px`;
    }

    protected updateScale() {
        this.element.style.transform = `scale(${this.scale})`;
        this.updateInteractionScale();
    }

    get scale(): number {
        return this.scale_;
    }

    set scale(scale: number) {
        this.scale_ = scale;
        this.halfWidth_ = this.element.clientWidth * 0.5;
        this.halfHeight_ = this.element.clientHeight * 0.5;
        this.updateScale();
    }

    protected updateInteractionScale() {
        const finalScale = this.scale * this.interactionScale;
        this.interactionElement.style.transform = `translate(-50%, -50%) scale(${this.interactionScale})`;
        this.interactionElement.style.borderWidth = `${5 / finalScale}px`;
    }

    get interactionScale(): number {
        return this.interactionScale_;
    }

    set interactionScale(interactionScale: number) {
        this.interactionScale_ = interactionScale;
        this.updateInteractionScale();
    }

    get editing(): boolean {
        return this.editing_;
    }

    set editing(editing: boolean) {
        this.editing_ = editing;
        this.interactionElement.style.opacity = editing ? "1" : "0";
    }

    get layoutData(): ControlLayoutData {
        return {
            x: this.x_,
            y: this.y_,
            scale: this.scale_,
        };
    }

    set layoutData(layoutData: ControlLayoutData) {
        if (typeof layoutData.x !== "undefined") {
            this.x = layoutData.x;
        }
        if (typeof layoutData.y !== "undefined") {
            this.y = layoutData.y;
        }
        if (typeof layoutData.scale !== "undefined") {
            this.scale = layoutData.scale;
        }
        if (typeof layoutData.interactionScale !== "undefined") {
            this.interactionScale = layoutData.interactionScale;
        }
    }

    constructor(
        public readonly element: HTMLElement,
        layoutData: ControlLayoutData = {}
    ) {
        this.interactionElement = element.getElementsByClassName(
            "interaction"
        )[0] as HTMLElement;

        this.halfWidth_ = this.element.clientWidth * 0.5;
        this.halfHeight_ = this.element.clientHeight * 0.5;

        layoutData.x ??= this.element.offsetLeft + this.halfWidth_;
        layoutData.y ??= this.element.offsetTop + this.halfHeight_;
        layoutData.scale ??= 1.0;
        layoutData.interactionScale ??= 1.0;

        this.layoutData = layoutData;
    }
}

export class Button extends Control {
    public readonly button!: HTMLButtonElement;

    constructor(
        element: HTMLElement,
        public readonly stateBit: number,
        layoutData: ControlLayoutData = {}
    ) {
        super(element);
        this.button = element.getElementsByTagName(
            "button"
        )[0] as HTMLButtonElement;
        this.layoutData = layoutData;
    }
}

export class Dpad extends Control {
    private up: HTMLElement;
    private down: HTMLElement;
    private left: HTMLElement;
    private right: HTMLElement;

    resetTouches() {
        this.up.classList.remove("pressed");
        this.down.classList.remove("pressed");
        this.left.classList.remove("pressed");
        this.right.classList.remove("pressed");
    }

    processTouch(touch: Touch, state: number): number {
        const angle = Math.atan2(this.y - touch.y, touch.x - this.x);
        const bits = [
            InputBits.Right,
            InputBits.Right | InputBits.Up,
            InputBits.Up,
            InputBits.Up | InputBits.Left,
            InputBits.Left,
            InputBits.Left | InputBits.Down,
            InputBits.Down,
            InputBits.Down | InputBits.Right,
        ][Math.round((angle * 4) / Math.PI) & 7]!;
        if (bits & InputBits.Up) this.up.classList.add("pressed");
        if (bits & InputBits.Down) this.down.classList.add("pressed");
        if (bits & InputBits.Left) this.left.classList.add("pressed");
        if (bits & InputBits.Right) this.right.classList.add("pressed");
        return state | bits;
    }

    constructor(element: HTMLElement, layoutData: ControlLayoutData = {}) {
        layoutData.interactionScale ??= 1.2;
        super(element, layoutData);
        this.up = document.getElementById("dpad-up")!;
        this.down = document.getElementById("dpad-down")!;
        this.left = document.getElementById("dpad-left")!;
        this.right = document.getElementById("dpad-right")!;
    }
}

export interface ControlsLayoutData {
    dpad?: ControlLayoutData;
    a?: ControlLayoutData;
    b?: ControlLayoutData;
    x?: ControlLayoutData;
    y?: ControlLayoutData;
    l?: ControlLayoutData;
    r?: ControlLayoutData;
    start?: ControlLayoutData;
    select?: ControlLayoutData;
    pause?: ControlLayoutData;
}

export type ButtonKey = "a" | "b" | "x" | "y" | "l" | "r" | "start" | "select";

export class TouchControls {
    public buttons: Map<ButtonKey, Button>;
    public dpad: Dpad;
    public pause: Button;

    private editing_: boolean = false;

    constructor(
        public element: HTMLElement,
        layoutData: ControlsLayoutData = {}
    ) {
        function button(key: ButtonKey, bit: number): [ButtonKey, Button] {
            return [
                key,
                new Button(
                    document.getElementById(`btn-${key}`)!,
                    bit,
                    layoutData[key]
                ),
            ];
        }

        this.buttons = new Map([
            button("a", InputBits.A),
            button("b", InputBits.B),
            button("x", InputBits.X),
            button("y", InputBits.Y),
            button("l", InputBits.L),
            button("r", InputBits.R),
            button("start", InputBits.Start),
            button("select", InputBits.Select),
        ]);

        this.dpad = new Dpad(document.getElementById("dpad")!, layoutData.dpad);

        this.pause = new Button(
            document.getElementById(`btn-pause`)!,
            0,
            layoutData.pause
        );
    }

    resetTouches() {
        for (const button of this.buttons.values()) {
            button.element.classList.remove("pressed");
        }
        this.dpad.resetTouches();
    }

    containTouch(x: number, y: number): boolean {
        const elements = document.elementsFromPoint(x, y);

        if (elements.indexOf(this.pause.interactionElement) !== -1) {
            return true;
        }

        for (const button of this.buttons.values()) {
            if (elements.indexOf(button.interactionElement) !== -1) {
                return true;
            }
        }

        if (elements.indexOf(this.dpad.interactionElement) !== -1) {
            return true;
        }

        return false;
    }

    processTouch(touch: Touch, state: number): number {
        const elements = document.elementsFromPoint(touch.x, touch.y);

        for (const button of this.buttons.values()) {
            if (elements.indexOf(button.interactionElement) !== -1) {
                state |= button.stateBit;
                button.element.classList.add("pressed");
            }
        }

        if (elements.indexOf(this.dpad.interactionElement) !== -1) {
            state = this.dpad.processTouch(touch, state);
        }

        return state;
    }

    get layoutData(): ControlsLayoutData {
        const result: ControlsLayoutData = {
            dpad: this.dpad.layoutData,
            pause: this.pause.layoutData,
        };

        for (const [key, button] of this.buttons) {
            result[key] = button.layoutData;
        }

        return result;
    }

    set layoutData(layoutData: ControlsLayoutData) {
        for (const [key, button] of this.buttons) {
            const buttonLayoutData = layoutData[key];
            if (buttonLayoutData) {
                button.layoutData = buttonLayoutData;
            }
        }

        if (layoutData.dpad) this.dpad.layoutData = layoutData.dpad;
        if (layoutData.pause) this.pause.layoutData = layoutData.pause;
    }

    get defaultLayout(): ControlsLayoutData {
        const margin = parseFloat(getComputedStyle(document.body).fontSize);
        const width = document.body.clientWidth;
        const height = document.body.clientHeight;

        const aspectRatio = width / height;
        const baseY =
            (aspectRatio < 5 / 4 ? 0.4 : aspectRatio < 4 / 3 ? 0.3 : 0.1) *
            height;

        const a = this.buttons.get("a")!;
        const b = this.buttons.get("b")!;
        const x = this.buttons.get("x")!;
        const y = this.buttons.get("y")!;

        const faceButtonsAvgHalfWidth =
            (a.halfWidth + b.halfWidth + x.halfWidth + y.halfWidth) / 4;
        const faceButtonsAvgHalfHeight =
            (a.halfHeight + b.halfHeight + x.halfHeight + y.halfHeight) / 4;

        const l = this.buttons.get("l")!;
        const r = this.buttons.get("r")!;
        const start = this.buttons.get("start")!;
        const select = this.buttons.get("select")!;

        const pauseInteractionRadius =
            this.pause.halfWidth * this.pause.interactionScale;

        const centerX = 0.5 * width;

        const dpadRight = 2 * this.dpad.halfWidth + 2 * margin;
        const faceButtonsLeft =
            width - (6 * faceButtonsAvgHalfWidth + 2 * margin);

        const startX =
            centerX + pauseInteractionRadius + start.halfWidth + margin;
        const selectX =
            centerX - (pauseInteractionRadius + select.halfWidth + margin);

        const startRight = startX - start.halfWidth;
        const selectLeft = selectX - select.halfWidth;

        const dpadFaceButtonsBase =
            dpadRight >= selectLeft || faceButtonsLeft <= startRight
                ? height -
                  (2 * Math.max(start.halfHeight, select.halfHeight) +
                      2 * margin)
                : height - margin;

        return {
            dpad: {
                x: this.dpad.halfWidth + margin,
                y: dpadFaceButtonsBase - this.dpad.halfHeight,
            },
            a: {
                x: width - (faceButtonsAvgHalfWidth + margin),
                y: dpadFaceButtonsBase - 3 * faceButtonsAvgHalfHeight,
                interactionScale: 1.75,
            },
            b: {
                x: width - (3 * faceButtonsAvgHalfWidth + margin),
                y: dpadFaceButtonsBase - faceButtonsAvgHalfHeight,
                interactionScale: 1.75,
            },
            x: {
                x: width - (3 * faceButtonsAvgHalfWidth + margin),
                y: dpadFaceButtonsBase - 5 * faceButtonsAvgHalfHeight,
                interactionScale: 1.75,
            },
            y: {
                x: width - (5 * faceButtonsAvgHalfWidth + margin),
                y: dpadFaceButtonsBase - 3 * faceButtonsAvgHalfHeight,
                interactionScale: 1.75,
            },
            l: {
                x: margin + l.halfWidth,
                y: baseY + margin + l.halfHeight,
            },
            r: {
                x: width - (margin + r.halfWidth),
                y: baseY + margin + r.halfHeight,
            },
            start: {
                x: startX,
                y: height - (start.halfHeight + margin),
            },
            select: {
                x: selectX,
                y: height - (select.halfHeight + margin),
            },
            pause: {
                x: centerX,
                y:
                    height -
                    ((this.element.classList.contains("touch")
                        ? Math.max(
                              this.pause.halfHeight,
                              (start.halfHeight + select.halfHeight) / 2
                          )
                        : this.pause.halfHeight) +
                        margin),
            },
        };
    }

    get editing(): boolean {
        return this.editing_;
    }

    set editing(editing: boolean) {
        if (editing === this.editing_) {
            return;
        }
        this.editing_ = editing;
        for (const button of this.buttons.values()) {
            button.editing = editing;
        }
    }
}
