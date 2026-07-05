// Scene model for the editor: shapes are plain data objects re-rendered
// every frame, so everything stays movable/undoable until export.

export interface Point {
  x: number;
  y: number;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface StrokeShape {
  kind: "pen" | "highlighter" | "neon";
  points: Point[];
  color: string;
  width: number;
}

export interface ArrowShape {
  kind: "arrow";
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  color: string;
  width: number;
}

export interface BoxShape {
  kind: "rect" | "ellipse";
  x: number;
  y: number;
  w: number;
  h: number;
  color: string;
  width: number;
}

export interface TextShape {
  kind: "text";
  x: number;
  y: number;
  text: string;
  color: string;
  size: number;
  /** Instagram-style filled pill behind the text. */
  bg?: boolean;
}

export interface BlurShape {
  kind: "blur";
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface BadgeShape {
  kind: "badge";
  x: number;
  y: number;
  n: number;
  color: string;
}

export type Shape = StrokeShape | ArrowShape | BoxShape | TextShape | BlurShape | BadgeShape;

export interface SceneState {
  shapes: Shape[];
  crop: Rect;
}

export function cloneState(state: SceneState): SceneState {
  return JSON.parse(JSON.stringify(state));
}

function normalizedRect(r: Rect): Rect {
  return {
    x: Math.min(r.x, r.x + r.w),
    y: Math.min(r.y, r.y + r.h),
    w: Math.abs(r.w),
    h: Math.abs(r.h),
  };
}

/// Renders the scene (image + shapes, cropped) onto `ctx`. `base` holds the
/// untouched screenshot; blur shapes always sample from it so redactions
/// can't be peeled back out of the export.
export function renderScene(
  ctx: CanvasRenderingContext2D,
  base: HTMLCanvasElement,
  state: SceneState,
  selected: Shape | null,
  handleSize = 8,
) {
  const { crop } = state;
  ctx.canvas.width = Math.max(1, Math.round(crop.w));
  ctx.canvas.height = Math.max(1, Math.round(crop.h));
  ctx.save();
  ctx.translate(-crop.x, -crop.y);

  ctx.drawImage(base, 0, 0);

  for (const shape of state.shapes) {
    drawShape(ctx, base, shape);
  }

  if (selected) {
    drawSelectionOutline(ctx, selected, handleSize);
  }
  ctx.restore();
}

function drawShape(ctx: CanvasRenderingContext2D, base: HTMLCanvasElement, shape: Shape) {
  ctx.save();
  switch (shape.kind) {
    case "pen":
      drawStroke(ctx, shape, 1, shape.width, "source-over");
      break;
    case "highlighter":
      drawStroke(ctx, shape, 0.4, shape.width * 3, "multiply");
      break;
    case "neon": {
      // Instagram-style glow pen: colored halo with a near-white core.
      ctx.shadowColor = shape.color;
      ctx.shadowBlur = shape.width * 2.2;
      drawStroke(ctx, shape, 1, shape.width, "source-over");
      ctx.shadowBlur = shape.width * 1.2;
      drawStroke(ctx, shape, 0.9, Math.max(1.5, shape.width * 0.4), "source-over", "#ffffff");
      break;
    }
    case "arrow":
      drawArrow(ctx, shape);
      break;
    case "rect": {
      applyShadow(ctx);
      ctx.strokeStyle = shape.color;
      ctx.lineWidth = shape.width;
      ctx.lineJoin = "round";
      const r = normalizedRect(shape);
      ctx.strokeRect(r.x, r.y, r.w, r.h);
      break;
    }
    case "ellipse": {
      applyShadow(ctx);
      ctx.strokeStyle = shape.color;
      ctx.lineWidth = shape.width;
      const r = normalizedRect(shape);
      ctx.beginPath();
      ctx.ellipse(r.x + r.w / 2, r.y + r.h / 2, r.w / 2, r.h / 2, 0, 0, Math.PI * 2);
      ctx.stroke();
      break;
    }
    case "text": {
      ctx.font = `700 ${shape.size}px Inter, system-ui, sans-serif`;
      ctx.textBaseline = "top";
      if (shape.bg) {
        // Filled pill in the shape's color; text flips light/dark for contrast.
        const w = ctx.measureText(shape.text).width;
        const padX = shape.size * 0.45;
        const padY = shape.size * 0.28;
        applyShadow(ctx);
        ctx.fillStyle = shape.color;
        roundedRect(
          ctx,
          shape.x - padX,
          shape.y - padY,
          w + padX * 2,
          shape.size * 1.15 + padY * 2,
          shape.size * 0.45,
        );
        ctx.fill();
        ctx.shadowColor = "transparent";
        ctx.fillStyle = isLightColor(shape.color) ? "#1d1d1f" : "#ffffff";
      } else {
        applyShadow(ctx);
        ctx.fillStyle = shape.color;
      }
      ctx.fillText(shape.text, shape.x, shape.y);
      break;
    }
    case "blur":
      drawPixelated(ctx, base, normalizedRect(shape));
      break;
    case "badge": {
      applyShadow(ctx);
      const radius = 16;
      ctx.fillStyle = shape.color;
      ctx.beginPath();
      ctx.arc(shape.x, shape.y, radius, 0, Math.PI * 2);
      ctx.fill();
      ctx.shadowColor = "transparent";
      ctx.fillStyle = "#ffffff";
      ctx.font = "700 17px Inter, system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(String(shape.n), shape.x, shape.y + 1);
      break;
    }
  }
  ctx.restore();
}

function applyShadow(ctx: CanvasRenderingContext2D) {
  ctx.shadowColor = "rgba(0, 0, 0, 0.35)";
  ctx.shadowBlur = 6;
  ctx.shadowOffsetX = 1;
  ctx.shadowOffsetY = 2;
}

function roundedRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  const radius = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + radius, y);
  ctx.arcTo(x + w, y, x + w, y + h, radius);
  ctx.arcTo(x + w, y + h, x, y + h, radius);
  ctx.arcTo(x, y + h, x, y, radius);
  ctx.arcTo(x, y, x + w, y, radius);
  ctx.closePath();
}

export function isLightColor(hex: string): boolean {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex);
  if (!m) return false;
  const v = parseInt(m[1], 16);
  const r = (v >> 16) & 0xff;
  const g = (v >> 8) & 0xff;
  const b = v & 0xff;
  return 0.299 * r + 0.587 * g + 0.114 * b > 160;
}

function drawStroke(
  ctx: CanvasRenderingContext2D,
  shape: StrokeShape,
  alpha: number,
  width: number,
  composite: GlobalCompositeOperation,
  colorOverride?: string,
) {
  if (shape.points.length < 2) return;
  ctx.globalAlpha = alpha;
  ctx.globalCompositeOperation = composite;
  ctx.strokeStyle = colorOverride ?? shape.color;
  ctx.lineWidth = width;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.beginPath();
  ctx.moveTo(shape.points[0].x, shape.points[0].y);
  // Quadratic smoothing through midpoints.
  for (let i = 1; i < shape.points.length - 1; i++) {
    const mid = {
      x: (shape.points[i].x + shape.points[i + 1].x) / 2,
      y: (shape.points[i].y + shape.points[i + 1].y) / 2,
    };
    ctx.quadraticCurveTo(shape.points[i].x, shape.points[i].y, mid.x, mid.y);
  }
  const last = shape.points[shape.points.length - 1];
  ctx.lineTo(last.x, last.y);
  ctx.stroke();
}

function drawArrow(ctx: CanvasRenderingContext2D, shape: ArrowShape) {
  applyShadow(ctx);
  ctx.strokeStyle = shape.color;
  ctx.fillStyle = shape.color;
  ctx.lineWidth = shape.width;
  ctx.lineCap = "round";

  const angle = Math.atan2(shape.y2 - shape.y1, shape.x2 - shape.x1);
  const headLength = Math.max(12, shape.width * 3);
  // Stop the shaft short of the tip so it doesn't poke through the head.
  const shaftX = shape.x2 - headLength * 0.6 * Math.cos(angle);
  const shaftY = shape.y2 - headLength * 0.6 * Math.sin(angle);

  ctx.beginPath();
  ctx.moveTo(shape.x1, shape.y1);
  ctx.lineTo(shaftX, shaftY);
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(shape.x2, shape.y2);
  ctx.lineTo(
    shape.x2 - headLength * Math.cos(angle - Math.PI / 7),
    shape.y2 - headLength * Math.sin(angle - Math.PI / 7),
  );
  ctx.lineTo(
    shape.x2 - headLength * Math.cos(angle + Math.PI / 7),
    shape.y2 - headLength * Math.sin(angle + Math.PI / 7),
  );
  ctx.closePath();
  ctx.fill();
}

function drawPixelated(ctx: CanvasRenderingContext2D, base: HTMLCanvasElement, r: Rect) {
  if (r.w < 2 || r.h < 2) return;
  const block = 14;
  const small = document.createElement("canvas");
  small.width = Math.max(1, Math.round(r.w / block));
  small.height = Math.max(1, Math.round(r.h / block));
  const sctx = small.getContext("2d")!;
  sctx.imageSmoothingEnabled = true;
  sctx.drawImage(base, r.x, r.y, r.w, r.h, 0, 0, small.width, small.height);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(small, 0, 0, small.width, small.height, r.x, r.y, r.w, r.h);
  ctx.imageSmoothingEnabled = true;
}

function drawSelectionOutline(ctx: CanvasRenderingContext2D, shape: Shape, handleSize: number) {
  const b = selectionBounds(shape);
  ctx.save();
  ctx.strokeStyle = "#0071e3";
  ctx.lineWidth = handleSize / 5;
  ctx.setLineDash([handleSize * 0.75, handleSize * 0.5]);
  ctx.strokeRect(b.x, b.y, b.w, b.h);

  // Resize grip at the bottom-right corner.
  ctx.setLineDash([]);
  ctx.fillStyle = "#ffffff";
  ctx.strokeStyle = "#0071e3";
  ctx.lineWidth = handleSize / 4;
  ctx.beginPath();
  ctx.arc(b.x + b.w, b.y + b.h, handleSize * 0.75, 0, Math.PI * 2);
  ctx.fill();
  ctx.stroke();
  ctx.restore();
}

/// Selection box: shape bounds plus a small margin so the frame never sits
/// on top of the artwork itself.
export function selectionBounds(shape: Shape): Rect {
  const b = shapeBounds(shape);
  const pad = 6;
  return { x: b.x - pad, y: b.y - pad, w: b.w + pad * 2, h: b.h + pad * 2 };
}

export function shapeBounds(shape: Shape): Rect {
  switch (shape.kind) {
    case "pen":
    case "highlighter":
    case "neon": {
      let minX = Infinity,
        minY = Infinity,
        maxX = -Infinity,
        maxY = -Infinity;
      for (const p of shape.points) {
        minX = Math.min(minX, p.x);
        minY = Math.min(minY, p.y);
        maxX = Math.max(maxX, p.x);
        maxY = Math.max(maxY, p.y);
      }
      return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
    }
    case "arrow":
      return {
        x: Math.min(shape.x1, shape.x2),
        y: Math.min(shape.y1, shape.y2),
        w: Math.abs(shape.x2 - shape.x1),
        h: Math.abs(shape.y2 - shape.y1),
      };
    case "rect":
    case "ellipse":
    case "blur":
      return normalizedRect(shape);
    case "text":
      // Approximate: measured properly at draw time isn't worth the plumbing.
      return { x: shape.x, y: shape.y, w: shape.text.length * shape.size * 0.6, h: shape.size * 1.2 };
    case "badge":
      return { x: shape.x - 16, y: shape.y - 16, w: 32, h: 32 };
  }
}

export function hitTest(shapes: Shape[], p: Point): Shape | null {
  for (let i = shapes.length - 1; i >= 0; i--) {
    const b = shapeBounds(shapes[i]);
    const pad = 6;
    if (p.x >= b.x - pad && p.x <= b.x + b.w + pad && p.y >= b.y - pad && p.y <= b.y + b.h + pad) {
      return shapes[i];
    }
  }
  return null;
}

export function moveShape(shape: Shape, dx: number, dy: number) {
  switch (shape.kind) {
    case "pen":
    case "highlighter":
    case "neon":
      for (const p of shape.points) {
        p.x += dx;
        p.y += dy;
      }
      break;
    case "arrow":
      shape.x1 += dx;
      shape.y1 += dy;
      shape.x2 += dx;
      shape.y2 += dy;
      break;
    case "rect":
    case "ellipse":
    case "blur":
    case "text":
    case "badge":
      shape.x += dx;
      shape.y += dy;
      break;
  }
}

export function cloneShape<T extends Shape>(shape: T): T {
  return JSON.parse(JSON.stringify(shape));
}

/// Uniformly scales a shape about `anchor`. Stroke widths and text sizes
/// scale with it so proportions hold.
export function scaleShape(shape: Shape, f: number, anchor: Point) {
  const sx = (x: number) => anchor.x + (x - anchor.x) * f;
  const sy = (y: number) => anchor.y + (y - anchor.y) * f;
  switch (shape.kind) {
    case "pen":
    case "highlighter":
    case "neon":
      for (const p of shape.points) {
        p.x = sx(p.x);
        p.y = sy(p.y);
      }
      shape.width = Math.max(1, shape.width * f);
      break;
    case "arrow":
      shape.x1 = sx(shape.x1);
      shape.y1 = sy(shape.y1);
      shape.x2 = sx(shape.x2);
      shape.y2 = sy(shape.y2);
      shape.width = Math.max(1, shape.width * f);
      break;
    case "rect":
    case "ellipse":
      shape.x = sx(shape.x);
      shape.y = sy(shape.y);
      shape.w *= f;
      shape.h *= f;
      shape.width = Math.max(1, shape.width * f);
      break;
    case "blur":
      shape.x = sx(shape.x);
      shape.y = sy(shape.y);
      shape.w *= f;
      shape.h *= f;
      break;
    case "text":
      shape.x = sx(shape.x);
      shape.y = sy(shape.y);
      shape.size = Math.max(8, shape.size * f);
      break;
    case "badge":
      // Badges keep their size; only their position scales.
      shape.x = sx(shape.x);
      shape.y = sy(shape.y);
      break;
  }
}
