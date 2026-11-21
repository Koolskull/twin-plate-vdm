//! Single-image vector displacement brushes.
//!
//! These are the sculpt-brush kind of VDM, not a closed solid. One OpenEXR
//! stores the vector from the flat plate to the surface. Red, green, and blue
//! are X, Y, and Z in the same units as the plate. Zero is no displacement.
//! Alpha is the mask. A miss stays on the plate and is left out of the mesh.

#[cfg(target_arch = "wasm32")]
use crate::fmath::FloatMath;
use crate::math::Vec3;

pub const COUNT: u32 = 3;

pub fn name(id: u32) -> &'static str {
    match id {
        0 => "horn",
        1 => "crater",
        2 => "ridge",
        _ => "brush",
    }
}

#[allow(dead_code)]
pub fn blurb(id: u32) -> &'static str {
    match id {
        0 => "A horn leans as it rises, so the map has to store sideways displacement, not just height.",
        1 => "A crater drops in the middle and lifts a rim. The vectors point down as well as up.",
        2 => "A ridge overhangs one side. The lip is past the base, which a height map cannot do.",
        _ => "One OpenEXR, read as a vector displacement brush.",
    }
}

/// `x` and `z` are plate coordinates, about -1 to 1. `None` is outside the stamp.
pub fn eval(id: u32, x: f32, z: f32) -> Option<Vec3> {
    match id {
        0 => horn(x, z),
        1 => crater(x, z),
        2 => ridge(x, z),
        _ => None,
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn horn(x: f32, z: f32) -> Option<Vec3> {
    let r = x.hypot(z);
    let edge = 0.78;
    if r >= edge {
        return None;
    }
    let s = smooth(1.0 - r / edge);
    let lift = s * 1.15;
    let lean = s * s * 0.72;
    Some(Vec3::new(lean, lift, z * s * 0.12))
}

fn crater(x: f32, z: f32) -> Option<Vec3> {
    let r = x.hypot(z);
    let edge = 0.9;
    if r >= edge {
        return None;
    }
    let bowl = smooth(1.0 - (r / 0.42).min(1.0));
    let rim_d = (r - 0.55).abs();
    let rim = smooth(1.0 - (rim_d / 0.18).min(1.0));
    let y = rim * 0.28 - bowl * 0.62;
    Some(Vec3::new(0.0, y, 0.0))
}

fn ridge(x: f32, z: f32) -> Option<Vec3> {
    if z.abs() >= 0.72 || x.abs() >= 0.7 {
        return None;
    }
    let across = 1.0 - (x / 0.55).abs();
    if across <= 0.0 {
        return None;
    }
    let along = smooth(1.0 - (z.abs() / 0.72));
    let profile = smooth(across.min(1.0)) * along;
    let y = profile * 0.55;
    // The lip leans to +X, past the footprint of the base.
    let lean = profile * profile * 0.48;
    Some(Vec3::new(lean, y, 0.0))
}
