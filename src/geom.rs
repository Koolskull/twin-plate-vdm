//! Bake two plate images from a math solid, pack them as PNG, and rebuild a mesh
//! by sampling those images. The mesh path does not call the solid again except
//! to measure how far the result drifted from the formula.

#[cfg(target_arch = "wasm32")]
use crate::fmath::FloatMath;
use crate::math::Vec3;
use crate::pngio::{self, decode_rgba16, encode_rgba16};
use crate::shape::{self, cast, residual};

/// Both plates share this plane. Zero puts them on the same center.
pub const PLATE_Y: f32 = 0.0;
pub const SPAN: f32 = 2.0;
pub const RANGE: f32 = 2.5;
pub const MAX_RES: u32 = 256;

const MAX: usize = 256;
const PIXELS: usize = MAX * MAX;
const PNG_MAX: usize = 540_000;
const RAW_MAX: usize = MAX * (1 + MAX * 8);
const VERT_STRIDE: usize = 6;
const MAX_VERTS: usize = 2 * MAX * MAX;
const MAX_INDICES: usize = 1_048_576;
const MAX_ARROWS: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Header {
    pub status: u32,
    pub shape: u32,
    pub bake_res: u32,
    pub mesh_res: u32,
    pub png_plus_ptr: u32,
    pub png_plus_len: u32,
    pub png_minus_ptr: u32,
    pub png_minus_len: u32,
    pub vert_ptr: u32,
    pub vert_count: u32,
    pub vert_stride: u32,
    pub index_ptr: u32,
    pub index_count: u32,
    pub arrow_ptr: u32,
    pub arrow_count: u32,
    pub valid_plus: u32,
    pub valid_minus: u32,
    pub sample_count: u32,
    pub tri_count: u32,
    pub clamped: u32,
    pub center_plus_ok: u32,
    pub center_minus_ok: u32,
    pub quant_error: f32,
    pub surface_error: f32,
    pub opposing: f32,
    pub plate_y: f32,
    pub span: f32,
    pub range: f32,
    pub center_plus: [f32; 3],
    pub center_minus: [f32; 3],
}

impl Header {
    const fn zero() -> Self {
        Self {
            status: 5,
            shape: 0,
            bake_res: 0,
            mesh_res: 0,
            png_plus_ptr: 0,
            png_plus_len: 0,
            png_minus_ptr: 0,
            png_minus_len: 0,
            vert_ptr: 0,
            vert_count: 0,
            vert_stride: VERT_STRIDE as u32,
            index_ptr: 0,
            index_count: 0,
            arrow_ptr: 0,
            arrow_count: 0,
            valid_plus: 0,
            valid_minus: 0,
            sample_count: 0,
            tri_count: 0,
            clamped: 0,
            center_plus_ok: 0,
            center_minus_ok: 0,
            quant_error: 0.0,
            surface_error: 0.0,
            opposing: 0.0,
            plate_y: PLATE_Y,
            span: SPAN,
            range: RANGE,
            center_plus: [0.0; 3],
            center_minus: [0.0; 3],
        }
    }
}

struct State {
    plus: [u16; PIXELS * 4],
    minus: [u16; PIXELS * 4],
    png_plus: [u8; PNG_MAX],
    png_minus: [u8; PNG_MAX],
    png_plus_len: u32,
    png_minus_len: u32,
    raw: [u8; RAW_MAX],
    verts: [f32; MAX_VERTS * VERT_STRIDE],
    indices: [u32; MAX_INDICES],
    arrows: [f32; MAX_ARROWS * 6],
    mask: [u8; PIXELS],
    bake_res: u32,
    shape: u32,
    ready: u32,
    header: Header,
}

/// One baker at a time. The page is single-threaded wasm. Host tests run on one thread.
struct StateCell(core::cell::UnsafeCell<State>);
unsafe impl Sync for StateCell {}

static STATE: StateCell = StateCell(core::cell::UnsafeCell::new(State {
    plus: [0; PIXELS * 4],
    minus: [0; PIXELS * 4],
    png_plus: [0; PNG_MAX],
    png_minus: [0; PNG_MAX],
    png_plus_len: 0,
    png_minus_len: 0,
    raw: [0; RAW_MAX],
    verts: [0.0; MAX_VERTS * VERT_STRIDE],
    indices: [0; MAX_INDICES],
    arrows: [0.0; MAX_ARROWS * 6],
    mask: [0; PIXELS],
    bake_res: 0,
    shape: 0,
    ready: 0,
    header: Header::zero(),
}));

fn state() -> &'static mut State {
    unsafe { &mut *STATE.0.get() }
}

pub fn enc(c: f32) -> u16 {
    let t = (c / (2.0 * RANGE) + 0.5).clamp(0.0, 1.0);
    let q = (t * 65535.0).round();
    if q <= 0.0 {
        0
    } else if q >= 65535.0 {
        65535
    } else {
        q as u16
    }
}

pub fn dec(q: u16) -> f32 {
    (q as f32 / 65535.0 - 0.5) * (2.0 * RANGE)
}

fn quantize(v: Vec3, err: &mut f32, clamped: &mut u32) -> [u16; 3] {
    let mut q = [0u16; 3];
    for k in 0..3 {
        let c = v.comp(k);
        if c.abs() > RANGE {
            *clamped += 1;
        }
        let encoded = enc(c);
        q[k] = encoded;
        let delta = (dec(encoded) - c.clamp(-RANGE, RANGE)).abs();
        if delta > *err {
            *err = delta;
        }
    }
    q
}

fn write_texel(img: &mut [u16], res: u32, i: u32, j: u32, rgb: [u16; 3], alpha: u16) {
    let o = ((j * res + i) * 4) as usize;
    img[o] = rgb[0];
    img[o + 1] = rgb[1];
    img[o + 2] = rgb[2];
    img[o + 3] = alpha;
}

fn texel(img: &[u16], res: u32, i: i32, j: i32) -> Option<[u16; 4]> {
    if i < 0 || j < 0 || i >= res as i32 || j >= res as i32 {
        return None;
    }
    let o = ((j as u32 * res + i as u32) * 4) as usize;
    let px = [img[o], img[o + 1], img[o + 2], img[o + 3]];
    if px[3] < 32768 {
        None
    } else {
        Some(px)
    }
}

fn texel_vec(img: &[u16], res: u32, i: i32, j: i32) -> Option<Vec3> {
    texel(img, res, i, j).map(|px| Vec3::new(dec(px[0]), dec(px[1]), dec(px[2])))
}

pub fn decoded_vector(plate: u32, i: u32, j: u32) -> Option<Vec3> {
    let s = unsafe { &*STATE.0.get() };
    if s.ready == 0 || i >= s.bake_res || j >= s.bake_res {
        return None;
    }
    let img = if plate == 0 { &s.plus[..] } else { &s.minus[..] };
    texel_vec(img, s.bake_res, i as i32, j as i32)
}

fn checksum(img: &[u16], res: u32) -> u32 {
    let n = (res * res * 4) as usize;
    let mut sum = 0u32;
    for &v in &img[..n] {
        sum = sum.wrapping_add(v as u32);
    }
    sum
}

fn zero_images(s: &mut State) {
    unsafe {
        core::ptr::write_bytes(s.plus.as_mut_ptr(), 0, s.plus.len());
        core::ptr::write_bytes(s.minus.as_mut_ptr(), 0, s.minus.len());
    }
}

fn fill_ptrs(s: &mut State) {
    s.header.png_plus_ptr = s.png_plus.as_ptr() as u32;
    s.header.png_minus_ptr = s.png_minus.as_ptr() as u32;
    s.header.vert_ptr = s.verts.as_ptr() as u32;
    s.header.index_ptr = s.indices.as_ptr() as u32;
    s.header.arrow_ptr = s.arrows.as_ptr() as u32;
    s.header.vert_stride = VERT_STRIDE as u32;
    s.header.plate_y = PLATE_Y;
    s.header.span = SPAN;
    s.header.range = RANGE;
    s.header.png_plus_len = s.png_plus_len;
    s.header.png_minus_len = s.png_minus_len;
    s.header.shape = s.shape;
    s.header.bake_res = s.bake_res;
}

fn ray_dir(x: f32, z: f32, y_sign: f32, lean: f32) -> Vec3 {
    // y_sign is the plate normal. Lean splays the ray away from the center.
    Vec3::new(lean * x, y_sign, lean * z).normalized()
}

fn plate_origin(_plate: u32, u: f32, v: f32) -> Vec3 {
    let x = (u - 0.5) * SPAN;
    let z = (v - 0.5) * SPAN;
    Vec3::new(x, PLATE_Y, z)
}

pub fn bake_model(shape: u32, res: u32, lean: f32) -> u32 {
    if shape >= shape::SHAPE_COUNT || !(8..=MAX_RES).contains(&res) || !lean.is_finite() {
        let s = state();
        s.ready = 0;
        s.header.status = 1;
        return 1;
    }
    let s = state();
    s.ready = 0;
    s.shape = shape;
    s.bake_res = res;
    let kept_surface = 0.0f32;
    s.header = Header::zero();
    s.header.surface_error = kept_surface;
    s.header.shape = shape;
    s.header.bake_res = res;

    let mut quant_error = 0.0f32;
    let mut clamped = 0u32;
    let neutral = quantize(Vec3::ZERO, &mut 0.0, &mut 0);

    for j in 0..res {
        for i in 0..res {
            let u = (i as f32 + 0.5) / res as f32;
            let v = (j as f32 + 0.5) / res as f32;
            let x = (u - 0.5) * SPAN;
            let z = (v - 0.5) * SPAN;
            let origin = Vec3::new(x, PLATE_Y, z);
            let hit_plus = cast(shape, origin, ray_dir(x, z, 1.0, lean));
            let hit_minus = cast(shape, origin, ray_dir(x, z, -1.0, lean));
            let (rgb_p, alpha_p) = match hit_plus {
                Some(hit) => (quantize(hit - origin, &mut quant_error, &mut clamped), 65535),
                None => (neutral, 0),
            };
            let (rgb_m, alpha_m) = match hit_minus {
                Some(hit) => (
                    quantize(hit - origin, &mut quant_error, &mut clamped),
                    65535,
                ),
                None => (neutral, 0),
            };
            write_texel(&mut s.plus, res, i, j, rgb_p, alpha_p);
            write_texel(&mut s.minus, res, i, j, rgb_m, alpha_m);
        }
    }

    let sum = checksum(&s.plus, res) ^ checksum(&s.minus, res).rotate_left(1);
    let used = (res * res * 4) as usize;
    let comment_plus = pngio::comment_for(shape, shape::shape_name(shape), res, lean, "plus");
    let plus_len = match encode_rgba16(
        &s.plus[..used],
        res,
        comment_plus.as_str(),
        &mut s.raw,
        &mut s.png_plus,
    ) {
        Ok(n) => n as u32,
        Err(()) => {
            s.header.status = 2;
            return 2;
        }
    };
    let comment_minus = pngio::comment_for(shape, shape::shape_name(shape), res, lean, "minus");
    let minus_len = match encode_rgba16(
        &s.minus[..used],
        res,
        comment_minus.as_str(),
        &mut s.raw,
        &mut s.png_minus,
    ) {
        Ok(n) => n as u32,
        Err(()) => {
            s.header.status = 2;
            return 2;
        }
    };
    s.png_plus_len = plus_len;
    s.png_minus_len = minus_len;
    zero_images(s);
    if decode_pair(s).is_err() {
        s.ready = 0;
        s.header.status = 3;
        return 3;
    }
    let sum_after = checksum(&s.plus, res) ^ checksum(&s.minus, res).rotate_left(1);
    if sum != sum_after || s.bake_res != res {
        s.ready = 0;
        s.header.status = 3;
        return 3;
    }
    publish_image_stats(s);
    s.header.quant_error = quant_error;
    s.header.clamped = clamped;
    s.ready = 1;
    s.header.status = 0;
    0
}

fn decode_pair(s: &mut State) -> Result<(), ()> {
    let plus_len = s.png_plus_len as usize;
    let minus_len = s.png_minus_len as usize;
    let res_plus = decode_rgba16(
        &s.png_plus[..plus_len],
        &mut s.plus,
        &mut s.raw,
    )?;
    let res_minus = decode_rgba16(
        &s.png_minus[..minus_len],
        &mut s.minus,
        &mut s.raw,
    )?;
    if res_plus != res_minus || res_plus < 8 || res_plus > MAX_RES {
        return Err(());
    }
    s.bake_res = res_plus;
    Ok(())
}

fn publish_image_stats(s: &mut State) {
    let res = s.bake_res;
    let mut valid_plus = 0u32;
    let mut valid_minus = 0u32;
    let mut both = 0u32;
    let mut facing = 0u32;
    for j in 0..res {
        for i in 0..res {
            let top = texel_vec(&s.plus, res, i as i32, j as i32);
            let bottom = texel_vec(&s.minus, res, i as i32, j as i32);
            if top.is_some() {
                valid_plus += 1;
            }
            if bottom.is_some() {
                valid_minus += 1;
            }
            if let (Some(a), Some(b)) = (top, bottom) {
                both += 1;
                if a.y > 0.0 && b.y < 0.0 {
                    facing += 1;
                }
            }
        }
    }
    s.header.valid_plus = valid_plus;
    s.header.valid_minus = valid_minus;
    s.header.opposing = if both == 0 {
        1.0
    } else {
        facing as f32 / both as f32
    };
    s.header.shape = s.shape;
    s.header.bake_res = res;
    s.header.plate_y = PLATE_Y;
    s.header.span = SPAN;
    s.header.range = RANGE;
    write_center(s);
    fill_ptrs(s);
}

fn write_center(s: &mut State) {
    let c = s.bake_res / 2;
    match texel_vec(&s.plus, s.bake_res, c as i32, c as i32) {
        Some(v) => {
            s.header.center_plus = [v.x, v.y, v.z];
            s.header.center_plus_ok = 1;
        }
        None => {
            s.header.center_plus = [0.0; 3];
            s.header.center_plus_ok = 0;
        }
    }
    match texel_vec(&s.minus, s.bake_res, c as i32, c as i32) {
        Some(v) => {
            s.header.center_minus = [v.x, v.y, v.z];
            s.header.center_minus_ok = 1;
        }
        None => {
            s.header.center_minus = [0.0; 3];
            s.header.center_minus_ok = 0;
        }
    }
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn shape_from_png(png: &[u8]) -> Option<u32> {
    let marker = b"shape=";
    let pos = find_subslice(png, marker)? + marker.len();
    let mut n = 0u32;
    let mut i = pos;
    let mut any = false;
    while i < png.len() && png[i].is_ascii_digit() {
        any = true;
        n = n * 10 + (png[i] - b'0') as u32;
        i += 1;
        if n > 99 {
            break;
        }
    }
    if any { Some(n) } else { None }
}

pub fn wipe_images() {
    let s = state();
    zero_images(s);
    s.ready = 0;
    s.header.status = 5;
}

pub fn ingest_pngs(plus: &[u8], minus: &[u8]) -> u32 {
    if plus.is_empty() || minus.is_empty() || plus.len() > PNG_MAX || minus.len() > PNG_MAX {
        let s = state();
        s.ready = 0;
        s.header.status = 3;
        return 3;
    }
    let s = state();
    s.png_plus[..plus.len()].copy_from_slice(plus);
    s.png_minus[..minus.len()].copy_from_slice(minus);
    s.png_plus_len = plus.len() as u32;
    s.png_minus_len = minus.len() as u32;
    finish_ingest(s)
}

fn finish_ingest(s: &mut State) -> u32 {
    if let Some(shape) = shape_from_png(&s.png_plus[..s.png_plus_len as usize]) {
        if shape < shape::SHAPE_COUNT {
            s.shape = shape;
        }
    }
    zero_images(s);
    if decode_pair(s).is_err() {
        s.ready = 0;
        s.header.status = 3;
        return 3;
    }
    let quant = s.header.quant_error;
    let clamped = s.header.clamped;
    publish_image_stats(s);
    s.header.quant_error = quant;
    s.header.clamped = clamped;
    s.ready = 1;
    s.header.status = 0;
    0
}

#[cfg(target_arch = "wasm32")]
pub fn load_pngs_from_memory(p_ptr: u32, p_len: u32, m_ptr: u32, m_len: u32) -> u32 {
    if p_len == 0 || m_len == 0 || p_len as usize > PNG_MAX || m_len as usize > PNG_MAX {
        let s = state();
        s.ready = 0;
        s.header.status = 3;
        return 3;
    }
    unsafe {
        let raw = STATE.0.get();
        let plus_addr = (*raw).png_plus.as_ptr() as u32;
        let minus_addr = (*raw).png_minus.as_ptr() as u32;
        if p_ptr != plus_addr {
            core::ptr::copy_nonoverlapping(
                p_ptr as *const u8,
                (*raw).png_plus.as_mut_ptr(),
                p_len as usize,
            );
        }
        if m_ptr != minus_addr {
            core::ptr::copy_nonoverlapping(
                m_ptr as *const u8,
                (*raw).png_minus.as_mut_ptr(),
                m_len as usize,
            );
        }
        (*raw).png_plus_len = p_len;
        (*raw).png_minus_len = m_len;
    }
    finish_ingest(state())
}

#[cfg(target_arch = "wasm32")]
pub fn header_address() -> u32 {
    unsafe { &(*STATE.0.get()).header as *const Header as u32 }
}

pub fn header() -> Header {
    unsafe { (*STATE.0.get()).header }
}

pub fn png_plus() -> &'static [u8] {
    unsafe {
        let s = &*STATE.0.get();
        &s.png_plus[..s.png_plus_len as usize]
    }
}

pub fn png_minus() -> &'static [u8] {
    unsafe {
        let s = &*STATE.0.get();
        &s.png_minus[..s.png_minus_len as usize]
    }
}

pub fn mesh_vertices() -> &'static [f32] {
    unsafe {
        let s = &*STATE.0.get();
        let n = s.header.vert_count as usize * VERT_STRIDE;
        &s.verts[..n]
    }
}

pub fn mesh_indices() -> &'static [u32] {
    unsafe {
        let s = &*STATE.0.get();
        &s.indices[..s.header.index_count as usize]
    }
}

fn uv(i: u32, j: u32, n: u32) -> (f32, f32) {
    (
        (i as f32 + 0.5) / n as f32,
        (j as f32 + 0.5) / n as f32,
    )
}

fn fetch(img: &[u16], bake: u32, u: f32, v: f32) -> Option<Vec3> {
    let res = bake as f32;
    let x = u * res - 0.5;
    let y = v * res - 0.5;
    let ix = x.round() as i32;
    let iy = y.round() as i32;
    if texel(img, bake, ix, iy).is_none() {
        return None;
    }
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let corners = [
        (x0, y0, (1.0 - tx) * (1.0 - ty)),
        (x0 + 1, y0, tx * (1.0 - ty)),
        (x0, y0 + 1, (1.0 - tx) * ty),
        (x0 + 1, y0 + 1, tx * ty),
    ];
    let mut acc = Vec3::ZERO;
    let mut wsum = 0.0f32;
    for (cx, cy, w) in corners {
        if w <= 1e-8 {
            continue;
        }
        if let Some(sample) = texel_vec(img, bake, cx, cy) {
            acc = acc + sample * w;
            wsum += w;
        }
    }
    if wsum < 1e-6 {
        None
    } else {
        Some(acc * (1.0 / wsum))
    }
}

fn vid(plate: u32, i: u32, j: u32, n: u32) -> u32 {
    plate * n * n + j * n + i
}

pub fn build_mesh(n: u32) -> u32 {
    let s = state();
    if s.ready == 0 || s.bake_res == 0 {
        s.header.status = 5;
        return 5;
    }
    if !(4..=MAX_RES).contains(&n) {
        s.header.status = 1;
        return 1;
    }
    let bake = s.bake_res;
    let shape = s.shape;
    let mut samples = 0u32;
    for j in 0..n {
        for i in 0..n {
            let (u, v) = uv(i, j, n);
            let top = fetch(&s.plus, bake, u, v).is_some();
            let bottom = fetch(&s.minus, bake, u, v).is_some();
            let mut bits = 0u8;
            if top {
                bits |= 1;
            }
            if bottom {
                bits |= 2;
            }
            s.mask[(j * n + i) as usize] = bits;
            if bits == 3 {
                samples += 1;
            }
        }
    }

    let mut max_residual = 0.0f32;
    for plate in 0..2u32 {
        for j in 0..n {
            for i in 0..n {
                let (u, v) = uv(i, j, n);
                let origin = plate_origin(plate, u, v);
                let img = if plate == 0 { &s.plus[..] } else { &s.minus[..] };
                let position = if let Some(vector) = fetch(img, bake, u, v) {
                    let p = origin + vector;
                    let err = residual(shape, p);
                    if err > max_residual {
                        max_residual = err;
                    }
                    p
                } else {
                    origin
                };
                let slot = ((plate * n * n + j * n + i) as usize) * VERT_STRIDE;
                s.verts[slot] = position.x;
                s.verts[slot + 1] = position.y;
                s.verts[slot + 2] = position.z;
                s.verts[slot + 3] = origin.x;
                s.verts[slot + 4] = origin.y;
                s.verts[slot + 5] = origin.z;
            }
        }
    }

    let mut ic = 0usize;
    let mut overflow = false;
    let mut push = |buf: &mut [u32], a: u32, b: u32, c: u32| {
        if ic + 3 > buf.len() {
            overflow = true;
            return;
        }
        buf[ic] = a;
        buf[ic + 1] = b;
        buf[ic + 2] = c;
        ic += 3;
    };

    // Bit 1 is the upward plate, bit 2 the downward plate. A shell is emitted
    // wherever that plate hit. The wall between them is only where both hit
    // and the neighbor is the outside of the solid.
    let plate_filled = |mask: &[u8], i: i32, j: i32, bit: u8| -> bool {
        if i < 0 || j < 0 {
            return false;
        }
        let i = i as u32;
        let j = j as u32;
        if i + 1 >= n || j + 1 >= n {
            return false;
        }
        let ok = |ii: u32, jj: u32| mask[(jj * n + ii) as usize] & bit == bit;
        ok(i, j) && ok(i + 1, j) && ok(i + 1, j + 1) && ok(i, j + 1)
    };

    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let ii = i as i32;
            let jj = j as i32;
            let up = plate_filled(&s.mask, ii, jj, 1);
            let down = plate_filled(&s.mask, ii, jj, 2);
            if !up && !down {
                continue;
            }
            if up {
                let a = vid(0, i, j, n);
                let b = vid(0, i + 1, j, n);
                let c = vid(0, i + 1, j + 1, n);
                let d = vid(0, i, j + 1, n);
                push(&mut s.indices, a, d, b);
                push(&mut s.indices, b, d, c);
            }
            if down {
                let a = vid(1, i, j, n);
                let b = vid(1, i + 1, j, n);
                let c = vid(1, i + 1, j + 1, n);
                let d = vid(1, i, j + 1, n);
                push(&mut s.indices, a, b, d);
                push(&mut s.indices, b, c, d);
            }
            if !(up && down) {
                continue;
            }
            if !plate_filled(&s.mask, ii, jj - 1, 3) {
                let t_a = vid(0, i, j, n);
                let t_b = vid(0, i + 1, j, n);
                let b_a = vid(1, i, j, n);
                let b_b = vid(1, i + 1, j, n);
                push(&mut s.indices, t_a, t_b, b_b);
                push(&mut s.indices, t_a, b_b, b_a);
            }
            if !plate_filled(&s.mask, ii + 1, jj, 3) {
                let t_b = vid(0, i + 1, j, n);
                let t_c = vid(0, i + 1, j + 1, n);
                let b_b = vid(1, i + 1, j, n);
                let b_c = vid(1, i + 1, j + 1, n);
                push(&mut s.indices, t_b, t_c, b_c);
                push(&mut s.indices, t_b, b_c, b_b);
            }
            if !plate_filled(&s.mask, ii, jj + 1, 3) {
                let t_c = vid(0, i + 1, j + 1, n);
                let t_d = vid(0, i, j + 1, n);
                let b_c = vid(1, i + 1, j + 1, n);
                let b_d = vid(1, i, j + 1, n);
                push(&mut s.indices, t_c, t_d, b_d);
                push(&mut s.indices, t_c, b_d, b_c);
            }
            if !plate_filled(&s.mask, ii - 1, jj, 3) {
                let t_d = vid(0, i, j + 1, n);
                let t_a = vid(0, i, j, n);
                let b_d = vid(1, i, j + 1, n);
                let b_a = vid(1, i, j, n);
                push(&mut s.indices, t_d, t_a, b_a);
                push(&mut s.indices, t_d, b_a, b_d);
            }
        }
    }

    if overflow {
        s.header.status = 4;
        s.header.index_count = 0;
        s.header.tri_count = 0;
        return 4;
    }

    // The mesh uses every sample. These rays are only a reading aid.
    let stride = (n / 8).max(1);
    let mut arrows = 0usize;
    for plate in 0..2u32 {
        let mut j = 0u32;
        while j < n {
            let mut i = 0u32;
            while i < n {
                let (u, v) = uv(i, j, n);
                let img = if plate == 0 { &s.plus[..] } else { &s.minus[..] };
                if let Some(vector) = fetch(img, bake, u, v) {
                    if arrows < MAX_ARROWS {
                        let origin = plate_origin(plate, u, v);
                        let tip = origin + vector;
                        let slot = arrows * 6;
                        s.arrows[slot] = origin.x;
                        s.arrows[slot + 1] = origin.y;
                        s.arrows[slot + 2] = origin.z;
                        s.arrows[slot + 3] = tip.x;
                        s.arrows[slot + 4] = tip.y;
                        s.arrows[slot + 5] = tip.z;
                        arrows += 1;
                    }
                }
                i += stride;
            }
            j += stride;
        }
    }

    s.header.status = 0;
    s.header.mesh_res = n;
    s.header.vert_count = 2 * n * n;
    s.header.index_count = ic as u32;
    s.header.tri_count = (ic / 3) as u32;
    s.header.arrow_count = arrows as u32;
    s.header.sample_count = samples;
    s.header.surface_error = max_residual;
    fill_ptrs(s);
    0
}
