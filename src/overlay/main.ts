import { invoke } from "@tauri-apps/api/core";

interface MonitorFrame {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface WindowRect {
  0: number; // x
  1: number; // y
  2: number; // width
  3: number; // height
}

const monitorIndex = Number(new URLSearchParams(location.search).get("monitor") ?? "0");

const frozenCanvas = document.getElementById("frozen") as HTMLCanvasElement;
const selectionEl = document.getElementById("selection") as HTMLDivElement;
const dimensionsEl = document.getElementById("dimensions") as HTMLDivElement;

let frame: MonitorFrame | null = null;
let dragStart: { x: number; y: number } | null = null;
let windowSnapRect: WindowRect | null = null;
let snapLookupInFlight = false;

function toPhysical(clientX: number, clientY: number) {
  const scale = window.devicePixelRatio || 1;
  return { x: Math.round(clientX * scale), y: Math.round(clientY * scale) };
}

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

function showSelection(x: number, y: number, width: number, height: number, isWindowSnap: boolean) {
  selectionEl.classList.remove("hidden");
  selectionEl.classList.toggle("window-snap", isWindowSnap);
  const scale = window.devicePixelRatio || 1;
  selectionEl.style.left = `${x / scale}px`;
  selectionEl.style.top = `${y / scale}px`;
  selectionEl.style.width = `${width / scale}px`;
  selectionEl.style.height = `${height / scale}px`;

  dimensionsEl.classList.remove("hidden");
  dimensionsEl.textContent = `${width} × ${height}`;
  dimensionsEl.style.left = `${x / scale}px`;
  dimensionsEl.style.top = `${y / scale}px`;
}

function hideSelection() {
  selectionEl.classList.add("hidden");
  dimensionsEl.classList.add("hidden");
}

async function loadFrame() {
  // Raw RGBA over the custom `capture:` protocol, blitted straight onto a
  // canvas — no PNG encode/decode anywhere. Geometry comes over IPC.
  const [response, meta] = await Promise.all([
    fetch(`http://capture.localhost/${monitorIndex}`),
    invoke<MonitorFrame>("get_pending_capture_meta", { monitor: monitorIndex }),
  ]);
  const bytes = new Uint8ClampedArray(await response.arrayBuffer());
  frame = meta;

  frozenCanvas.width = meta.width;
  frozenCanvas.height = meta.height;
  const ctx = frozenCanvas.getContext("2d")!;
  ctx.putImageData(new ImageData(bytes, meta.width, meta.height), 0, 0);

  // Frame is painted — now it's safe to show this window (avoids the
  // black flash of an empty webview).
  await invoke("overlay_ready");
}

function reportError(context: string, err: unknown) {
  const message = err instanceof Error ? `${err.message}\n${err.stack ?? ""}` : String(err);
  void invoke("frontend_log", { message: `overlay ${context}: ${message}` });
}

window.addEventListener("error", (e) => reportError("window.onerror", e.error ?? e.message));
window.addEventListener("unhandledrejection", (e) => reportError("unhandledrejection", e.reason));

function finish(localX: number, localY: number, width: number, height: number) {
  invoke("finish_region_capture", {
    monitor: monitorIndex,
    region: { x: localX, y: localY, width, height },
  }).catch((e) => reportError("finish_region_capture", e));
}

function cancel() {
  invoke("cancel_region_capture").catch((e) => reportError("cancel_region_capture", e));
}

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") cancel();
});

window.addEventListener("mousedown", (e) => {
  if (!frame) return;
  const p = toPhysical(e.clientX, e.clientY);
  dragStart = p;
  windowSnapRect = null;
  showSelection(p.x, p.y, 0, 0, false);
});

window.addEventListener("mousemove", async (e) => {
  if (!frame) return;
  const p = toPhysical(e.clientX, e.clientY);

  if (dragStart) {
    const x = Math.min(dragStart.x, p.x);
    const y = Math.min(dragStart.y, p.y);
    const width = Math.abs(p.x - dragStart.x);
    const height = Math.abs(p.y - dragStart.y);
    showSelection(x, y, width, height, false);
    return;
  }

  if (snapLookupInFlight) return;
  snapLookupInFlight = true;
  try {
    const globalX = frame.x + p.x;
    const globalY = frame.y + p.y;
    const rect = await invoke<WindowRect | null>("window_rect_at_point", {
      x: globalX,
      y: globalY,
    });
    if (rect) {
      const localX = clamp(rect[0] - frame.x, 0, frame.width);
      const localY = clamp(rect[1] - frame.y, 0, frame.height);
      const width = clamp(rect[2], 0, frame.width - localX);
      const height = clamp(rect[3], 0, frame.height - localY);
      windowSnapRect = [localX, localY, width, height] as unknown as WindowRect;
      showSelection(localX, localY, width, height, true);
    } else {
      windowSnapRect = null;
      hideSelection();
    }
  } finally {
    snapLookupInFlight = false;
  }
});

window.addEventListener("mouseup", (e) => {
  if (!frame) return;
  const p = toPhysical(e.clientX, e.clientY);

  if (dragStart) {
    const x = Math.min(dragStart.x, p.x);
    const y = Math.min(dragStart.y, p.y);
    const width = Math.abs(p.x - dragStart.x);
    const height = Math.abs(p.y - dragStart.y);
    dragStart = null;

    if (width > 4 && height > 4) {
      finish(x, y, width, height);
      return;
    }
    hideSelection();
  }

  if (windowSnapRect) {
    const [x, y, width, height] = windowSnapRect as unknown as [number, number, number, number];
    if (width > 0 && height > 0) {
      finish(x, y, width, height);
    }
  }
});

loadFrame();
