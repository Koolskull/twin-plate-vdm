//! 16-bit RGBA PNG, written with stored (uncompressed) zlib blocks.
//! Any PNG reader can open the files. Deflate is intentionally not used so the
//! codec stays small enough to read in one file.

use core::fmt::Write;

const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";

struct W<'a> {
    buf: &'a mut [u8],
    n: usize,
    ok: bool,
}

impl<'a> W<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, n: 0, ok: true }
    }

    fn u8(&mut self, v: u8) {
        if self.n >= self.buf.len() {
            self.ok = false;
            return;
        }
        self.buf[self.n] = v;
        self.n += 1;
    }

    fn bytes(&mut self, data: &[u8]) {
        if self.n + data.len() > self.buf.len() {
            self.ok = false;
            return;
        }
        self.buf[self.n..self.n + data.len()].copy_from_slice(data);
        self.n += data.len();
    }

    fn u16le(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }

    fn u32be(&mut self, v: u32) {
        self.bytes(&v.to_be_bytes());
    }

    fn patch_u32be(&mut self, at: usize, v: u32) {
        if at + 4 > self.n {
            self.ok = false;
            return;
        }
        self.buf[at..at + 4].copy_from_slice(&v.to_be_bytes());
    }
}

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = if crc & 1 == 0 { 0 } else { 0xEDB8_8320 };
            crc = (crc >> 1) ^ mask;
        }
    }
    crc
}

fn crc32(data: &[u8]) -> u32 {
    crc32_update(0xFFFF_FFFF, data) ^ 0xFFFF_FFFF
}

fn crc32_parts(a: &[u8], b: &[u8]) -> u32 {
    let crc = crc32_update(0xFFFF_FFFF, a);
    crc32_update(crc, b) ^ 0xFFFF_FFFF
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a += byte as u32;
        if a >= 65521 {
            a -= 65521;
        }
        b += a;
        if b >= 65521 {
            b -= 65521;
        }
    }
    (b << 16) | a
}

fn chunk(w: &mut W, tag: &[u8; 4], data: &[u8]) {
    w.u32be(data.len() as u32);
    let start = w.n;
    w.bytes(tag);
    w.bytes(data);
    if !w.ok {
        return;
    }
    let crc = crc32(&w.buf[start..w.n]);
    w.u32be(crc);
}

fn u32be_at(data: &[u8]) -> u32 {
    u32::from_be_bytes([data[0], data[1], data[2], data[3]])
}

/// Encode `res * res` RGBA16 pixels. `comment` is stored in a tEXt chunk.
pub fn encode_rgba16(
    pixels: &[u16],
    res: u32,
    comment: &str,
    raw: &mut [u8],
    out: &mut [u8],
) -> Result<usize, ()> {
    let res_us = res as usize;
    if res == 0 || pixels.len() < res_us * res_us * 4 {
        return Err(());
    }
    let row = 1 + res_us * 8;
    let raw_len = res_us * row;
    if raw.len() < raw_len {
        return Err(());
    }
    for y in 0..res_us {
        let dst = y * row;
        raw[dst] = 0;
        for x in 0..res_us {
            let src = (y * res_us + x) * 4;
            for c in 0..4 {
                let v = pixels[src + c];
                let o = dst + 1 + (x * 4 + c) * 2;
                raw[o] = (v >> 8) as u8;
                raw[o + 1] = (v & 0xff) as u8;
            }
        }
    }
    let raw_slice_len = raw_len;

    let mut w = W::new(out);
    w.bytes(SIG);

    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&res.to_be_bytes());
    ihdr[4..8].copy_from_slice(&res.to_be_bytes());
    ihdr[8] = 16;
    ihdr[9] = 6;
    chunk(&mut w, b"IHDR", &ihdr);

    if !comment.is_empty() {
        let mut text = [0u8; 220];
        let keyword = b"Comment\0";
        if keyword.len() + comment.len() > text.len() {
            return Err(());
        }
        text[..keyword.len()].copy_from_slice(keyword);
        text[keyword.len()..keyword.len() + comment.len()].copy_from_slice(comment.as_bytes());
        chunk(&mut w, b"tEXt", &text[..keyword.len() + comment.len()]);
    }

    let len_at = w.n;
    w.u32be(0);
    let type_at = w.n;
    w.bytes(b"IDAT");
    let data_at = w.n;
    write_zlib_stored(&mut w, &raw[..raw_slice_len]);
    if !w.ok {
        return Err(());
    }
    let data_len = w.n - data_at;
    w.patch_u32be(len_at, data_len as u32);
    let crc = crc32(&w.buf[type_at..w.n]);
    w.u32be(crc);
    chunk(&mut w, b"IEND", &[]);
    if !w.ok {
        return Err(());
    }
    Ok(w.n)
}

fn write_zlib_stored(w: &mut W, data: &[u8]) {
    w.u8(0x78);
    w.u8(0x01);
    let mut i = 0;
    while i < data.len() {
        let n = (data.len() - i).min(65535);
        let last = i + n == data.len();
        w.u8(if last { 1 } else { 0 });
        w.u16le(n as u16);
        w.u16le(!(n as u16));
        w.bytes(&data[i..i + n]);
        i += n;
    }
    w.u32be(adler32(data));
}

pub fn decode_rgba16(png: &[u8], pixels: &mut [u16], raw: &mut [u8]) -> Result<u32, ()> {
    if png.len() < 8 || &png[..8] != SIG {
        return Err(());
    }
    let mut i = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut saw_ihdr = false;
    let mut idat: Option<(usize, usize)> = None;
    while i + 12 <= png.len() {
        let len = u32be_at(&png[i..]) as usize;
        let type_at = i + 4;
        let data_at = i + 8;
        let data_end = data_at.checked_add(len).ok_or(())?;
        let crc_end = data_end.checked_add(4).ok_or(())?;
        if crc_end > png.len() {
            return Err(());
        }
        let tag = &png[type_at..data_at];
        let data = &png[data_at..data_end];
        let expect = crc32_parts(tag, data);
        let got = u32be_at(&png[data_end..]);
        if expect != got {
            return Err(());
        }
        if tag == b"IHDR" {
            if data.len() < 13 {
                return Err(());
            }
            width = u32be_at(&data[0..]);
            height = u32be_at(&data[4..]);
            if data[8] != 16 || data[9] != 6 || data[10] != 0 || data[11] != 0 || data[12] != 0 {
                return Err(());
            }
            if width != height || width == 0 || width > 4096 {
                return Err(());
            }
            saw_ihdr = true;
        } else if tag == b"IDAT" {
            if idat.is_some() {
                return Err(());
            }
            idat = Some((data_at, data_end));
        } else if tag == b"IEND" {
            break;
        }
        i = crc_end;
    }
    if !saw_ihdr {
        return Err(());
    }
    let (idat_at, idat_end) = idat.ok_or(())?;
    let inflated = inflate_stored(&png[idat_at..idat_end], raw)?;
    let res = width as usize;
    let row = 1 + res * 8;
    if height as usize != res || inflated != res * row {
        return Err(());
    }
    if pixels.len() < res * res * 4 {
        return Err(());
    }
    for y in 0..res {
        let row_bytes = &raw[y * row..(y + 1) * row];
        if row_bytes[0] != 0 {
            return Err(());
        }
        for x in 0..res {
            let dst = (y * res + x) * 4;
            for c in 0..4 {
                let o = 1 + (x * 4 + c) * 2;
                pixels[dst + c] = u16::from_be_bytes([row_bytes[o], row_bytes[o + 1]]);
            }
        }
    }
    Ok(width)
}

fn inflate_stored(z: &[u8], out: &mut [u8]) -> Result<usize, ()> {
    if z.len() < 6 || z[0] != 0x78 {
        return Err(());
    }
    let mut i = 2;
    let mut o = 0usize;
    loop {
        if i + 5 > z.len() {
            return Err(());
        }
        let header = z[i];
        if header > 1 {
            return Err(());
        }
        let len = u16::from_le_bytes([z[i + 1], z[i + 2]]) as usize;
        let nlen = u16::from_le_bytes([z[i + 3], z[i + 4]]);
        if nlen != !(len as u16) {
            return Err(());
        }
        i += 5;
        if i + len > z.len() || o + len > out.len() {
            return Err(());
        }
        out[o..o + len].copy_from_slice(&z[i..i + len]);
        o += len;
        i += len;
        if header == 1 {
            break;
        }
    }
    if i + 4 > z.len() {
        return Err(());
    }
    let got = u32be_at(&z[i..]);
    if got != adler32(&out[..o]) {
        return Err(());
    }
    Ok(o)
}

pub fn comment_for(shape: u32, name: &str, res: u32, lean: f32, plate: &str) -> Comment {
    let mut c = Comment::default();
    let _ = write!(
        c,
        "twin-plate-vdm plate={plate} shape={shape} name={name} res={res} lean={lean:.4} range=2.5 span=2 plateY=0 rgb=XYZ alpha=mask"
    );
    c
}

pub struct Comment {
    buf: [u8; 180],
    len: usize,
}

impl Default for Comment {
    fn default() -> Self {
        Self {
            buf: [0; 180],
            len: 0,
        }
    }
}

impl Write for Comment {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        if self.len + bytes.len() > self.buf.len() {
            return Err(core::fmt::Error);
        }
        self.buf[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        Ok(())
    }
}

impl Comment {
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_png_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn adler_matches_known_vector() {
        assert_eq!(adler32(b"123456789"), 0x091E_01DE);
    }

    #[test]
    fn rgba16_roundtrip() {
        let mut pixels = [0u16; 2 * 2 * 4];
        for (i, px) in pixels.iter_mut().enumerate() {
            *px = (i as u16).wrapping_mul(4000).wrapping_add(7);
        }
        let mut raw = [0u8; 128];
        let mut out = [0u8; 512];
        let n = encode_rgba16(&pixels, 2, "twin-plate-vdm test", &mut raw, &mut out).unwrap();
        assert_eq!(&out[..8], SIG);
        let mut back = [9u16; 16];
        let mut raw2 = [0u8; 128];
        let res = decode_rgba16(&out[..n], &mut back, &mut raw2).unwrap();
        assert_eq!(res, 2);
        assert_eq!(back, pixels);
    }
}
