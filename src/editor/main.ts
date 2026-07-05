import { invoke } from "@tauri-apps/api/core";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import {
  cloneShape,
  cloneState,
  hitTest,
  isLightColor,
  moveShape,
  renderScene,
  scaleShape,
  selectionBounds,
  type Point,
  type Rect,
  type SceneState,
  type Shape,
} from "./scene";

// No "select" mode: like Instagram/Snapchat, drawing tools always draw, and
// anything already on the image is grabbed directly — tap to select, drag
// inside its frame to move, drag the corner grip to resize, ✕ to delete.
type Tool =
  | "crop"
  | "pen"
  | "neon"
  | "highlighter"
  | "arrow"
  | "rect"
  | "ellipse"
  | "text"
  | "blur"
  | "badge"
  | "eraser";

const PRESET_COLORS = [
  "#ff3b30",
  "#ff9500",
  "#ffcc00",
  "#34c759",
  "#0066cc",
  "#af52de",
  "#ffffff",
  "#1d1d1f",
];

const sceneCanvas = document.getElementById("scene") as HTMLCanvasElement;
const sceneCtx = sceneCanvas.getContext("2d")!;
const textWrap = document.getElementById("text-wrap") as HTMLDivElement;
const textInput = document.getElementById("text-input") as HTMLInputElement;
const textBgToggle = document.getElementById("text-bg-toggle") as HTMLButtonElement;
const stage = document.getElementById("stage") as HTMLElement;
const selDelete = document.getElementById("sel-delete") as HTMLButtonElement;
const brushPreview = document.getElementById("brush-preview") as HTMLDivElement;

const base = document.createElement("canvas");

let state: SceneState = { shapes: [], crop: { x: 0, y: 0, w: 0, h: 0 } };
let undoStack: SceneState[] = [];
let redoStack: SceneState[] = [];

let tool: Tool = "pen";
let color = PRESET_COLORS[0];
let strokeWidth = 6;
let selected: Shape | null = null;
let textBg = false;

// In-progress interaction state
let drafting: Shape | null = null;
let cropDraft: Rect | null = null;
let pendingText: Point | null = null;
let pressStart: Point | null = null;
let moving = false;
let dragLast: Point | null = null;
let resizing: { original: Shape; anchor: Point; startDist: number } | null = null;
let erasedAny = false;
let eraseSnapshot: SceneState | null = null;

function pushUndo() {
  undoStack.push(cloneState(state));
  if (undoStack.length > 100) undoStack.shift();
  redoStack = [];
}

function undo() {
  const prev = undoStack.pop();
  if (!prev) return;
  redoStack.push(cloneState(state));
  state = prev;
  selected = null;
  render();
}

function redo() {
  const next = redoStack.pop();
  if (!next) return;
  undoStack.push(cloneState(state));
  state = next;
  selected = null;
  render();
}

/// CSS pixels per canvas pixel — canvas is scaled to fit the stage.
function displayScale(): number {
  const rect = sceneCanvas.getBoundingClientRect();
  return rect.width / sceneCanvas.width || 1;
}

/// Selection chrome (dashes, grip) drawn in canvas units but sized so it
/// reads the same on screen regardless of zoom.
function handleCanvasPx(): number {
  return 9 / displayScale();
}

function render() {
  renderScene(sceneCtx, base, state, selected, handleCanvasPx());
  if (cropDraft) drawCropDraft();
  updateSelectionUi();
}

function updateSelectionUi() {
  if (!selected) {
    selDelete.classList.add("hidden");
    return;
  }
  const b = selectionBounds(selected);
  const rect = sceneCanvas.getBoundingClientRect();
  const stageRect = stage.getBoundingClientRect();
  const s = displayScale();
  selDelete.style.left = `${rect.left - stageRect.left + (b.x + b.w - state.crop.x) * s - 8}px`;
  selDelete.style.top = `${rect.top - stageRect.top + (b.y - state.crop.y) * s - 18}px`;
  selDelete.classList.remove("hidden");
}

selDelete.addEventListener("click", () => {
  if (!selected) return;
  pushUndo();
  state.shapes = state.shapes.filter((s) => s !== selected);
  selected = null;
  render();
});

function drawCropDraft() {
  const r = cropDraft!;
  const ctx = sceneCtx;
  ctx.save();
  ctx.translate(-state.crop.x, -state.crop.y);
  const x = Math.min(r.x, r.x + r.w);
  const y = Math.min(r.y, r.y + r.h);
  const w = Math.abs(r.w);
  const h = Math.abs(r.h);

  // Dim outside the draft
  ctx.fillStyle = "rgba(0, 0, 0, 0.5)";
  const c = state.crop;
  ctx.fillRect(c.x, c.y, c.w, c.h);
  ctx.clearRect(x, y, w, h);
  ctx.drawImage(base, x, y, w, h, x, y, w, h);

  // Frame + rule-of-thirds grid
  ctx.strokeStyle = "#ffffff";
  ctx.lineWidth = 2;
  ctx.strokeRect(x, y, w, h);
  ctx.strokeStyle = "rgba(255, 255, 255, 0.35)";
  ctx.lineWidth = 1;
  for (let i = 1; i <= 2; i++) {
    ctx.beginPath();
    ctx.moveTo(x + (w * i) / 3, y);
    ctx.lineTo(x + (w * i) / 3, y + h);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(x, y + (h * i) / 3);
    ctx.lineTo(x + w, y + (h * i) / 3);
    ctx.stroke();
  }
  ctx.restore();
}

/// Canvas-space (image pixel) coordinates from a pointer event, accounting
/// for the CSS scale-to-fit and the current crop offset.
function canvasPoint(e: PointerEvent): Point {
  const rect = sceneCanvas.getBoundingClientRect();
  const scaleX = sceneCanvas.width / rect.width;
  const scaleY = sceneCanvas.height / rect.height;
  return {
    x: (e.clientX - rect.left) * scaleX + state.crop.x,
    y: (e.clientY - rect.top) * scaleY + state.crop.y,
  };
}

function badgeCount(): number {
  return state.shapes.filter((s) => s.kind === "badge").length;
}

function pointInRect(p: Point, r: Rect, pad = 0): boolean {
  return p.x >= r.x - pad && p.x <= r.x + r.w + pad && p.y >= r.y - pad && p.y <= r.y + r.h + pad;
}

function eraseAt(p: Point) {
  const hit = hitTest(state.shapes, p);
  if (!hit) return;
  if (!erasedAny) {
    undoStack.push(eraseSnapshot!);
    if (undoStack.length > 100) undoStack.shift();
    redoStack = [];
    erasedAny = true;
  }
  state.shapes = state.shapes.filter((s) => s !== hit);
  if (selected === hit) selected = null;
  render();
}

// ── Pointer interactions ─────────────────────────────────

sceneCanvas.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  sceneCanvas.setPointerCapture(e.pointerId);
  const p = canvasPoint(e);
  commitTextIfOpen();
  pressStart = p;

  // Direct manipulation of the current selection beats every tool: grip
  // corner resizes, anywhere inside the frame moves.
  if (selected) {
    const b = selectionBounds(selected);
    const grip = handleCanvasPx() * 1.6;
    if (Math.hypot(p.x - (b.x + b.w), p.y - (b.y + b.h)) <= grip) {
      pushUndo();
      resizing = {
        original: cloneShape(selected),
        anchor: { x: b.x, y: b.y },
        startDist: Math.max(1, Math.hypot(p.x - b.x, p.y - b.y)),
      };
      return;
    }
    if (pointInRect(p, b)) {
      pushUndo();
      moving = true;
      dragLast = p;
      return;
    }
    selected = null;
    render();
  }

  switch (tool) {
    case "crop":
      cropDraft = { x: p.x, y: p.y, w: 0, h: 0 };
      break;
    case "eraser":
      eraseSnapshot = cloneState(state);
      erasedAny = false;
      eraseAt(p);
      break;
    case "pen":
    case "neon":
    case "highlighter":
      pushUndo();
      drafting = { kind: tool, points: [p], color, width: strokeWidth };
      state.shapes.push(drafting);
      break;
    case "arrow":
      pushUndo();
      drafting = { kind: "arrow", x1: p.x, y1: p.y, x2: p.x, y2: p.y, color, width: strokeWidth };
      state.shapes.push(drafting);
      break;
    case "rect":
    case "ellipse":
      pushUndo();
      drafting = { kind: tool, x: p.x, y: p.y, w: 0, h: 0, color, width: strokeWidth };
      state.shapes.push(drafting);
      break;
    case "blur":
      pushUndo();
      drafting = { kind: "blur", x: p.x, y: p.y, w: 0, h: 0 };
      state.shapes.push(drafting);
      break;
    case "badge":
      pushUndo();
      state.shapes.push({ kind: "badge", x: p.x, y: p.y, n: badgeCount() + 1, color });
      render();
      break;
    case "text":
      // Without this, the canvas takes focus after pointerdown, which
      // blurs the input the instant it opens and commits it empty.
      e.preventDefault();
      openTextInput(e, p);
      break;
  }
});

sceneCanvas.addEventListener("pointermove", (e) => {
  const p = canvasPoint(e);

  if (resizing && selected) {
    const dist = Math.hypot(p.x - resizing.anchor.x, p.y - resizing.anchor.y);
    const f = Math.min(20, Math.max(0.1, dist / resizing.startDist));
    const scaled = cloneShape(resizing.original);
    scaleShape(scaled, f, resizing.anchor);
    Object.assign(selected, scaled);
    render();
    return;
  }
  if (moving && selected && dragLast) {
    moveShape(selected, p.x - dragLast.x, p.y - dragLast.y);
    dragLast = p;
    render();
    return;
  }
  if (tool === "eraser" && eraseSnapshot) {
    eraseAt(p);
    return;
  }
  if (cropDraft) {
    cropDraft.w = p.x - cropDraft.x;
    cropDraft.h = p.y - cropDraft.y;
    render();
    return;
  }
  if (!drafting) return;

  switch (drafting.kind) {
    case "pen":
    case "neon":
    case "highlighter":
      drafting.points.push(p);
      break;
    case "arrow":
      drafting.x2 = p.x;
      drafting.y2 = p.y;
      break;
    case "rect":
    case "ellipse":
    case "blur":
      drafting.w = p.x - drafting.x;
      drafting.h = p.y - drafting.y;
      break;
  }
  render();
});

sceneCanvas.addEventListener("pointerup", (e) => {
  const p = canvasPoint(e);
  const tapped =
    pressStart !== null && Math.hypot(p.x - pressStart.x, p.y - pressStart.y) < 3;
  pressStart = null;

  if (resizing) {
    resizing = null;
    return;
  }
  if (moving) {
    moving = false;
    dragLast = null;
    // A tap inside the frame without dragging: pass the selection through
    // to whatever is under the pointer (or deselect on empty space).
    if (tapped) {
      undoStack.pop();
      selected = hitTest(state.shapes, p);
      render();
    }
    return;
  }
  if (tool === "eraser") {
    eraseSnapshot = null;
    return;
  }

  if (cropDraft) {
    const x = Math.min(cropDraft.x, cropDraft.x + cropDraft.w);
    const y = Math.min(cropDraft.y, cropDraft.y + cropDraft.h);
    const w = Math.abs(cropDraft.w);
    const h = Math.abs(cropDraft.h);
    cropDraft = null;
    if (w > 8 && h > 8) {
      pushUndo();
      state.crop = { x, y, w, h };
    }
    render();
    return;
  }

  if (drafting) {
    // Discard degenerate shapes (a click without a drag) — and treat the
    // click as a grab of whatever is under the pointer instead.
    const degenerate =
      (drafting.kind === "rect" || drafting.kind === "ellipse" || drafting.kind === "blur"
        ? Math.abs(drafting.w) < 3 || Math.abs(drafting.h) < 3
        : false) ||
      (drafting.kind === "arrow"
        ? Math.hypot(drafting.x2 - drafting.x1, drafting.y2 - drafting.y1) < 3
        : false) ||
      ((drafting.kind === "pen" || drafting.kind === "neon" || drafting.kind === "highlighter") &&
        drafting.points.length < 2);
    if (degenerate) {
      state.shapes.pop();
      undoStack.pop();
      selected = hitTest(state.shapes, p);
    }
    drafting = null;
    render();
  }
});

// ── Text tool ────────────────────────────────────────────

function applyTextInputStyle() {
  textWrap.classList.toggle("bg-on", textBg);
  if (textBg) {
    textInput.style.background = color;
    textInput.style.color = isLightColor(color) ? "#1d1d1f" : "#ffffff";
    textInput.style.borderColor = "transparent";
  } else {
    textInput.style.background = "transparent";
    textInput.style.color = color;
    textInput.style.borderColor = "rgba(255, 255, 255, 0.6)";
  }
}

function openTextInput(e: PointerEvent, p: Point) {
  pendingText = p;
  const stageRect = stage.getBoundingClientRect();
  const fontSize = textFontSize();
  const cssScale = displayScale();
  textWrap.style.left = `${e.clientX - stageRect.left}px`;
  textWrap.style.top = `${e.clientY - stageRect.top}px`;
  textInput.style.fontSize = `${fontSize * cssScale}px`;
  textInput.value = "";
  applyTextInputStyle();
  textWrap.classList.remove("hidden");
  // Focus after the pointer event settles; focusing mid-pointerdown loses
  // to the default focus change that follows the event.
  requestAnimationFrame(() => textInput.focus());
}

function textFontSize(): number {
  return 12 + strokeWidth * 3;
}

function commitTextIfOpen() {
  if (textWrap.classList.contains("hidden")) return;
  const value = textInput.value.trim();
  if (value && pendingText) {
    pushUndo();
    const shape: Shape = {
      kind: "text",
      x: pendingText.x,
      y: pendingText.y,
      text: value,
      color,
      size: textFontSize(),
      bg: textBg,
    };
    state.shapes.push(shape);
    // Freshly placed text is selected, ready to drag into position.
    selected = shape;
  }
  textWrap.classList.add("hidden");
  pendingText = null;
  render();
}

textInput.addEventListener("keydown", (e) => {
  e.stopPropagation();
  if (e.key === "Enter") commitTextIfOpen();
  if (e.key === "Escape") {
    textInput.value = "";
    commitTextIfOpen();
  }
});
textInput.addEventListener("blur", (e) => {
  // Clicking the bg toggle must not commit the entry.
  if (e.relatedTarget === textBgToggle) return;
  commitTextIfOpen();
});

textBgToggle.addEventListener("click", () => {
  textBg = !textBg;
  applyTextInputStyle();
  textInput.focus();
});

// ── Tool rail ────────────────────────────────────────────

const toolButtons = Array.from(document.querySelectorAll<HTMLButtonElement>("#tool-rail .tool"));

function setTool(next: Tool) {
  tool = next;
  selected = null;
  for (const btn of toolButtons) {
    btn.classList.toggle("active", btn.dataset.tool === next);
  }
  sceneCanvas.className = `tool-${next}`;
  render();
}

for (const btn of toolButtons) {
  btn.addEventListener("click", () => setTool(btn.dataset.tool as Tool));
}

// ── Color bar ────────────────────────────────────────────

function updateBrushPreview() {
  const d = Math.max(6, Math.min(26, strokeWidth));
  brushPreview.style.width = `${d}px`;
  brushPreview.style.height = `${d}px`;
  brushPreview.style.background = color;
}

function applyColor(next: string) {
  color = next;
  updateBrushPreview();
  // Tap a color while something is selected → recolor it in place.
  if (selected && "color" in selected) {
    pushUndo();
    selected.color = next;
    render();
  }
}

const colorsEl = document.getElementById("color-bar")!;
for (const preset of PRESET_COLORS) {
  const swatch = document.createElement("button");
  swatch.className = "swatch";
  swatch.style.background = preset;
  swatch.title = preset;
  if (preset === color) swatch.classList.add("active");
  swatch.addEventListener("click", () => {
    applyColor(preset);
    document.querySelectorAll(".swatch.active").forEach((el) => el.classList.remove("active"));
    swatch.classList.add("active");
  });
  colorsEl.appendChild(swatch);
}
const customColor = document.createElement("input");
customColor.type = "color";
customColor.id = "custom-color";
customColor.title = "Custom color";
customColor.addEventListener("input", () => {
  applyColor(customColor.value);
  document.querySelectorAll(".swatch.active").forEach((el) => el.classList.remove("active"));
});
colorsEl.appendChild(customColor);

const strokeSlider = document.getElementById("stroke") as HTMLInputElement;
strokeSlider.addEventListener("input", () => {
  strokeWidth = Number(strokeSlider.value);
  updateBrushPreview();
});

document.getElementById("btn-undo")!.addEventListener("click", undo);
document.getElementById("btn-redo")!.addEventListener("click", redo);

// ── Export / save / copy ─────────────────────────────────

async function exportPngBytes(): Promise<number[]> {
  const out = document.createElement("canvas");
  const ctx = out.getContext("2d")!;
  renderScene(ctx, base, state, null);
  const blob: Blob = await new Promise((resolve, reject) =>
    out.toBlob((b) => (b ? resolve(b) : reject(new Error("toBlob failed"))), "image/png"),
  );
  return Array.from(new Uint8Array(await blob.arrayBuffer()));
}

async function save() {
  commitTextIfOpen();
  const pngBytes = await exportPngBytes();
  await invoke("save_edited", { pngBytes, copyToClipboard: true });
}

async function copyOnly() {
  commitTextIfOpen();
  const pngBytes = await exportPngBytes();
  await invoke("copy_edited", { pngBytes });
}

function cancel() {
  invoke("cancel_edit");
}

document.getElementById("btn-save")!.addEventListener("click", () => void save());
document.getElementById("btn-copy")!.addEventListener("click", () => void copyOnly());

// ── Drag out ─────────────────────────────────────────────
// Press the Drag chip and pull: the annotated PNG becomes a real file and
// a native OS drag starts, so it can be dropped into Slack, a browser
// upload zone, Explorer — anywhere Windows accepts files.

const dragBtn = document.getElementById("btn-drag") as HTMLButtonElement;
let dragBusy = false;

dragBtn.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || dragBusy) return;
  dragBusy = true;
  dragBtn.classList.add("busy");
  void (async () => {
    try {
      commitTextIfOpen();
      const pngBytes = await exportPngBytes();
      const path = await invoke<string>("export_drag_file", { pngBytes });
      await startDrag({ item: [path], icon: path });
    } catch (err) {
      reportError("drag out", err);
    } finally {
      dragBusy = false;
      dragBtn.classList.remove("busy");
    }
  })();
});

// ── Keyboard ─────────────────────────────────────────────

const TOOL_KEYS: Record<string, Tool> = {
  p: "pen",
  n: "neon",
  h: "highlighter",
  a: "arrow",
  r: "rect",
  o: "ellipse",
  t: "text",
  b: "blur",
  d: "badge",
  e: "eraser",
  c: "crop",
};

window.addEventListener("keydown", (e) => {
  if (e.target === textInput) return;

  if (e.ctrlKey && e.key.toLowerCase() === "z") {
    e.preventDefault();
    undo();
  } else if (e.ctrlKey && e.key.toLowerCase() === "y") {
    e.preventDefault();
    redo();
  } else if (e.ctrlKey && e.key.toLowerCase() === "c") {
    e.preventDefault();
    void copyOnly();
  } else if (e.ctrlKey && e.key.toLowerCase() === "s") {
    e.preventDefault();
    void save();
  } else if (e.key === "Enter") {
    void save();
  } else if (e.key === "Escape") {
    if (selected) {
      selected = null;
      render();
    } else {
      cancel();
    }
  } else if (e.key === "Delete" || e.key === "Backspace") {
    if (selected) {
      pushUndo();
      state.shapes = state.shapes.filter((s) => s !== selected);
      selected = null;
      render();
    }
  } else if (!e.ctrlKey && TOOL_KEYS[e.key.toLowerCase()]) {
    setTool(TOOL_KEYS[e.key.toLowerCase()]);
  }
});

// Keep the floating delete button glued to the selection when the window
// resizes (the canvas re-scales to fit).
window.addEventListener("resize", () => updateSelectionUi());

// ── Bootstrap ────────────────────────────────────────────

function reportError(context: string, err: unknown) {
  const message = err instanceof Error ? `${err.message}\n${err.stack ?? ""}` : String(err);
  void invoke("frontend_log", { message: `${context}: ${message}` });
}

window.addEventListener("error", (e) => reportError("window.onerror", e.error ?? e.message));
window.addEventListener("unhandledrejection", (e) => reportError("unhandledrejection", e.reason));

async function init() {
  const [response, [width, height]] = await Promise.all([
    fetch("http://capture.localhost/edit"),
    invoke<[number, number]>("get_edit_meta"),
  ]);
  const bytes = new Uint8ClampedArray(await response.arrayBuffer());

  base.width = width;
  base.height = height;
  base.getContext("2d")!.putImageData(new ImageData(bytes, width, height), 0, 0);

  state = { shapes: [], crop: { x: 0, y: 0, w: width, h: height } };
  render();
  setTool("pen");
  updateBrushPreview();
  await invoke("editor_ready");
}

init().catch((e) => reportError("init", e));
