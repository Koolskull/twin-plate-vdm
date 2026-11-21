//! Float helpers that exist on `f32` only when `std` is linked.
//!
//! Wasm builds are `no_std`. Homebrew rustc also cannot load the upstream
//! wasm `std`, so the baker carries these operations itself. Host tests keep
//! using the standard library methods; this module is checked against them.

#[cfg(target_arch = "wasm32")]
pub trait FloatMath {
    fn sqrt(self) -> Self;
    fn sin(self) -> Self;
    fn floor(self) -> Self;
    fn round(self) -> Self;
    fn hypot(self, other: Self) -> Self;
}

#[cfg(target_arch = "wasm32")]
impl FloatMath for f32 {
    fn sqrt(self) -> Self {
        sqrt(self)
    }
    fn sin(self) -> Self {
        sin(self)
    }
    fn floor(self) -> Self {
        floor(self)
    }
    fn round(self) -> Self {
        round(self)
    }
    fn hypot(self, other: Self) -> Self {
        hypot(self, other)
    }
}

pub fn floor(x: f32) -> f32 {
    if !(x.is_finite()) || x.abs() >= 16_777_216.0 {
        return x;
    }
    let truncated = x as i32 as f32;
    if x >= 0.0 || truncated == x {
        truncated
    } else {
        truncated - 1.0
    }
}

pub fn round(x: f32) -> f32 {
    if x >= 0.0 {
        floor(x + 0.5)
    } else {
        -floor(-x + 0.5)
    }
}

pub fn sqrt(x: f32) -> f32 {
    if x < 0.0 {
        return f32::NAN;
    }
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1fc0_0000);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y
}

pub fn hypot(x: f32, y: f32) -> f32 {
    sqrt(x * x + y * y)
}

pub fn sin(x: f32) -> f32 {
    if !x.is_finite() {
        return f32::NAN;
    }
    const PI: f32 = 3.141_592_7;
    const TAU: f32 = 6.283_185_5;
    let mut a = x - TAU * round(x / TAU);
    if a > PI {
        a -= TAU;
    } else if a < -PI {
        a += TAU;
    }
    let neg = a < 0.0;
    if neg {
        a = -a;
    }
    if a > PI * 0.5 {
        a = PI - a;
    }
    let z = a * a;
    let s = a
        * (1.0
            + z * (-1.666_666_7e-1
                + z * (8.333_333e-3
                    + z * (-1.984_127e-4
                        + z * (2.755_731_9e-6 + z * (-2.505_210_8e-8 + z * 1.605_904_4e-10))))));
    if neg { -s } else { s }
}

#[cfg(test)]
mod tests {
    use super::{floor, hypot, round, sin, sqrt};

    fn close(got: f32, want: f32) -> bool {
        if want.is_nan() {
            return got.is_nan();
        }
        (got - want).abs() <= 2e-6 * want.abs().max(1.0)
    }

    #[test]
    fn floor_and_round_match_std() {
        let mut x = -8.0_f32;
        while x <= 8.0 {
            assert!(close(floor(x), x.floor()), "floor {x}");
            assert!(close(round(x), x.round()), "round {x}");
            x += 0.05;
        }
        for x in [0.0, 0.5, -0.5, 1.5, -1.5, 65534.4, 65534.6, 65535.0] {
            assert!(close(floor(x), x.floor()), "floor {x}");
            assert!(close(round(x), x.round()), "round {x}");
        }
    }

    #[test]
    fn sqrt_and_hypot_match_std() {
        for i in 0..200 {
            let x = (i as f32) * (i as f32) * 0.01;
            assert!(close(sqrt(x), x.sqrt()), "sqrt {x} {} {}", sqrt(x), x.sqrt());
        }
        assert!(sqrt(-1.0).is_nan());
        assert!(close(hypot(3.0, 4.0), 5.0));
        assert!(close(hypot(-1.5, 2.5), (-1.5_f32).hypot(2.5)));
    }

    #[test]
    fn sin_matches_std_on_the_ripple_domain() {
        let mut x = -8.0_f32;
        while x <= 8.0 {
            assert!(close(sin(x), x.sin()), "sin {x} {} {}", sin(x), x.sin());
            x += 0.02;
        }
    }
}
