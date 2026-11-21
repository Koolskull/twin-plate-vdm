import { readFileSync, writeFileSync } from "node:fs";

const bytes = readFileSync(new URL("./web/twin_plate_vdm.wasm", import.meta.url));
const { instance } = await WebAssembly.instantiate(bytes, {});
const exp = instance.exports;

function header() {
  const view = new DataView(exp.memory.buffer, exp.header_ptr(), 136);
  const u = (offset) => view.getUint32(offset, true);
  const f = (offset) => view.getFloat32(offset, true);
  return {
    status: u(0),
    bakeRes: u(8),
    pngPlusPtr: u(16),
    pngPlusLen: u(20),
    pngMinusPtr: u(24),
    pngMinusLen: u(28),
    triCount: u(72),
    quantError: f(88),
    opposing: f(96),
    centerPlusY: f(116),
    centerMinusY: f(128),
    centerPlusOk: u(80),
  };
}

function png(ptr, len) {
  return new Uint8Array(exp.memory.buffer).slice(ptr, ptr + len);
}

if (exp.bake(0, 32, 0.25) !== 0) throw new Error("bake failed");
let h = header();
const plus = png(h.pngPlusPtr, h.pngPlusLen);
const minus = png(h.pngMinusPtr, h.pngMinusLen);
if (plus[0] !== 0x89 || plus[1] !== 0x50 || plus[2] !== 0x4e || plus[3] !== 0x47) {
  throw new Error("plus png magic");
}
if (h.centerPlusY <= 0 || h.centerMinusY >= 0) throw new Error("vectors do not project outward");
if (h.opposing < 0.98) throw new Error(`opposing ${h.opposing}`);
if (h.quantError > 1e-3) throw new Error(`quant ${h.quantError}`);

exp.wipe();
const afterWipe = header();
new Uint8Array(exp.memory.buffer).set(plus, afterWipe.pngPlusPtr);
new Uint8Array(exp.memory.buffer).set(minus, afterWipe.pngMinusPtr);
if (exp.load_pngs(afterWipe.pngPlusPtr, plus.length, afterWipe.pngMinusPtr, minus.length) !== 0) {
  throw new Error("load_pngs failed");
}
if (exp.reconstruct(16) !== 0) throw new Error("reconstruct 16");
const coarse = header().triCount;
if (exp.reconstruct(32) !== 0) throw new Error("reconstruct 32");
const fine = header().triCount;
if (!(fine > coarse && coarse > 0)) throw new Error(`tris ${coarse} -> ${fine}`);

if (exp.bake(2, 32, 0) !== 0) throw new Error("torus bake");
if (header().centerPlusOk !== 0) throw new Error("torus hole missing");

writeFileSync("/tmp/twin-plate-plus.png", plus);
writeFileSync("/tmp/twin-plate-minus.png", minus);
console.log(`wasm ok  sphere tris ${coarse} -> ${fine}  png ${plus.length}+${minus.length} bytes`);
