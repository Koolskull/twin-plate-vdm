//! Scanline OpenEXR, enough for vector displacement brushes.
//!
//! Writes one uncompressed float RGBA part: R, G, B are the X, Y, Z vector and
//! A is the mask. Reads that back, and also reads the files other tools write:
//! HALF or FLOAT, RGB or RGBA, extra channels skipped, NONE, RLE, ZIP, and
//! ZIPS. ZIP blocks may be raw deflate or a zlib wrapper. Tiled, deep, and
//! multipart files are refused.

#[cfg(target_arch = "wasm32")]
use crate::fmath::FloatMath;
use miniz_oxide::inflate::core::decompress;
use miniz_oxide::inflate::core::inflate_flags::{
    TINFL_FLAG_PARSE_ZLIB_HEADER, TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
};
use miniz_oxide::inflate::core::DecompressorOxide;
use miniz_oxide::inflate::TINFLStatus;

const MAGIC: [u8; 4] = [0x76, 0x2f, 0x31, 0x01];
const MAX_CHANNELS: usize = 8;
const MAX_RES: u32 = 256;

struct W<'a> {
    buf: &'a mut [u8],
    n: usize,
    ok: bool,
}

impl<'a> W<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, n: 0, ok: true }
    }

    fn bytes(&mut self, data: &[u8]) {
        if self.n + data.len() > self.buf.len() {
            self.ok = false;
            return;
        }
        self.buf[self.n..self.n + data.len()].copy_from_slice(data);
        self.n += data.len();
    }

    fn u8(&mut self, v: u8) {
        self.bytes(&[v]);
    }

    fn cstr(&mut self, s: &str) {
        self.bytes(s.as_bytes());
        self.u8(0);
    }

    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }

    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }

    fn patch_u32(&mut self, at: usize, v: u32) {
        if at + 4 > self.n {
            self.ok = false;
            return;
        }
        self.buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn patch_u64(&mut self, at: usize, v: u64) {
        if at + 8 > self.n {
            self.ok = false;
            return;
        }
        self.buf[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
}

#[derive(Clone, Copy)]
struct Chan {
    kind: u8, // b'R', b'G', b'B', b'A', or 0 for something we skip
    pixel: i32,
    bytes: usize,
}

struct Info {
    xmin: i32,
    ymin: i32,
    width: usize,
    height: usize,
    compression: u8,
    channels: [Chan; MAX_CHANNELS],
    nchan: usize,
    bps: usize,
    lines: usize,
}

pub fn is_exr(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && bytes[..4] == MAGIC
}

fn half_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let mut exp = ((h >> 10) & 0x1f) as i32;
    let mut mant = (h & 0x3ff) as u32;
    if exp == 0 {
        if mant == 0 {
            return f32::from_bits(sign << 31);
        }
        while mant & 0x400 == 0 {
            mant <<= 1;
            exp -= 1;
        }
        mant &= 0x3ff;
        exp += 1;
    } else if exp == 31 {
        let bits = (sign << 31) | (0xff << 23) | (mant << 13);
        return f32::from_bits(bits);
    }
    let biased = exp + (127 - 15);
    let bits = (sign << 31) | ((biased as u32) << 23) | (mant << 13);
    f32::from_bits(bits)
}

fn read_f32(pixel: i32, bytes: &[u8]) -> f32 {
    if pixel == 1 {
        if bytes.len() < 2 {
            return f32::NAN;
        }
        half_to_f32(u16::from_le_bytes([bytes[0], bytes[1]]))
    } else {
        if bytes.len() < 4 {
            return f32::NAN;
        }
        f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
}

fn quantize(c: f32, range: f32) -> u16 {
    let t = (c / (2.0 * range) + 0.5).clamp(0.0, 1.0);
    let q = (t * 65535.0).round();
    if q <= 0.0 {
        0
    } else if q >= 65535.0 {
        65535
    } else {
        q as u16
    }
}

fn dequant(q: u16, range: f32) -> f32 {
    (q as f32 / 65535.0 - 0.5) * (2.0 * range)
}

fn inflater() -> &'static mut DecompressorOxide {
    use core::cell::UnsafeCell;
    struct Slot(UnsafeCell<Option<DecompressorOxide>>);
    unsafe impl Sync for Slot {}
    static DEC: Slot = Slot(UnsafeCell::new(None));
    unsafe {
        let slot = &mut *DEC.0.get();
        if slot.is_none() {
            *slot = Some(DecompressorOxide::new());
        }
        slot.as_mut().unwrap()
    }
}

fn inflate(src: &[u8], dst: &mut [u8]) -> Result<usize, ()> {
    let flags = TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    for zlib in [false, true] {
        let dec = inflater();
        dec.init();
        let flags = if zlib {
            flags | TINFL_FLAG_PARSE_ZLIB_HEADER
        } else {
            flags
        };
        let (status, _, n) = decompress(dec, src, dst, 0, flags);
        if status == TINFLStatus::Done && n > 0 {
            return Ok(n);
        }
    }
    Err(())
}

fn rle_decode(src: &[u8], dst: &mut [u8]) -> Result<usize, ()> {
    let mut i = 0usize;
    let mut o = 0usize;
    while i < src.len() {
        let n = src[i] as i8;
        i += 1;
        if n < 0 {
            let count = (-(n as i32)) as usize;
            if i + count > src.len() || o + count > dst.len() {
                return Err(());
            }
            dst[o..o + count].copy_from_slice(&src[i..i + count]);
            i += count;
            o += count;
        } else {
            let count = n as usize + 1;
            if i >= src.len() || o + count > dst.len() {
                return Err(());
            }
            let v = src[i];
            i += 1;
            for slot in &mut dst[o..o + count] {
                *slot = v;
            }
            o += count;
        }
    }
    Ok(o)
}

/// Predictor, then the even/odd interleave. Matches OpenEXRCore zip and RLE.
fn reorder(packed: &mut [u8], out: &mut [u8]) {
    let n = packed.len();
    let mut t = 1usize;
    while t < n {
        let d = packed[t - 1].wrapping_add(packed[t]).wrapping_sub(128);
        packed[t] = d;
        t += 1;
    }
    let mut t1 = 0usize;
    let mut t2 = (n + 1) / 2;
    let mut s = 0usize;
    while s < n {
        out[s] = packed[t1];
        s += 1;
        t1 += 1;
        if s < n {
            out[s] = packed[t2];
            s += 1;
            t2 += 1;
        }
    }
}

fn u32_at(b: &[u8], i: usize) -> Result<u32, ()> {
    let s = b.get(i..i + 4).ok_or(())?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn i32_at(b: &[u8], i: usize) -> Result<i32, ()> {
    Ok(u32_at(b, i)? as i32)
}

fn cstr_at(b: &[u8], i: usize) -> Result<(&[u8], usize), ()> {
    let rest = b.get(i..).ok_or(())?;
    let end = rest.iter().position(|c| *c == 0).ok_or(())?;
    Ok((&rest[..end], i + end + 1))
}

fn parse(file: &[u8]) -> Result<(Info, usize), ()> {
    if !is_exr(file) {
        return Err(());
    }
    let ver = u32_at(file, 4)?;
    if ver & 0xff != 2 {
        return Err(());
    }
    let flags = ver & !0xff;
    // Tiled, deep, or multipart. Long names (0x400) are still a scanline part.
    if flags & (0x200 | 0x800 | 0x1000) != 0 {
        return Err(());
    }
    let mut i = 8usize;
    let mut xmin = 0i32;
    let mut ymin = 0i32;
    let mut xmax = -1i32;
    let mut ymax = -1i32;
    let mut compression = 0u8;
    let mut saw_window = false;
    let mut channels = [Chan {
        kind: 0,
        pixel: 0,
        bytes: 0,
    }; MAX_CHANNELS];
    let mut nchan = 0usize;
    while i < file.len() {
        if file[i] == 0 {
            i += 1;
            break;
        }
        let (_name, next) = cstr_at(file, i)?;
        let name = &file[i..next - 1];
        i = next;
        let (typ, next) = cstr_at(file, i)?;
        i = next;
        let size = u32_at(file, i)? as usize;
        i += 4;
        let val = file.get(i..i + size).ok_or(())?;
        i += size;
        if name == b"channels" && typ == b"chlist" {
            let mut p = 0usize;
            while p < val.len() && val[p] != 0 {
                let (cn, np) = cstr_at(val, p)?;
                p = np;
                if p + 16 > val.len() || nchan >= MAX_CHANNELS {
                    return Err(());
                }
                let pixel = i32::from_le_bytes([val[p], val[p + 1], val[p + 2], val[p + 3]]);
                p += 4;
                p += 4; // pLinear + reserved
                let xs = i32::from_le_bytes([val[p], val[p + 1], val[p + 2], val[p + 3]]);
                let ys = i32::from_le_bytes([val[p + 4], val[p + 5], val[p + 6], val[p + 7]]);
                p += 8;
                if xs != 1 || ys != 1 || !(pixel == 1 || pixel == 2) {
                    return Err(());
                }
                let kind = if cn == b"R" {
                    b'R'
                } else if cn == b"G" {
                    b'G'
                } else if cn == b"B" {
                    b'B'
                } else if cn == b"A" {
                    b'A'
                } else {
                    0
                };
                channels[nchan] = Chan {
                    kind,
                    pixel,
                    bytes: if pixel == 1 { 2 } else { 4 },
                };
                nchan += 1;
            }
        } else if name == b"compression" && size >= 1 {
            compression = val[0];
        } else if name == b"dataWindow" && size >= 16 {
            xmin = i32::from_le_bytes([val[0], val[1], val[2], val[3]]);
            ymin = i32::from_le_bytes([val[4], val[5], val[6], val[7]]);
            xmax = i32::from_le_bytes([val[8], val[9], val[10], val[11]]);
            ymax = i32::from_le_bytes([val[12], val[13], val[14], val[15]]);
            saw_window = true;
        }
    }
    if !saw_window || nchan == 0 || xmax < xmin || ymax < ymin {
        return Err(());
    }
    if !(compression == 0 || compression == 1 || compression == 2 || compression == 3) {
        return Err(());
    }
    let width = (xmax - xmin + 1) as usize;
    let height = (ymax - ymin + 1) as usize;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(());
    }
    let mut bps = 0usize;
    let mut saw_rgb = [false; 3];
    for c in &channels[..nchan] {
        bps = bps.saturating_add(width.saturating_mul(c.bytes));
        if c.kind == b'R' {
            saw_rgb[0] = true;
        }
        if c.kind == b'G' {
            saw_rgb[1] = true;
        }
        if c.kind == b'B' {
            saw_rgb[2] = true;
        }
    }
    if !saw_rgb[0] || !saw_rgb[1] || !saw_rgb[2] || bps == 0 {
        return Err(());
    }
    let lines = if compression == 3 { 16 } else { 1 };
    Ok((
        Info {
            xmin,
            ymin,
            width,
            height,
            compression,
            channels,
            nchan,
            bps,
            lines,
        },
        i,
    ))
}

fn decode_chunk(file: &[u8], off: usize, info: &Info, scratch: &mut [u8]) -> Result<(i32, usize), ()> {
    if off + 8 > file.len() {
        return Err(());
    }
    let y = i32_at(file, off)?;
    let size = i32_at(file, off + 4)?;
    if size < 0 {
        return Err(());
    }
    let size = size as usize;
    let src = file.get(off + 8..off + 8 + size).ok_or(())?;
    let nlines = (info.ymin + info.height as i32 - y).clamp(1, info.lines as i32) as usize;
    let raw_len = nlines * info.bps;
    if scratch.len() < raw_len * 2 {
        return Err(());
    }
    if info.compression == 0 {
        if src.len() < raw_len {
            return Err(());
        }
        scratch[..raw_len].copy_from_slice(&src[..raw_len]);
    } else if info.compression == 1 {
        let n = rle_decode(src, &mut scratch[..raw_len])?;
        if n != raw_len {
            return Err(());
        }
        {
            let (packed, rest) = scratch.split_at_mut(raw_len);
            reorder(packed, &mut rest[..raw_len]);
        }
        scratch.copy_within(raw_len..raw_len * 2, 0);
    } else {
        let n = inflate(src, &mut scratch[..raw_len])?;
        if n != raw_len {
            return Err(());
        }
        {
            let (packed, rest) = scratch.split_at_mut(raw_len);
            reorder(packed, &mut rest[..raw_len]);
        }
        scratch.copy_within(raw_len..raw_len * 2, 0);
    }
    Ok((y, nlines))
}

fn pixel_at(scratch: &[u8], info: &Info, line: usize, x: usize) -> [f32; 4] {
    let mut out = [0.0, 0.0, 0.0, 1.0];
    let mut has_a = false;
    let base = line * info.bps;
    let mut o = base;
    for c in &info.channels[..info.nchan] {
        if c.kind != 0 && x < info.width {
            let at = o + x * c.bytes;
            let v = read_f32(c.pixel, &scratch[at..]);
            match c.kind {
                b'R' => out[0] = v,
                b'G' => out[1] = v,
                b'B' => out[2] = v,
                b'A' => {
                    out[3] = v;
                    has_a = true;
                }
                _ => {}
            }
        }
        o += info.width * c.bytes;
    }
    if !has_a {
        out[3] = 1.0;
    }
    out
}

fn store_texel(pixels: &mut [u16], out_res: usize, dx: usize, dy: usize, sample: [f32; 4], range: f32) {
    let o = (dy * out_res + dx) * 4;
    if o + 3 >= pixels.len() {
        return;
    }
    let finite = sample[0].is_finite() && sample[1].is_finite() && sample[2].is_finite();
    if finite {
        pixels[o] = quantize(sample[0], range);
        pixels[o + 1] = quantize(sample[1], range);
        pixels[o + 2] = quantize(sample[2], range);
    } else {
        let zero = quantize(0.0, range);
        pixels[o] = zero;
        pixels[o + 1] = zero;
        pixels[o + 2] = zero;
    }
    pixels[o + 3] = if finite && sample[3] > 0.5 { 65535 } else { 0 };
}

/// Decode into a square of at most 256. A larger or non-square file is center-cropped.
pub fn decode_rgba16(file: &[u8], pixels: &mut [u16], scratch: &mut [u8], range: f32) -> Result<u32, ()> {
    let (info, table) = parse(file)?;
    let side = info.width.min(info.height);
    if side == 0 {
        return Err(());
    }
    let out_res = side.min(MAX_RES as usize);
    if pixels.len() < out_res * out_res * 4 {
        return Err(());
    }
    let zero = quantize(0.0, range);
    for px in pixels.iter_mut().take(out_res * out_res * 4) {
        *px = zero;
    }
    for i in (3..out_res * out_res * 4).step_by(4) {
        pixels[i] = 0;
    }
    let blocks = (info.height + info.lines - 1) / info.lines;
    let ox0 = (info.width - side) / 2;
    let oy0 = (info.height - side) / 2;
    for b in 0..blocks {
        let at = table + b * 8;
        let off = u64::from_le_bytes(file.get(at..at + 8).ok_or(())?.try_into().unwrap()) as usize;
        if off == 0 {
            return Err(());
        }
        let (y, nlines) = decode_chunk(file, off, &info, scratch)?;
        let y0 = (y - info.ymin) as usize;
        for line in 0..nlines {
            let src_y = y0 + line;
            if src_y < oy0 || src_y >= oy0 + side {
                continue;
            }
            let local_y = src_y - oy0;
            let dy = if side == out_res {
                local_y as isize
            } else {
                (((local_y as f32) + 0.5) * out_res as f32 / side as f32 - 0.5).round() as isize
            };
            if dy < 0 || dy >= out_res as isize {
                continue;
            }
            for local_x in 0..side {
                let src_x = ox0 + local_x;
                let dx = if side == out_res {
                    local_x as isize
                } else {
                    (((local_x as f32) + 0.5) * out_res as f32 / side as f32 - 0.5).round() as isize
                };
                if dx < 0 || dx >= out_res as isize {
                    continue;
                }
                let sample = pixel_at(scratch, &info, line, src_x);
                store_texel(pixels, out_res, dx as usize, dy as usize, sample, range);
            }
        }
    }
    Ok(out_res as u32)
}

pub fn sample_rgba(file: &[u8], scratch: &mut [u8], x: i32, y: i32) -> Result<[f32; 4], ()> {
    let (info, table) = parse(file)?;
    if x < info.xmin || y < info.ymin || x >= info.xmin + info.width as i32 || y >= info.ymin + info.height as i32
    {
        return Err(());
    }
    let local_y = (y - info.ymin) as usize;
    let block = local_y / info.lines;
    let at = table + block * 8;
    let off = u64::from_le_bytes(file.get(at..at + 8).ok_or(())?.try_into().unwrap()) as usize;
    let (cy, _nlines) = decode_chunk(file, off, &info, scratch)?;
    let line = (y - cy) as usize;
    Ok(pixel_at(scratch, &info, line, (x - info.xmin) as usize))
}

fn write_chlist(buf: &mut [u8]) -> Result<usize, ()> {
    let mut n = 0usize;
    for name in [b"A".as_slice(), b"B", b"G", b"R"] {
        if n + name.len() + 1 + 16 > buf.len() {
            return Err(());
        }
        buf[n..n + name.len()].copy_from_slice(name);
        n += name.len();
        buf[n] = 0;
        n += 1;
        buf[n..n + 4].copy_from_slice(&2i32.to_le_bytes()); // FLOAT
        n += 4;
        buf[n] = 0;
        buf[n + 1] = 0;
        buf[n + 2] = 0;
        buf[n + 3] = 0;
        n += 4;
        buf[n..n + 4].copy_from_slice(&1i32.to_le_bytes());
        buf[n + 4..n + 8].copy_from_slice(&1i32.to_le_bytes());
        n += 8;
    }
    if n >= buf.len() {
        return Err(());
    }
    buf[n] = 0;
    Ok(n + 1)
}

fn encode_rows<F>(res: u32, comment: &str, scratch: &mut [u8], out: &mut [u8], mut fill: F) -> Result<usize, ()>
where
    F: FnMut(u32, &mut [f32]),
{
    if res == 0 || res > MAX_RES {
        return Err(());
    }
    let mut row = [0f32; 1024];
    let planar = (res as usize) * 16;
    if scratch.len() < planar {
        return Err(());
    }
    let mut chlist = [0u8; 96];
    let ch_len = write_chlist(&mut chlist)?;

    let mut w = W::new(out);
    w.bytes(&MAGIC);
    w.u32(2);
    w.cstr("channels");
    w.cstr("chlist");
    w.u32(ch_len as u32);
    w.bytes(&chlist[..ch_len]);
    w.cstr("compression");
    w.cstr("compression");
    w.u32(1);
    w.u8(0);
    if !comment.is_empty() && comment.len() < 400 {
        w.cstr("comment");
        w.cstr("string");
        w.u32(4 + comment.len() as u32);
        w.u32(comment.len() as u32);
        w.bytes(comment.as_bytes());
    }
    w.cstr("dataWindow");
    w.cstr("box2i");
    w.u32(16);
    w.i32(0);
    w.i32(0);
    w.i32(res as i32 - 1);
    w.i32(res as i32 - 1);
    w.cstr("displayWindow");
    w.cstr("box2i");
    w.u32(16);
    w.i32(0);
    w.i32(0);
    w.i32(res as i32 - 1);
    w.i32(res as i32 - 1);
    w.cstr("lineOrder");
    w.cstr("lineOrder");
    w.u32(1);
    w.u8(0);
    w.cstr("pixelAspectRatio");
    w.cstr("float");
    w.u32(4);
    w.f32(1.0);
    w.cstr("screenWindowCenter");
    w.cstr("v2f");
    w.u32(8);
    w.f32(0.0);
    w.f32(0.0);
    w.cstr("screenWindowWidth");
    w.cstr("float");
    w.u32(4);
    w.f32(1.0);
    w.u8(0);
    let table = w.n;
    for _ in 0..res {
        w.u64(0);
    }
    let res_us = res as usize;
    for y in 0..res {
        if !w.ok {
            return Err(());
        }
        w.patch_u64(table + y as usize * 8, w.n as u64);
        w.i32(y as i32);
        let size_at = w.n;
        w.i32(0);
        let data_at = w.n;
        fill(y, &mut row[..res_us * 4]);
        // Channel order A, B, G, R. B is Z, G is Y, R is X.
        let plane = res_us * 4;
        for x in 0..res_us {
            let a = row[x * 4 + 3];
            scratch[x * 4..x * 4 + 4].copy_from_slice(&a.to_le_bytes());
        }
        for x in 0..res_us {
            let b = row[x * 4 + 2];
            let o = plane + x * 4;
            scratch[o..o + 4].copy_from_slice(&b.to_le_bytes());
        }
        for x in 0..res_us {
            let g = row[x * 4 + 1];
            let o = plane * 2 + x * 4;
            scratch[o..o + 4].copy_from_slice(&g.to_le_bytes());
        }
        for x in 0..res_us {
            let r = row[x * 4];
            let o = plane * 3 + x * 4;
            scratch[o..o + 4].copy_from_slice(&r.to_le_bytes());
        }
        w.bytes(&scratch[..planar]);
        let data_len = w.n - data_at;
        w.patch_u32(size_at, data_len as u32);
    }
    if !w.ok {
        return Err(());
    }
    Ok(w.n)
}

pub fn encode_rgba16(
    pixels: &[u16],
    res: u32,
    comment: &str,
    scratch: &mut [u8],
    out: &mut [u8],
    range: f32,
) -> Result<usize, ()> {
    let res_us = res as usize;
    if pixels.len() < res_us * res_us * 4 {
        return Err(());
    }
    encode_rows(res, comment, scratch, out, |y, row| {
        for x in 0..res_us {
            let s = ((y as usize) * res_us + x) * 4;
            let a = pixels[s + 3];
            row[x * 4] = dequant(pixels[s], range);
            row[x * 4 + 1] = dequant(pixels[s + 1], range);
            row[x * 4 + 2] = dequant(pixels[s + 2], range);
            row[x * 4 + 3] = if a >= 32768 { 1.0 } else { 0.0 };
        }
    })
}

#[allow(dead_code)]
pub fn encode_rgba_f32(
    pixels: &[f32],
    res: u32,
    comment: &str,
    scratch: &mut [u8],
    out: &mut [u8],
) -> Result<usize, ()> {
    let res_us = res as usize;
    if pixels.len() < res_us * res_us * 4 {
        return Err(());
    }
    encode_rows(res, comment, scratch, out, |y, row| {
        let s = (y as usize) * res_us * 4;
        row[..res_us * 4].copy_from_slice(&pixels[s..s + res_us * 4]);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_one_is_one() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0x0000), 0.0);
        assert!((half_to_f32(0x3800) - 0.5).abs() < 1e-6);
        let tiny = half_to_f32(0x0001);
        assert!(tiny > 0.0 && tiny < 1e-6, "{tiny}");
    }

    #[test]
    fn uncompressed_roundtrip_keeps_the_codes() {
        let res = 8u32;
        let mut pixels = vec![0u16; (res * res * 4) as usize];
        for (i, px) in pixels.iter_mut().enumerate() {
            let v = ((i as u32) * 997) % 65536;
            *px = v as u16;
        }
        for i in 0..(res * res) as usize {
            pixels[i * 4 + 3] = if i % 5 == 0 { 0 } else { 65535 };
        }
        let mut scratch = vec![0u8; 64 * 1024];
        let mut out = vec![0u8; 64 * 1024];
        let n = encode_rgba16(&pixels, res, "twin-plate-vdm plate=plus shape=0", &mut scratch, &mut out, 2.5)
            .unwrap();
        assert_eq!(&out[..4], &MAGIC);
        assert!(out[..n].windows(6).any(|w| w == b"shape="));
        let mut back = vec![9u16; pixels.len()];
        let got = decode_rgba16(&out[..n], &mut back, &mut scratch, 2.5).unwrap();
        assert_eq!(got, res);
        assert_eq!(back, pixels);
    }

    fn deconstruct(source: &[u8]) -> Vec<u8> {
        let n = source.len();
        let mut scratch = vec![0u8; n];
        let mut t1 = 0usize;
        let mut t2 = (n + 1) / 2;
        let mut raw = source.iter().copied();
        while t1 < (n + 1) / 2 {
            scratch[t1] = raw.next().unwrap();
            t1 += 1;
            if t2 < n {
                scratch[t2] = raw.next().unwrap();
                t2 += 1;
            }
        }
        let mut p = scratch[0] as i32;
        for i in 1..n {
            let cur = scratch[i] as i32;
            let d = cur - p + (128 + 256);
            p = cur;
            scratch[i] = d as u8;
        }
        scratch
    }

    fn stored_deflate(data: &[u8], zlib: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if zlib {
            out.extend_from_slice(&[0x78, 0x01]);
        }
        let mut i = 0usize;
        while i < data.len() || data.is_empty() && out.len() <= 2 {
            let n = (data.len() - i).min(65535);
            let last = i + n == data.len();
            // BFINAL, BTYPE = 00, then 5 bits of padding so the header ends on a byte.
            out.push(if last { 1 } else { 0 });
            out.extend_from_slice(&(n as u16).to_le_bytes());
            out.extend_from_slice(&(!n as u16).to_le_bytes());
            out.extend_from_slice(&data[i..i + n]);
            i += n;
            if data.is_empty() {
                break;
            }
        }
        if zlib {
            let mut a = 1u32;
            let mut b = 0u32;
            for &byte in data {
                a = (a + byte as u32) % 65521;
                b = (b + a) % 65521;
            }
            let sum = (b << 16) | a;
            out.extend_from_slice(&sum.to_be_bytes());
        }
        out
    }

    fn recompress(file: &[u8], zlib: bool) -> Vec<u8> {
        let (info, table) = parse(file).unwrap();
        assert_eq!(info.compression, 0);
        let mut scratch = vec![0u8; info.bps * info.lines * 2 + 64];
        let blocks = (info.height + info.lines - 1) / info.lines;
        let mut chunks = Vec::new();
        for b in 0..blocks {
            let off = u64::from_le_bytes(file[table + b * 8..table + b * 8 + 8].try_into().unwrap()) as usize;
            let (y, nlines) = decode_chunk(file, off, &info, &mut scratch).unwrap();
            let raw_len = nlines * info.bps;
            let packed = deconstruct(&scratch[..raw_len]);
            let comp = stored_deflate(&packed, zlib);
            chunks.push((y, comp));
        }
        let mut out = file[..table].to_vec();
        // ZIPS is one scanline per chunk, which is how the uncompressed file was written.
        let marker = b"compression\0compression\0";
        let at = out.windows(marker.len()).position(|w| w == marker).unwrap();
        let value = at + marker.len() + 4;
        out[value] = 2;
        let table_n = out.len();
        out.extend(std::iter::repeat(0).take(blocks * 8));
        for (i, (y, comp)) in chunks.iter().enumerate() {
            let off = out.len() as u64;
            out[table_n + i * 8..table_n + i * 8 + 8].copy_from_slice(&off.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
            out.extend_from_slice(&(comp.len() as i32).to_le_bytes());
            out.extend_from_slice(comp);
        }
        out
    }

    #[test]
    fn zip_and_zlib_roundtrip() {
        let res = 4u32;
        let mut pixels = vec![0u16; 4 * 4 * 4];
        for i in 0..16usize {
            pixels[i * 4] = (1000 + i * 17) as u16;
            pixels[i * 4 + 1] = (2000 + i * 13) as u16;
            pixels[i * 4 + 2] = (3000 + i * 19) as u16;
            pixels[i * 4 + 3] = if i % 3 == 0 { 0 } else { 65535 };
        }
        let mut scratch = vec![0u8; 32 * 1024];
        let mut out = vec![0u8; 32 * 1024];
        let n = encode_rgba16(&pixels, res, "zip probe", &mut scratch, &mut out, 2.5).unwrap();
        for zlib in [false, true] {
            let zipped = recompress(&out[..n], zlib);
            let mut back = vec![0u16; pixels.len()];
            let got = decode_rgba16(&zipped, &mut back, &mut scratch, 2.5).unwrap();
            assert_eq!(got, res, "zlib {zlib}");
            assert_eq!(back, pixels, "zlib {zlib}");
        }
    }

    #[test]
    fn blobbies_center_pixel_matches_ilm() {
        let path = "/tmp/Blobbies.exr";
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => return,
        };
        let mut scratch = vec![0u8; 600_000];
        let px = sample_rgba(&bytes, &mut scratch, 500, 500).unwrap();
        assert!((px[0] - 0.16516113).abs() < 2e-4, "{px:?}");
        assert!((px[1] - 0.29125977).abs() < 2e-4, "{px:?}");
        assert!((px[2] - 0.06732178).abs() < 2e-4, "{px:?}");
        assert!((px[3] - 1.0009766).abs() < 2e-3, "{px:?}");
        let mut pixels = vec![0u16; 256 * 256 * 4];
        let res = decode_rgba16(&bytes, &mut pixels, &mut scratch, 2.5).unwrap();
        assert_eq!(res, 256);
        assert!(pixels.iter().any(|v| *v != 0));
    }
}
