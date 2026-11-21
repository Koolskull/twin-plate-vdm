//! Analytic solids. Each shape is a formula. The baker casts a ray from the
//! shared center plate and keeps the outer surface in that direction: the
//! point where the ray leaves the solid.

#[cfg(target_arch = "wasm32")]
use crate::fmath::FloatMath;
use crate::math::{Vec2, Vec3};

pub const SHAPE_COUNT: u32 = 7;
pub const SPHERE: u32 = 0;
pub const BOX: u32 = 1;
pub const TORUS: u32 = 2;
pub const CONE: u32 = 3;
pub const CAPSULE: u32 = 4;
pub const MUSHROOM: u32 = 5;
pub const RIPPLE: u32 = 6;

pub fn shape_name(id: u32) -> &'static str {
    match id {
        SPHERE => "sphere",
        BOX => "box",
        TORUS => "torus",
        CONE => "cone",
        CAPSULE => "capsule",
        MUSHROOM => "mushroom",
        RIPPLE => "ripple",
        _ => "unknown",
    }
}

const SPHERE_R: f32 = 0.72;

const BOX_H: Vec3 = Vec3::new(0.66, 0.40, 0.50);

const TORUS_R: f32 = 0.50;
const TORUS_MINOR: f32 = 0.24;

/// Finite cone. Base sits low, apex points up. Radius falls linearly to zero.
const CONE_Y0: f32 = -0.58;
const CONE_Y1: f32 = 0.75;
const CONE_RADIUS: f32 = 0.68;

const CAP_H: f32 = 0.40;
const CAP_R: f32 = 0.28;

const CAP_CENTER: Vec3 = Vec3::new(0.0, 0.28, 0.0);
const CAP_RADII: Vec3 = Vec3::new(0.70, 0.22, 0.70);
const STEM_R: f32 = 0.16;
const STEM_Y0: f32 = -0.78;
const STEM_Y1: f32 = 0.28;

const RIPPLE_BASE: f32 = 0.56;
const RIPPLE_AMP: f32 = 0.055;

pub fn cast(shape: u32, origin: Vec3, dir: Vec3) -> Option<Vec3> {
    let dir = dir.normalized();
    let hit_t = match shape {
        SPHERE => ray_sphere(origin, dir, SPHERE_R),
        BOX => ray_box(origin, dir, BOX_H),
        TORUS => return march(shape, origin, dir),
        CONE => ray_cone(origin, dir),
        CAPSULE => ray_capsule(origin, dir),
        MUSHROOM => ray_mushroom(origin, dir),
        RIPPLE => return march(shape, origin, dir),
        _ => None,
    };
    hit_t.map(|t| origin + dir * t)
}

/// World-space distance to the surface, positive outside. Used to grade a
/// rebuilt vertex, not to invent one.
pub fn residual(shape: u32, p: Vec3) -> f32 {
    match shape {
        SPHERE => (p.length() - SPHERE_R).abs(),
        BOX => sd_box(p, BOX_H).abs(),
        TORUS => sd_torus(p).abs(),
        CONE => sd_cone(p).abs(),
        CAPSULE => (segment_dist_y(p, -CAP_H, CAP_H) - CAP_R).abs(),
        MUSHROOM => sd_ellipsoid(p - CAP_CENTER, CAP_RADII)
            .min(sd_cylinder_y(p, STEM_Y0, STEM_Y1, STEM_R))
            .abs(),
        RIPPLE => (p.length() - ripple_radius(p)).abs(),
        _ => 1.0,
    }
}

fn outside(shape: u32, p: Vec3) -> bool {
    match shape {
        SPHERE => p.length() > SPHERE_R,
        BOX => p.x.abs() > BOX_H.x || p.y.abs() > BOX_H.y || p.z.abs() > BOX_H.z,
        TORUS => sd_torus(p) > 0.0,
        CONE => !inside_cone(p),
        CAPSULE => segment_dist_y(p, -CAP_H, CAP_H) > CAP_R,
        MUSHROOM => {
            !inside_ellipsoid(p, CAP_CENTER, CAP_RADII) && !inside_cylinder_y(p, STEM_Y0, STEM_Y1, STEM_R)
        }
        RIPPLE => p.length() > ripple_radius(p),
        _ => true,
    }
}

fn ripple_radius(p: Vec3) -> f32 {
    let len = p.length().max(1e-6);
    let n = p * (1.0 / len);
    RIPPLE_BASE + RIPPLE_AMP * (n.x * 5.0).sin() * (n.y * 4.0).sin() * (n.z * 5.0).sin()
}

fn march(shape: u32, origin: Vec3, dir: Vec3) -> Option<Vec3> {
    if !outside(shape, origin) {
        return march_to_exit(shape, origin, dir);
    }
    let mut t = 0.0f32;
    let mut last_step = 0.02f32;
    let max_step = if shape == RIPPLE { 0.02 } else { 0.35 };
    for _ in 0..420 {
        let p = origin + dir * t;
        let gap = match shape {
            RIPPLE => p.length() - ripple_radius(p),
            TORUS => sd_torus(p),
            _ => residual(shape, p),
        };
        let entered = gap < 0.0 || !outside(shape, p);
        if entered || (shape == TORUS && gap < 1.2e-3) {
            let mut lo = (t - last_step).max(0.0);
            let mut hi = t;
            if outside(shape, origin + dir * hi) {
                hi = (t + gap.abs() + 2e-3).min(4.0);
            }
            if outside(shape, origin + dir * hi) {
                // Close beside the ray, not across it. A hit at the origin
                // would be a zero vector, which pulls the shell back to the plate.
                if !entered {
                    let step = gap.clamp(0.002, max_step);
                    last_step = step;
                    t += step;
                    if t > 4.2 {
                        return None;
                    }
                    continue;
                }
                return Some(origin + dir * t);
            }
            for _ in 0..28 {
                let mid = 0.5 * (lo + hi);
                if outside(shape, origin + dir * mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            // hi is just inside. Walk to where the ray leaves, so a shell
            // stores its outer surface rather than the face toward the plate.
            return march_to_exit(shape, origin + dir * hi, dir);
        }
        let step = if shape == RIPPLE {
            (gap * 0.45).clamp(0.003, max_step)
        } else {
            gap.clamp(0.002, max_step)
        };
        last_step = step;
        t += step;
        if t > 4.2 {
            return None;
        }
    }
    None
}

fn march_to_exit(shape: u32, origin: Vec3, dir: Vec3) -> Option<Vec3> {
    let max_step = if shape == RIPPLE { 0.02 } else { 0.08 };
    let mut t = 0.0f32;
    for _ in 0..480 {
        let p = origin + dir * t;
        let gap = match shape {
            RIPPLE => ripple_radius(p) - p.length(),
            TORUS => -sd_torus(p),
            _ => 0.02,
        };
        let step = if gap > 0.0 {
            gap.clamp(0.002, max_step)
        } else {
            0.002
        };
        let next = t + step;
        if outside(shape, origin + dir * next) {
            let mut lo = t;
            let mut hi = next;
            for _ in 0..28 {
                let mid = 0.5 * (lo + hi);
                if outside(shape, origin + dir * mid) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return Some(origin + dir * hi);
        }
        t = next;
        if t > 4.2 {
            return None;
        }
    }
    None
}

/// Farthest t where the ray leaves the solid. An inside start is the exit.
/// An outside start passes through and keeps the outer surface.
fn last_exit(shape: u32, origin: Vec3, dir: Vec3, cands: &[f32]) -> Option<f32> {
    let mut best: Option<f32> = None;
    for &t in cands {
        if !t.is_finite() || t < 1e-4 {
            continue;
        }
        let behind = origin + dir * (t - 1.5e-3);
        let ahead = origin + dir * (t + 1.5e-3);
        if outside(shape, behind) || !outside(shape, ahead) {
            continue;
        }
        best = Some(match best {
            Some(b) if b > t => b,
            _ => t,
        });
    }
    best
}

fn ray_sphere(o: Vec3, d: Vec3, r: f32) -> Option<f32> {
    let b = o.dot(d);
    let c = o.dot(o) - r * r;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    last_exit(SPHERE, o, d, &[-b - s, -b + s])
}

fn ray_box(o: Vec3, d: Vec3, he: Vec3) -> Option<f32> {
    let mut tmin = 0.0f32;
    let mut tmax = 1.0e9f32;
    let origins = [o.x, o.y, o.z];
    let dirs = [d.x, d.y, d.z];
    let bounds = [he.x, he.y, he.z];
    for axis in 0..3 {
        if dirs[axis].abs() < 1e-8 {
            if origins[axis].abs() > bounds[axis] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dirs[axis];
        let mut t1 = (-bounds[axis] - origins[axis]) * inv;
        let mut t2 = (bounds[axis] - origins[axis]) * inv;
        if t1 > t2 {
            core::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return None;
        }
    }
    if tmax < 1e-4 {
        return None;
    }
    last_exit(BOX, o, d, &[tmin, tmax])
}

fn ray_cone(o: Vec3, d: Vec3) -> Option<f32> {
    let mut cands = [f32::NAN; 4];
    let mut n = 0;
    let height = CONE_Y1 - CONE_Y0;
    let k = (CONE_RADIUS / height) * (CONE_RADIUS / height);
    let a = d.x * d.x + d.z * d.z - k * d.y * d.y;
    let b = 2.0 * (o.x * d.x + o.z * d.z) + 2.0 * k * (CONE_Y1 - o.y) * d.y;
    let c = o.x * o.x + o.z * o.z - k * (CONE_Y1 - o.y) * (CONE_Y1 - o.y);
    let mut roots = [0.0f32; 2];
    let count = quad_roots(a, b, c, &mut roots);
    for i in 0..count {
        let t = roots[i];
        if !t.is_finite() || t < 1e-4 {
            continue;
        }
        let y = o.y + t * d.y;
        if y >= CONE_Y0 - 1e-4 && y <= CONE_Y1 + 1e-4 {
            cands[n] = t;
            n += 1;
        }
    }
    if d.y.abs() > 1e-8 {
        let t = (CONE_Y0 - o.y) / d.y;
        if t > 1e-4 {
            let p = o + d * t;
            if p.x * p.x + p.z * p.z <= CONE_RADIUS * CONE_RADIUS + 1e-5 {
                cands[n] = t;
                n += 1;
            }
        }
    }
    last_exit(CONE, o, d, &cands[..n])
}

fn ray_capsule(o: Vec3, d: Vec3) -> Option<f32> {
    let mut cands = [f32::NAN; 6];
    let mut n = 0;
    push_cylinder_wall(&mut cands, &mut n, o, d, -CAP_H, CAP_H, CAP_R);
    push_sphere(&mut cands, &mut n, o, d, Vec3::new(0.0, -CAP_H, 0.0), CAP_R);
    push_sphere(&mut cands, &mut n, o, d, Vec3::new(0.0, CAP_H, 0.0), CAP_R);
    last_exit(CAPSULE, o, d, &cands[..n])
}

fn ray_mushroom(o: Vec3, d: Vec3) -> Option<f32> {
    let mut cands = [f32::NAN; 8];
    let mut n = 0;
    push_ellipsoid(&mut cands, &mut n, o, d, CAP_CENTER, CAP_RADII);
    push_cylinder_wall(&mut cands, &mut n, o, d, STEM_Y0, STEM_Y1, STEM_R);
    push_disk(&mut cands, &mut n, o, d, STEM_Y0, STEM_R);
    push_disk(&mut cands, &mut n, o, d, STEM_Y1, STEM_R);
    last_exit(MUSHROOM, o, d, &cands[..n])
}

fn push_sphere(out: &mut [f32], n: &mut usize, o: Vec3, d: Vec3, c: Vec3, r: f32) {
    let oc = o - c;
    let b = oc.dot(d);
    let cc = oc.dot(oc) - r * r;
    let disc = b * b - cc;
    if disc < 0.0 {
        return;
    }
    let s = disc.sqrt();
    push_cand(out, n, -b - s);
    push_cand(out, n, -b + s);
}

fn push_ellipsoid(out: &mut [f32], n: &mut usize, o: Vec3, d: Vec3, c: Vec3, r: Vec3) {
    let oc = div_comp(o - c, r);
    let dd = div_comp(d, r);
    let a = dd.dot(dd);
    let b = 2.0 * oc.dot(dd);
    let cc = oc.dot(oc) - 1.0;
    let mut roots = [0.0f32; 2];
    let count = quad_roots(a, b, cc, &mut roots);
    for i in 0..count {
        push_cand(out, n, roots[i]);
    }
}

fn push_cylinder_wall(out: &mut [f32], n: &mut usize, o: Vec3, d: Vec3, y0: f32, y1: f32, rad: f32) {
    let a = d.x * d.x + d.z * d.z;
    let b = 2.0 * (o.x * d.x + o.z * d.z);
    let c = o.x * o.x + o.z * o.z - rad * rad;
    let mut roots = [0.0f32; 2];
    let count = quad_roots(a, b, c, &mut roots);
    for i in 0..count {
        let t = roots[i];
        if t.is_finite() && t > 1e-4 {
            let y = o.y + t * d.y;
            if y >= y0 - 1e-4 && y <= y1 + 1e-4 {
                push_cand(out, n, t);
            }
        }
    }
}

fn push_disk(out: &mut [f32], n: &mut usize, o: Vec3, d: Vec3, y: f32, rad: f32) {
    if d.y.abs() < 1e-8 {
        return;
    }
    let t = (y - o.y) / d.y;
    if t < 1e-4 {
        return;
    }
    let p = o + d * t;
    if p.x * p.x + p.z * p.z <= rad * rad + 1e-5 {
        push_cand(out, n, t);
    }
}

fn push_cand(out: &mut [f32], n: &mut usize, t: f32) {
    if *n < out.len() && t.is_finite() {
        out[*n] = t;
        *n += 1;
    }
}

fn quad_roots(a: f32, b: f32, c: f32, out: &mut [f32; 2]) -> usize {
    if a.abs() < 1e-10 {
        if b.abs() < 1e-10 {
            return 0;
        }
        out[0] = -c / b;
        return 1;
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return 0;
    }
    let s = disc.sqrt();
    let sign_b = if b >= 0.0 { 1.0 } else { -1.0 };
    let q = -0.5 * (b + sign_b * s);
    if q.abs() < 1e-12 {
        out[0] = -b / (2.0 * a);
        return 1;
    }
    out[0] = q / a;
    out[1] = c / q;
    2
}

fn div_comp(p: Vec3, r: Vec3) -> Vec3 {
    Vec3::new(p.x / r.x, p.y / r.y, p.z / r.z)
}

fn segment_dist_y(p: Vec3, y0: f32, y1: f32) -> f32 {
    let y = p.y.clamp(y0, y1);
    Vec3::new(p.x, p.y - y, p.z).length()
}

fn inside_cone(p: Vec3) -> bool {
    if p.y < CONE_Y0 || p.y > CONE_Y1 {
        return false;
    }
    let t = (CONE_Y1 - p.y) / (CONE_Y1 - CONE_Y0);
    let radius = CONE_RADIUS * t;
    p.x * p.x + p.z * p.z <= radius * radius
}

fn inside_ellipsoid(p: Vec3, c: Vec3, r: Vec3) -> bool {
    let q = div_comp(p - c, r);
    q.dot(q) <= 1.0
}

fn inside_cylinder_y(p: Vec3, y0: f32, y1: f32, rad: f32) -> bool {
    p.y >= y0 && p.y <= y1 && p.x * p.x + p.z * p.z <= rad * rad
}

fn sd_box(p: Vec3, b: Vec3) -> f32 {
    let q = p.abs() - b;
    let outside = Vec3::new(q.x.max(0.0), q.y.max(0.0), q.z.max(0.0)).length();
    let inside = q.x.max(q.y.max(q.z)).min(0.0);
    outside + inside
}

fn sd_torus(p: Vec3) -> f32 {
    let q = Vec2::new((p.x * p.x + p.z * p.z).sqrt() - TORUS_R, p.y);
    q.length() - TORUS_MINOR
}

fn sd_ellipsoid(p: Vec3, r: Vec3) -> f32 {
    let k0 = div_comp(p, r).length();
    let k1 = div_comp(p, Vec3::new(r.x * r.x, r.y * r.y, r.z * r.z)).length();
    if k1 < 1e-8 {
        return -r.x.min(r.y.min(r.z));
    }
    k0 * (k0 - 1.0) / k1
}

fn sd_cylinder_y(p: Vec3, y0: f32, y1: f32, rad: f32) -> f32 {
    let half = (y1 - y0) * 0.5;
    let mid = (y1 + y0) * 0.5;
    let d = Vec2::new((p.x * p.x + p.z * p.z).sqrt(), (p.y - mid).abs()) - Vec2::new(rad, half);
    d.x.max(0.0).hypot(d.y.max(0.0)) + d.x.max(d.y).min(0.0)
}

/// Capped cone centered on the origin, radius `r1` at y = -h and `r2` at y = +h.
fn sd_capped_cone(p: Vec3, h: f32, r1: f32, r2: f32) -> f32 {
    let q = Vec2::new((p.x * p.x + p.z * p.z).sqrt(), p.y);
    let k1 = Vec2::new(r2, h);
    let k2 = Vec2::new(r2 - r1, 2.0 * h);
    let ca = Vec2::new(
        q.x - q.x.min(if q.y < 0.0 { r1 } else { r2 }),
        q.y.abs() - h,
    );
    let denom = k2.dot(k2).max(1e-8);
    let t = ((k1 - q).dot(k2) / denom).clamp(0.0, 1.0);
    let cb = q - k1 + k2 * t;
    let s = if cb.x < 0.0 && ca.y < 0.0 { -1.0 } else { 1.0 };
    s * ca.dot(ca).min(cb.dot(cb)).sqrt()
}

fn sd_cone(p: Vec3) -> f32 {
    let h = (CONE_Y1 - CONE_Y0) * 0.5;
    let mid = (CONE_Y1 + CONE_Y0) * 0.5;
    sd_capped_cone(Vec3::new(p.x, p.y - mid, p.z), h, CONE_RADIUS, 0.0)
}
