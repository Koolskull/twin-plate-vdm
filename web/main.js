const SHAPES = [
  {
    id: 0,
    name: "Sphere",
    blurb:
      "One center plate, two faces. Up grows the top of the shell. Down grows the bottom. They meet around the equator.",
  },
  {
    id: 1,
    name: "Box",
    blurb:
      "Both faces start on the center plane. Up stops on the top cap, down stops on the bottom cap, and the sides are the seam between those two footprints.",
  },
  {
    id: 2,
    name: "Torus",
    blurb:
      "The hole is empty on both faces. Around the ring, up and down grow the tube outward from the same center.",
  },
  {
    id: 3,
    name: "Cone",
    blurb:
      "Up from the center reaches the slope. Down reaches the base. A finer mesh samples the same two images.",
  },
  {
    id: 4,
    name: "Capsule",
    blurb:
      "Up reaches the top cap. Down reaches the bottom cap. Both start on the center plane inside the capsule.",
  },
  {
    id: 5,
    name: "Mushroom",
    blurb:
      "Down from the center reaches the stem. Up reaches the cap, including the ring past the stem.",
  },
  {
    id: 6,
    name: "Ripple",
    blurb:
      "The bumps are pixels grown outward from the center. A coarser mesh samples fewer of them. A finer mesh only interpolates.",
  },
];

const BAKE_STOPS = [16, 24, 32, 48, 64, 96, 128, 256];

const BRUSHES = [
  {
    id: 0,
    name: "Horn",
    blurb: "A horn leans as it rises. The map has to store sideways displacement, not just height.",
  },
  {
    id: 1,
    name: "Crater",
    blurb: "A crater drops in the middle and lifts a rim. The vectors point down as well as up.",
  },
  {
    id: 2,
    name: "Ridge",
    blurb: "A ridge overhangs one side. The lip is past the base, which a height map cannot do.",
  },
];

const state = {
  shape: 0,
  lean: 0.35,
  bake: 128,
  mesh: 32,
  container: 0,
  kind: "formula",
  brush: 0,
  solid: true,
  wire: true,
  plates: true,
  vectors: true,
  spin: true,
  dragging: false,
  theta: 0.62,
  phi: 1.02,
  radius: 3.55,
  token: 0,
};

const wasm = {
  exports: null,
  memory: null,
  header: null,
  pngPlus: null,
  pngMinus: null,
  urls: [],
};

const glState = {
  gl: null,
  program: null,
  wireProgram: null,
  plateProgram: null,
  lineProgram: null,
  mesh: null,
  wire: null,
  plates: null,
  arrows: null,
  frames: null,
  textures: [null, null],
  indexCount: 0,
  wireCount: 0,
  arrowCount: 0,
};

const $ = (id) => document.getElementById(id);

function readHeader() {
  const ptr = wasm.exports.header_ptr();
  const view = new DataView(wasm.memory.buffer, ptr, 136);
  const u = (offset) => view.getUint32(offset, true);
  const f = (offset) => view.getFloat32(offset, true);
  return {
    status: u(0),
    shape: u(4),
    bakeRes: u(8),
    meshRes: u(12),
    pngPlusPtr: u(16),
    pngPlusLen: u(20),
    pngMinusPtr: u(24),
    pngMinusLen: u(28),
    vertPtr: u(32),
    vertCount: u(36),
    vertStride: u(40),
    indexPtr: u(44),
    indexCount: u(48),
    arrowPtr: u(52),
    arrowCount: u(56),
    validPlus: u(60),
    validMinus: u(64),
    sampleCount: u(68),
    triCount: u(72),
    clamped: u(76),
    centerPlusOk: u(80),
    centerMinusOk: u(84),
    quantError: f(88),
    surfaceError: f(92),
    opposing: f(96),
    plateY: f(100),
    span: f(104),
    range: f(108),
    centerPlus: [f(112), f(116), f(120)],
    centerMinus: [f(124), f(128), f(132)],
  };
}

function copyRange(ptr, len) {
  return new Uint8Array(wasm.memory.buffer).slice(ptr, ptr + len);
}

function wasmBytes() {
  const encoded = globalThis.TWIN_WASM_BASE64;
  if (!encoded) return null;
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

async function loadWasm() {
  let bytes = wasmBytes();
  if (!bytes) {
    const response = await fetch("twin_plate_vdm.wasm");
    if (!response.ok) {
      throw new Error(`Could not fetch twin_plate_vdm.wasm (${response.status}).`);
    }
    bytes = await response.arrayBuffer();
  }
  const { instance } = await WebAssembly.instantiate(bytes, {});
  wasm.exports = instance.exports;
  wasm.memory = instance.exports.memory;
  if (!wasm.exports.bake || !wasm.exports.header_ptr) {
    throw new Error("The wasm module is missing the twin-plate exports.");
  }
}

function bindUI() {
  const shapes = $("shapes");
  for (const shape of SHAPES) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = shape.name;
    button.dataset.id = String(shape.id);
    button.addEventListener("click", () => {
      state.kind = "formula";
      state.shape = shape.id;
      markShapes();
      scheduleBake();
    });
    shapes.append(button);
  }
  const brushes = $("brushes");
  for (const brush of BRUSHES) {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = brush.name;
    button.dataset.id = String(brush.id);
    button.addEventListener("click", () => {
      state.kind = "brush";
      state.brush = brush.id;
      markShapes();
      scheduleBake();
    });
    brushes.append(button);
  }
  for (const button of $("formats").querySelectorAll("button")) {
    button.addEventListener("click", () => {
      state.container = Number(button.dataset.container);
      if (state.kind === "file") state.kind = "formula";
      markShapes();
      scheduleBake();
    });
  }
  $("file").addEventListener("change", () => {
    if ($("file").files.length) {
      loadFiles($("file").files).catch((error) => showFatal(error));
    }
  });
  const drop = $("drop");
  window.addEventListener("dragover", (event) => {
    if (![...event.dataTransfer.types].includes("Files")) return;
    event.preventDefault();
    drop.classList.add("hot");
  });
  window.addEventListener("dragleave", () => drop.classList.remove("hot"));
  window.addEventListener("drop", (event) => {
    if (!event.dataTransfer.files.length) return;
    event.preventDefault();
    drop.classList.remove("hot");
    loadFiles(event.dataTransfer.files).catch((error) => showFatal(error));
  });
  markShapes();

  $("lean").addEventListener("input", () => {
    state.lean = Number($("lean").value);
    $("lean-val").textContent = state.lean.toFixed(2);
    if (state.kind === "file") state.kind = "formula";
    markShapes();
    scheduleBake();
  });
  $("bake").addEventListener("input", () => {
    state.bake = BAKE_STOPS[Number($("bake").value)];
    $("bake-val").textContent = `${state.bake}²`;
    if (state.kind === "file") state.kind = "formula";
    markShapes();
    scheduleBake();
  });
  $("mesh").addEventListener("input", () => {
    state.mesh = Number($("mesh").value);
    $("mesh-val").textContent = `${state.mesh}²`;
    scheduleMesh();
  });
  for (const button of document.querySelectorAll("[data-mesh]")) {
    button.addEventListener("click", () => {
      state.mesh = Number(button.dataset.mesh);
      $("mesh").value = String(state.mesh);
      $("mesh-val").textContent = `${state.mesh}²`;
      scheduleMesh();
    });
  }
  for (const [id, key] of [
    ["show-solid", "solid"],
    ["show-wire", "wire"],
    ["show-plates", "plates"],
    ["show-vectors", "vectors"],
    ["spin", "spin"],
  ]) {
    $(id).addEventListener("change", () => {
      state[key] = $(id).checked;
    });
  }
  $("dl-plus").addEventListener("click", () => download(wasm.pngPlus, $("name-plus").textContent));
  $("dl-minus").addEventListener("click", () => download(wasm.pngMinus, $("name-minus").textContent));
  window.addEventListener("keydown", (event) => {
    const index = Number(event.key) - 1;
    if (index >= 0 && index < SHAPES.length && !event.metaKey && !event.ctrlKey) {
      state.kind = "formula";
      state.shape = index;
      markShapes();
      scheduleBake();
    }
  });
  $("lean-val").textContent = state.lean.toFixed(2);
  $("bake-val").textContent = `${state.bake}²`;
  $("mesh-val").textContent = `${state.mesh}²`;
  $("bake").value = String(BAKE_STOPS.indexOf(state.bake));
}

function markShapes() {
  for (const button of $("shapes").querySelectorAll("button")) {
    const on = state.kind === "formula" && button.dataset.id === String(state.shape);
    button.setAttribute("aria-pressed", on ? "true" : "false");
  }
  for (const button of $("brushes").querySelectorAll("button")) {
    const on = state.kind === "brush" && button.dataset.id === String(state.brush);
    button.setAttribute("aria-pressed", on ? "true" : "false");
  }
  for (const button of $("formats").querySelectorAll("button")) {
    const on = Number(button.dataset.container) === state.container;
    button.setAttribute("aria-pressed", on ? "true" : "false");
  }
  if (state.kind === "brush") {
    $("blurb").textContent = "The formula solids are sitting this one out. A brush is a single upward plate.";
    $("brush-blurb").textContent = BRUSHES[state.brush].blurb;
  } else if (state.kind === "file") {
    $("blurb").textContent = "This mesh was decoded from the files you dropped. Pick a shape or a brush to bake again.";
    $("brush-blurb").textContent = "A dropped OpenEXR with no downward plate is read as a brush.";
  } else {
    $("blurb").textContent = SHAPES[state.shape].blurb;
    $("brush-blurb").textContent = "One image, the sculpt-brush kind of vector displacement. The downward plate stays empty.";
  }
}

let bakeTimer = 0;
let meshTimer = 0;

function scheduleBake() {
  clearTimeout(bakeTimer);
  bakeTimer = setTimeout(() => {
    rebake().catch((error) => showFatal(error));
  }, 40);
}

function scheduleMesh() {
  clearTimeout(meshTimer);
  meshTimer = setTimeout(() => {
    rebuildMesh().catch((error) => showFatal(error));
  }, 16);
}

async function rebake() {
  const token = ++state.token;
  $("res-note").textContent = state.kind === "brush" ? "Writing the brush into one plate…" : "Casting rays into the two plates…";
  await new Promise((resolve) => setTimeout(resolve, 16));
  if (token !== state.token) return;

  wasm.exports.set_container(state.container);
  const bakeStatus = state.kind === "brush"
    ? wasm.exports.bake_brush(state.brush, state.bake)
    : wasm.exports.bake(state.shape, state.bake, state.lean);
  wasm.memory = wasm.exports.memory;
  if (bakeStatus !== 0) {
    throw new Error(`Bake failed with status ${bakeStatus}.`);
  }
  const baked = readHeader();
  const pngPlus = copyRange(baked.pngPlusPtr, baked.pngPlusLen);
  const pngMinus = copyRange(baked.pngMinusPtr, baked.pngMinusLen);
  wasm.exports.wipe();
  wasm.memory = wasm.exports.memory;
  const fresh = readHeader();
  new Uint8Array(wasm.memory.buffer).set(pngPlus, fresh.pngPlusPtr);
  new Uint8Array(wasm.memory.buffer).set(pngMinus, fresh.pngMinusPtr);
  const loadStatus = wasm.exports.load_pngs(
    fresh.pngPlusPtr,
    pngPlus.length,
    fresh.pngMinusPtr,
    pngMinus.length,
  );
  wasm.memory = wasm.exports.memory;
  if (loadStatus !== 0) {
    throw new Error(`Decode failed with status ${loadStatus}.`);
  }
  wasm.pngPlus = pngPlus;
  wasm.pngMinus = pngMinus;
  wasm.header = readHeader();
  showPngs(token);
  await rebuildMesh();
}

async function loadFiles(fileList) {
  const files = [...fileList].slice(0, 2);
  if (!files.length) return;
  const blobs = await Promise.all(files.map(async (file) => ({
    name: file.name.toLowerCase(),
    bytes: new Uint8Array(await file.arrayBuffer()),
  })));
  if (blobs.some((file) => file.bytes.length > 1_250_000)) {
    throw new Error("That file is over 1.2 MB. Plates here stop at 256².");
  }
  if (blobs.some((file) => !isPlate(file.bytes))) {
    throw new Error("Those need to be OpenEXR or 16-bit PNG plates.");
  }
  const token = ++state.token;
  wasm.exports.wipe();
  wasm.memory = wasm.exports.memory;
  const fresh = readHeader();
  const mem = new Uint8Array(wasm.memory.buffer);
  let status;
  if (blobs.length === 1) {
    mem.set(blobs[0].bytes, fresh.pngPlusPtr);
    status = wasm.exports.load_brush(fresh.pngPlusPtr, blobs[0].bytes.length);
  } else {
    let plus = blobs[0];
    let minus = blobs[1];
    const plusName = (file) => file.name.includes("plus") || file.name.includes("horn") || file.name.includes("crater") || file.name.includes("ridge");
    if (minus.name.includes("plus") && !plus.name.includes("plus")) [plus, minus] = [minus, plus];
    if (plus.name.includes("minus")) [plus, minus] = [minus, plus];
    if (!plusName(plus) && plusName(minus)) [plus, minus] = [minus, plus];
    mem.set(plus.bytes, fresh.pngPlusPtr);
    mem.set(minus.bytes, fresh.pngMinusPtr);
    status = wasm.exports.load_pngs(
      fresh.pngPlusPtr,
      plus.bytes.length,
      fresh.pngMinusPtr,
      minus.bytes.length,
    );
  }
  wasm.memory = wasm.exports.memory;
  if (status !== 0) {
    throw new Error(`Could not read that file (status ${status}). ZIP, ZIPS, RLE, and uncompressed scanlines are fine. Tiled EXR is not.`);
  }
  if (token !== state.token) return;
  state.kind = "file";
  wasm.header = readHeader();
  wasm.pngPlus = copyRange(wasm.header.pngPlusPtr, wasm.header.pngPlusLen);
  wasm.pngMinus = copyRange(wasm.header.pngMinusPtr, wasm.header.pngMinusLen);
  state.container = fileExt(wasm.pngPlus) === "exr" ? 1 : 0;
  markShapes();
  showPngs(token);
  await rebuildMesh();
}

function isPlate(bytes) {
  if (bytes.length >= 4 && bytes[0] === 0x76 && bytes[1] === 0x2f && bytes[2] === 0x31 && bytes[3] === 0x01) return true;
  return bytes.length >= 8 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47;
}

function fileExt(bytes) {
  return bytes && bytes[0] === 0x76 && bytes[1] === 0x2f ? "exr" : "png";
}

async function rebuildMesh() {
  const status = wasm.exports.reconstruct(state.mesh);
  wasm.memory = wasm.exports.memory;
  if (status !== 0) {
    throw new Error(`Reconstruct failed with status ${status}.`);
  }
  wasm.header = readHeader();
  uploadMesh();
  renderStats();
}

function showPngs(token) {
  paintPlate($("plate-plus"), 0);
  paintPlate($("plate-minus"), 1);
  if (token === state.token) {
    uploadPlateTexture(0, $("plate-plus"));
    uploadPlateTexture(1, $("plate-minus"));
  }
  const ext = fileExt(wasm.pngPlus);
  const h = wasm.header;
  const brush = h.validMinus === 0;
  $("name-plus").textContent = brush ? `brush.${ext}` : `plate-plus.${ext}`;
  $("name-minus").textContent = brush ? `empty.${ext}` : `plate-minus.${ext}`;
  $("cap-plus").textContent = `${h.bakeRes}² · ${formatBytes(wasm.pngPlus.length)} · ${h.validPlus.toLocaleString()} hits`;
  $("cap-minus").textContent = `${h.bakeRes}² · ${formatBytes(wasm.pngMinus.length)} · ${h.validMinus.toLocaleString()} hits`;
}

function paintPlate(canvas, plate) {
  const h = wasm.header;
  const res = h.bakeRes;
  if (!res) return;
  const ptr = wasm.exports.plate_ptr(plate) >>> 0;
  const texels = new Uint16Array(wasm.memory.buffer, ptr, res * res * 4);
  canvas.width = res;
  canvas.height = res;
  const ctx = canvas.getContext("2d");
  const image = ctx.createImageData(res, res);
  const dst = image.data;
  for (let i = 0; i < res * res; i++) {
    const o = i * 4;
    dst[o] = texels[o] >> 8;
    dst[o + 1] = texels[o + 1] >> 8;
    dst[o + 2] = texels[o + 2] >> 8;
    dst[o + 3] = texels[o + 3] >= 32768 ? 255 : 0;
  }
  ctx.putImageData(image, 0, 0);
}

function renderStats() {
  const h = wasm.header;
  const pair = wasm.pngPlus.length + wasm.pngMinus.length;
  const fileWord = fileExt(wasm.pngPlus) === "exr" ? "OpenEXR" : "PNG";
  const relation = state.kind === "brush" || h.validMinus === 0
    ? "One plate. Misses stay off the mesh, and the downward file is an empty mask."
    : state.mesh === h.bakeRes
      ? `This grid matches the ${fileWord}, so each vertex is one texel.`
      : state.mesh < h.bakeRes
        ? "This grid samples fewer texels from the same files."
        : `This grid is finer than the ${fileWord}, so it interpolates. Detail still lives in the files.`;
  $("res-note").textContent = relation;
  $("lean-note").textContent = state.kind === "brush"
    ? "Lean is part of the brush, not a separate ray splay. The horn and the ridge already lean sideways."
    : state.lean === 0
      ? "Lean is zero, so each vector runs straight out from the center and the color is mostly green."
      : "Lean splays both rays outward. Red and blue wake up, and the shell reaches around the sides.";
  const rows = [
    ["Triangles", h.triCount.toLocaleString()],
    ["Shared samples", h.sampleCount.toLocaleString()],
    ["Plate + hits", h.validPlus.toLocaleString()],
    ["Plate − hits", h.validMinus.toLocaleString()],
    ["Image pair", formatBytes(pair)],
    ["Facing", `${(h.opposing * 100).toFixed(1)}%`],
    ["Quantize", h.quantError.toExponential(2)],
    ["On surface", h.surfaceError.toFixed(4)],
  ];
  $("stats").replaceChildren(
    ...rows.flatMap(([name, value]) => {
      const dt = document.createElement("dt");
      dt.textContent = name;
      const dd = document.createElement("dd");
      dd.textContent = value;
      return [dt, dd];
    }),
  );
  $("vectors").textContent = [
    vectorLine("plate +", h.centerPlus, h.centerPlusOk, "out, up"),
    vectorLine("plate −", h.centerMinus, h.centerMinusOk, "out, down"),
    h.centerPlusOk && h.centerMinusOk
      ? "Center texels point away from each other, out from the shared plate."
      : "A miss here is an empty texel, as in the torus hole.",
  ].join("\n");
}

function vectorLine(label, vector, ok, direction) {
  if (!ok) return `${label}  miss`;
  const [x, y, z] = vector.map((n) => (n >= 0 ? "+" : "") + n.toFixed(3));
  return `${label}  (${x}, ${y}, ${z})  ${direction}`;
}

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

function download(bytes, name) {
  if (!bytes) return;
  const ext = fileExt(bytes);
  const type = ext === "exr" ? "image/x-exr" : "image/png";
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function showFatal(error) {
  const message = error && error.message ? error.message : String(error);
  const node = $("fatal");
  node.hidden = false;
  node.textContent = message;
  const note = $("gl-note");
  note.hidden = false;
  note.textContent = message;
  document.documentElement.dataset.twin = `fail ${message}`;
}

function compile(gl, type, source) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    throw new Error(gl.getShaderInfoLog(shader) || "shader compile failed");
  }
  return shader;
}

function program(gl, vs, fs) {
  const prog = gl.createProgram();
  gl.attachShader(prog, compile(gl, gl.VERTEX_SHADER, vs));
  gl.attachShader(prog, compile(gl, gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    throw new Error(gl.getProgramInfoLog(prog) || "program link failed");
  }
  return prog;
}

const MESH_VS = `#version 300 es
layout(location=0) in vec3 aPos;
uniform mat4 uMvp;
out vec3 vWorld;
void main() {
  vWorld = aPos;
  gl_Position = uMvp * vec4(aPos, 1.0);
}`;

const MESH_FS = `#version 300 es
precision highp float;
in vec3 vWorld;
uniform vec3 uEye;
uniform float uAlpha;
out vec4 frag;
void main() {
  vec3 n = normalize(cross(dFdx(vWorld), dFdy(vWorld)));
  vec3 view = normalize(uEye - vWorld);
  if (dot(n, view) < 0.0) n = -n;
  vec3 key = normalize(vec3(0.42, 0.82, 0.35));
  vec3 fill = normalize(vec3(-0.55, 0.15, -0.45));
  float lambert = clamp(dot(n, key), 0.0, 1.0);
  float bounce = clamp(dot(n, fill), 0.0, 1.0);
  vec3 clay = vec3(0.855, 0.816, 0.745);
  vec3 col = clay * (0.20 + 0.72 * lambert + 0.16 * bounce);
  col += vec3(0.05, 0.045, 0.03) * n.y;
  float fres = pow(1.0 - clamp(dot(n, view), 0.0, 1.0), 2.2);
  col += vec3(0.16, 0.14, 0.10) * fres;
  frag = vec4(col, uAlpha);
}`;

const WIRE_VS = `#version 300 es
layout(location=0) in vec3 aPos;
uniform mat4 uMvp;
void main() {
  vec4 clip = uMvp * vec4(aPos, 1.0);
  clip.z -= 0.0012 * clip.w;
  gl_Position = clip;
}`;

const WIRE_FS = `#version 300 es
precision highp float;
out vec4 frag;
void main() { frag = vec4(0.20, 0.15, 0.10, 1.0); }`;

const PLATE_VS = `#version 300 es
layout(location=0) in vec3 aPos;
layout(location=1) in vec2 aUv;
uniform mat4 uMvp;
out vec2 vUv;
void main() {
  vUv = aUv;
  gl_Position = uMvp * vec4(aPos, 1.0);
}`;

const PLATE_FS = `#version 300 es
precision highp float;
in vec2 vUv;
uniform sampler2D uTexPlus;
uniform sampler2D uTexMinus;
out vec4 frag;
void main() {
  // One plane. The side facing +Y is the upward plate, the side facing -Y is the downward plate.
  bool up = gl_FrontFacing;
  vec4 tex = up ? texture(uTexPlus, vUv) : texture(uTexMinus, vUv);
  vec3 tint = up ? vec3(0.84, 0.95, 0.36) : vec3(1.0, 0.36, 0.22);
  vec3 empty = vec3(0.11, 0.10, 0.085);
  vec3 col = mix(empty, tex.rgb, tex.a);
  float gx = step(0.985, fract(vUv.x * 8.0));
  float gy = step(0.985, fract(vUv.y * 8.0));
  col = mix(col, tint, 0.28 + (gx + gy) * 0.4);
  frag = vec4(col, mix(0.22, 0.78, tex.a));
}`;

const LINE_VS = `#version 300 es
layout(location=0) in vec3 aPos;
layout(location=1) in vec3 aColor;
uniform mat4 uMvp;
out vec3 vColor;
void main() {
  vColor = aColor;
  vec4 clip = uMvp * vec4(aPos, 1.0);
  clip.z -= 0.0008 * clip.w;
  gl_Position = clip;
}`;

const LINE_FS = `#version 300 es
precision highp float;
in vec3 vColor;
out vec4 frag;
void main() { frag = vec4(vColor, 0.95); }`;

function initGl() {
  const canvas = $("view");
  const gl = canvas.getContext("webgl2", { antialias: true, alpha: false });
  if (!gl) {
    $("gl-note").hidden = false;
    $("gl-note").textContent = "WebGL2 is unavailable. The plates below are still the model.";
    return false;
  }
  glState.gl = gl;
  glState.program = program(gl, MESH_VS, MESH_FS);
  glState.wireProgram = program(gl, WIRE_VS, WIRE_FS);
  glState.plateProgram = program(gl, PLATE_VS, PLATE_FS);
  glState.lineProgram = program(gl, LINE_VS, LINE_FS);
  glState.mesh = gl.createBuffer();
  glState.wire = gl.createBuffer();
  glState.index = gl.createBuffer();
  glState.plates = gl.createBuffer();
  glState.arrows = gl.createBuffer();
  glState.arrowColors = gl.createBuffer();
  glState.frames = gl.createBuffer();
  glState.frameColors = gl.createBuffer();
  for (let i = 0; i < 2; i++) {
    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    glState.textures[i] = tex;
  }
  gl.enable(gl.DEPTH_TEST);
  gl.clearColor(0.086, 0.078, 0.06, 1);
  bindCamera(canvas);
  return true;
}

function uploadPlateTexture(index, image) {
  const gl = glState.gl;
  if (!gl) return;
  gl.bindTexture(gl.TEXTURE_2D, glState.textures[index]);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
}

function uploadMesh() {
  const gl = glState.gl;
  const h = wasm.header;
  if (!gl || !h) return;
  const stride = h.vertStride;
  const verts = new Float32Array(wasm.memory.buffer, h.vertPtr, h.vertCount * stride);
  gl.bindBuffer(gl.ARRAY_BUFFER, glState.mesh);
  gl.bufferData(gl.ARRAY_BUFFER, verts, gl.DYNAMIC_DRAW);

  const indices = new Uint32Array(wasm.memory.buffer, h.indexPtr, h.indexCount);
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, glState.index);
  gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, indices, gl.DYNAMIC_DRAW);
  glState.indexCount = indices.length;

  const lines = new Uint32Array(indices.length * 2);
  for (let i = 0; i < indices.length; i += 3) {
    const o = i * 2;
    lines[o] = indices[i];
    lines[o + 1] = indices[i + 1];
    lines[o + 2] = indices[i + 1];
    lines[o + 3] = indices[i + 2];
    lines[o + 4] = indices[i + 2];
    lines[o + 5] = indices[i];
  }
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, glState.wire);
  gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, lines, gl.DYNAMIC_DRAW);
  glState.wireCount = lines.length;

  const arrows = new Float32Array(wasm.memory.buffer, h.arrowPtr, h.arrowCount * 6);
  const positions = new Float32Array(h.arrowCount * 6);
  const colors = new Float32Array(h.arrowCount * 6);
  positions.set(arrows);
  for (let i = 0; i < h.arrowCount; i++) {
    const up = arrows[i * 6 + 4] - arrows[i * 6 + 1];
    const color = up >= 0 ? [0.84, 0.95, 0.36] : [1.0, 0.36, 0.22];
    for (let v = 0; v < 2; v++) {
      colors.set(color, i * 6 + v * 3);
    }
  }
  gl.bindBuffer(gl.ARRAY_BUFFER, glState.arrows);
  gl.bufferData(gl.ARRAY_BUFFER, positions, gl.DYNAMIC_DRAW);
  gl.bindBuffer(gl.ARRAY_BUFFER, glState.arrowColors);
  gl.bufferData(gl.ARRAY_BUFFER, colors, gl.DYNAMIC_DRAW);
  glState.arrowCount = h.arrowCount * 2;
  uploadPlates();
}

function uploadPlates() {
  const gl = glState.gl;
  const h = wasm.header;
  if (!gl || !h) return;
  const s = h.span * 0.5;
  const py = h.plateY;
  // Both faces are this one plane. Winding faces +Y, so the back face is the downward plate.
  const data = new Float32Array([
    -s, py, -s, 0, 0,
    s, py, -s, 1, 0,
    -s, py, s, 0, 1,
    s, py, -s, 1, 0,
    s, py, s, 1, 1,
    -s, py, s, 0, 1,
  ]);
  gl.bindBuffer(gl.ARRAY_BUFFER, glState.plates);
  gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);

  const corners = [
    [-s, py, -s],
    [s, py, -s],
    [s, py, s],
    [-s, py, s],
  ];
  const frame = [];
  const frameColor = [];
  const color = [0.92, 0.9, 0.82];
  for (let i = 0; i < 4; i++) {
    frame.push(...corners[i], ...corners[(i + 1) % 4]);
    frameColor.push(...color, ...color);
  }
  gl.bindBuffer(gl.ARRAY_BUFFER, glState.frames);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(frame), gl.STATIC_DRAW);
  gl.bindBuffer(gl.ARRAY_BUFFER, glState.frameColors);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(frameColor), gl.STATIC_DRAW);
  glState.frameCount = 8;
}

function resize() {
  const canvas = $("view");
  const gl = glState.gl;
  if (!gl) return;
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const w = Math.max(2, Math.floor(canvas.clientWidth * dpr));
  const h = Math.max(2, Math.floor(canvas.clientHeight * dpr));
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
}

function perspective(fov, aspect, near, far) {
  const f = 1 / Math.tan(fov / 2);
  const nf = 1 / (near - far);
  return new Float32Array([
    f / aspect, 0, 0, 0,
    0, f, 0, 0,
    0, 0, (far + near) * nf, -1,
    0, 0, 2 * far * near * nf, 0,
  ]);
}

function lookAt(eye, center, up) {
  const z = normalize(sub(eye, center));
  const x = normalize(cross(up, z));
  const y = cross(z, x);
  return new Float32Array([
    x[0], y[0], z[0], 0,
    x[1], y[1], z[1], 0,
    x[2], y[2], z[2], 0,
    -dot(x, eye), -dot(y, eye), -dot(z, eye), 1,
  ]);
}

function mul(a, b) {
  const out = new Float32Array(16);
  for (let c = 0; c < 4; c++) {
    for (let r = 0; r < 4; r++) {
      out[c * 4 + r] =
        a[r] * b[c * 4] +
        a[4 + r] * b[c * 4 + 1] +
        a[8 + r] * b[c * 4 + 2] +
        a[12 + r] * b[c * 4 + 3];
    }
  }
  return out;
}

function sub(a, b) { return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]; }
function dot(a, b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }
function cross(a, b) {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
}
function normalize(v) {
  const len = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / len, v[1] / len, v[2] / len];
}

function eyePosition() {
  const s = Math.sin(state.phi);
  return [
    state.radius * s * Math.sin(state.theta),
    state.radius * Math.cos(state.phi),
    state.radius * s * Math.cos(state.theta),
  ];
}

function draw() {
  const gl = glState.gl;
  const h = wasm.header;
  if (!gl || !h || !glState.indexCount) return;
  resize();
  gl.viewport(0, 0, gl.canvas.width, gl.canvas.height);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
  const aspect = gl.canvas.width / gl.canvas.height;
  const eye = eyePosition();
  const view = lookAt(eye, [0, 0, 0], [0, 1, 0]);
  const proj = perspective((38 * Math.PI) / 180, aspect, 0.05, 40);
  const mvp = mul(proj, view);

  const diagram = state.plates || state.vectors;
  if (state.solid && !diagram) {
    gl.disable(gl.BLEND);
    gl.depthMask(true);
    drawSolid(gl, mvp, eye, gl.BACK, 1);
  } else if (state.solid) {
    // Far side of the shell first, so the shared plate reads as sitting inside it.
    gl.disable(gl.BLEND);
    gl.depthMask(true);
    drawSolid(gl, mvp, eye, gl.FRONT, 1);
  }

  if (state.plates) {
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.depthMask(false);
    gl.disable(gl.CULL_FACE);
    gl.useProgram(glState.plateProgram);
    gl.uniformMatrix4fv(gl.getUniformLocation(glState.plateProgram, "uMvp"), false, mvp);
    gl.uniform1i(gl.getUniformLocation(glState.plateProgram, "uTexPlus"), 0);
    gl.uniform1i(gl.getUniformLocation(glState.plateProgram, "uTexMinus"), 1);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, glState.textures[0]);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, glState.textures[1]);
    gl.bindBuffer(gl.ARRAY_BUFFER, glState.plates);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 20, 0);
    gl.enableVertexAttribArray(1);
    gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 20, 12);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    gl.disableVertexAttribArray(1);
    gl.depthMask(true);
    drawLines(gl, mvp, glState.frames, glState.frameColors, glState.frameCount || 0);
  }

  if (state.vectors && glState.arrowCount) {
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.depthMask(true);
    drawLines(gl, mvp, glState.arrows, glState.arrowColors, glState.arrowCount);
  }

  if (state.solid && diagram) {
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.depthMask(false);
    drawSolid(gl, mvp, eye, gl.BACK, 0.4);
  }

  if (state.wire && glState.wireCount) {
    gl.disable(gl.CULL_FACE);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.depthMask(false);
    gl.useProgram(glState.wireProgram);
    gl.uniformMatrix4fv(gl.getUniformLocation(glState.wireProgram, "uMvp"), false, mvp);
    gl.bindBuffer(gl.ARRAY_BUFFER, glState.mesh);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, h.vertStride * 4, 0);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, glState.wire);
    gl.drawElements(gl.LINES, glState.wireCount, gl.UNSIGNED_INT, 0);
  }
  gl.depthMask(true);
  gl.disable(gl.BLEND);
}

function drawSolid(gl, mvp, eye, cull, alpha) {
  gl.enable(gl.CULL_FACE);
  gl.cullFace(cull);
  gl.useProgram(glState.program);
  gl.uniformMatrix4fv(gl.getUniformLocation(glState.program, "uMvp"), false, mvp);
  gl.uniform3f(gl.getUniformLocation(glState.program, "uEye"), eye[0], eye[1], eye[2]);
  gl.uniform1f(gl.getUniformLocation(glState.program, "uAlpha"), alpha);
  gl.bindBuffer(gl.ARRAY_BUFFER, glState.mesh);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 3, gl.FLOAT, false, wasm.header.vertStride * 4, 0);
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, glState.index);
  gl.drawElements(gl.TRIANGLES, glState.indexCount, gl.UNSIGNED_INT, 0);
}

function drawLines(gl, mvp, posBuf, colorBuf, count) {
  if (!count) return;
  gl.useProgram(glState.lineProgram);
  gl.uniformMatrix4fv(gl.getUniformLocation(glState.lineProgram, "uMvp"), false, mvp);
  gl.bindBuffer(gl.ARRAY_BUFFER, posBuf);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
  gl.bindBuffer(gl.ARRAY_BUFFER, colorBuf);
  gl.enableVertexAttribArray(1);
  gl.vertexAttribPointer(1, 3, gl.FLOAT, false, 0, 0);
  gl.drawArrays(gl.LINES, 0, count);
  gl.disableVertexAttribArray(1);
}

function bindCamera(canvas) {
  canvas.addEventListener("pointerdown", (event) => {
    state.dragging = true;
    canvas.setPointerCapture(event.pointerId);
  });
  canvas.addEventListener("pointerup", () => {
    state.dragging = false;
  });
  canvas.addEventListener("pointermove", (event) => {
    if (!state.dragging) return;
    state.theta -= event.movementX * 0.008;
    state.phi -= event.movementY * 0.008;
    state.phi = Math.max(0.18, Math.min(Math.PI - 0.18, state.phi));
  });
  canvas.addEventListener("wheel", (event) => {
    event.preventDefault();
    state.radius *= Math.exp(event.deltaY * 0.001);
    state.radius = Math.min(8, Math.max(1.7, state.radius));
  }, { passive: false });
  canvas.addEventListener("dblclick", () => {
    state.theta = 0.62;
    state.phi = 1.02;
    state.radius = 3.55;
  });
  window.addEventListener("resize", resize);
}

let last = performance.now();
function frame(now) {
  const dt = Math.min(0.05, (now - last) / 1000);
  last = now;
  if (state.spin && !state.dragging) state.theta += dt * 0.18;
  draw();
  requestAnimationFrame(frame);
}

async function runSelfTest() {
  const notes = [];
  const fail = (message) => {
    notes.push(message);
  };
  state.shape = 0;
  state.lean = 0.3;
  state.bake = 64;
  state.mesh = 24;
  markShapes();
  await rebake();
  const sphere = wasm.header;
  if (sphere.status !== 0) fail(`status ${sphere.status}`);
  if (wasm.pngPlus[0] !== 0x89 || wasm.pngPlus[1] !== 0x50) fail("png magic");
  if (sphere.centerPlus[1] <= 0 || sphere.centerMinus[1] >= 0) fail("center vectors do not project outward");
  if (sphere.opposing < 0.98) fail(`opposing ${sphere.opposing}`);
  if (sphere.quantError > 0.001) fail(`quant ${sphere.quantError}`);
  const coarse = sphere.triCount;
  const pngLen = wasm.pngPlus.length;
  state.mesh = 64;
  await rebuildMesh();
  if (wasm.header.triCount <= coarse) fail("finer mesh did not add triangles");
  if (wasm.pngPlus.length !== pngLen) fail("png changed when the mesh changed");
  if (wasm.header.surfaceError > 0.02) fail(`surface ${wasm.header.surfaceError}`);
  state.shape = 2;
  state.mesh = 28;
  await rebake();
  if (wasm.header.centerPlusOk !== 0) fail("torus center should miss");
  state.shape = 5;
  state.lean = 0;
  state.bake = 64;
  await rebake();
  const stemY = wasm.header.plateY + wasm.header.centerMinus[1];
  if (stemY > -0.4) fail(`mushroom stem ${stemY}`);
  state.container = 1;
  state.kind = "formula";
  state.shape = 0;
  state.bake = 32;
  state.mesh = 24;
  markShapes();
  await rebake();
  if (wasm.pngPlus[0] !== 0x76 || wasm.pngPlus[1] !== 0x2f || wasm.pngPlus[2] !== 0x31) fail("exr magic");
  if (wasm.header.centerPlus[1] <= 0) fail("exr sphere");
  state.kind = "brush";
  state.brush = 0;
  state.bake = 32;
  markShapes();
  await rebake();
  if (wasm.header.validMinus !== 0) fail("brush should have an empty downward plate");
  if (wasm.header.centerPlus[1] < 0.8) fail(`horn ${wasm.header.centerPlus[1]}`);
  if (wasm.header.triCount < 4) fail("horn mesh");
  state.container = 0;
  state.kind = "formula";
  if (!glState.gl) fail("webgl missing");
  const result = notes.length ? `fail ${notes.join("; ")}` : "pass";
  document.documentElement.dataset.twin = result;
  $("selftest").textContent = result;
  state.shape = 0;
  state.kind = "formula";
  state.container = 0;
  state.lean = 0.35;
  state.bake = 128;
  state.mesh = 32;
  markShapes();
  $("lean").value = "0.35";
  $("mesh").value = "32";
  $("bake").value = String(BAKE_STOPS.indexOf(128));
  await rebake();
}

function readQuery() {
  const params = new URLSearchParams(location.search);
  const shape = Number(params.get("shape"));
  if (params.has("shape") && shape >= 0 && shape < SHAPES.length) state.shape = shape;
  const lean = Number(params.get("lean"));
  if (params.has("lean") && Number.isFinite(lean)) state.lean = Math.min(1.2, Math.max(0, lean));
  const mesh = Number(params.get("mesh"));
  if (params.has("mesh") && mesh >= 4 && mesh <= 256) state.mesh = Math.round(mesh);
  const bake = Number(params.get("bake"));
  if (BAKE_STOPS.includes(bake)) state.bake = bake;
  if (params.get("spin") === "0") {
    state.spin = false;
    $("spin").checked = false;
  }
}

async function boot() {
  try {
    await loadWasm();
    if (!new URLSearchParams(location.search).has("selftest")) readQuery();
    bindUI();
    const glOk = initGl();
    if (new URLSearchParams(location.search).has("selftest")) {
      await runSelfTest();
    } else {
      await rebake();
    }
    if (glOk) requestAnimationFrame(frame);
  } catch (error) {
    showFatal(error);
    const node = $("selftest");
    node.hidden = false;
    node.textContent = `fail ${error.message || error}`;
    document.documentElement.dataset.twin = node.textContent;
  }
}

boot();
