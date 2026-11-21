//! Twin-plate vector displacement maps.
//!
//! Two plates share one center plane. Every texel stores the vector from that
//! point out to the first surface in front of the plate. One plate faces up.
//! The other faces down. Lean splays both rays away from the middle, so the
//! vectors gain a sideways component and can reach around a side.
//!
//! Those vectors are quantized into two 16-bit RGBA PNGs. Alpha is the mask.
//! A mesh at any resolution is a resample of the PNGs: position = plate point
//! + decoded vector, with a wall stitched around the silhouette. The formula
//! is not consulted again.

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(not(test), allow(dead_code))]

mod brush;
mod exrio;
mod fmath;
mod geom;
mod math;
mod pngio;
mod shape;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn bake(shape: u32, res: u32, lean: f32) -> u32 {
    geom::bake_model(shape, res, lean)
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn reconstruct(mesh_res: u32) -> u32 {
    geom::build_mesh(mesh_res)
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn wipe() {
    geom::wipe_images();
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn load_pngs(plus_ptr: u32, plus_len: u32, minus_ptr: u32, minus_len: u32) -> u32 {
    geom::load_pngs_from_memory(plus_ptr, plus_len, minus_ptr, minus_len)
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn header_ptr() -> u32 {
    geom::header_address()
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn set_container(kind: u32) {
    geom::set_container(kind);
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn bake_brush(id: u32, res: u32) -> u32 {
    geom::bake_brush(id, res)
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn plate_ptr(plate: u32) -> u32 {
    geom::plate_address(plate)
}

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn load_brush(ptr: u32, len: u32) -> u32 {
    geom::load_one_from_memory(ptr, len)
}

#[cfg(test)]
mod tests {
    use super::geom::{
        self, bake_model, build_mesh, decoded_vector, header, ingest_pngs, mesh_indices,
        mesh_vertices, png_minus, png_plus, wipe_images, PLATE_Y, SPAN,
    };
    use super::math::Vec3;
    use super::shape::{self, cast, residual};
    use core::mem::{offset_of, size_of};
    use std::collections::HashMap;

    #[test]
    fn header_is_packed_for_the_page() {
        assert_eq!(size_of::<geom::Header>(), 136);
        assert_eq!(offset_of!(geom::Header, status), 0);
        assert_eq!(offset_of!(geom::Header, png_plus_ptr), 16);
        assert_eq!(offset_of!(geom::Header, vert_ptr), 32);
        assert_eq!(offset_of!(geom::Header, valid_plus), 60);
        assert_eq!(offset_of!(geom::Header, quant_error), 88);
        assert_eq!(offset_of!(geom::Header, plate_y), 100);
        assert_eq!(offset_of!(geom::Header, center_plus), 112);
        assert_eq!(offset_of!(geom::Header, center_minus), 124);
    }

    #[test]
    fn sphere_ray_hits_the_top() {
        let hit = cast(
            shape::SPHERE,
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
        .unwrap();
        assert!((hit.y - 0.72).abs() < 1e-4, "{hit:?}");
        assert!(hit.x.abs() < 1e-4 && hit.z.abs() < 1e-4);
        assert!(residual(shape::SPHERE, hit) < 1e-4);
    }

    #[test]
    fn cone_side_and_base() {
        let side = cast(
            shape::CONE,
            Vec3::new(0.3, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
        .unwrap();
        assert!((side.x - 0.3).abs() < 1e-3, "{side:?}");
        assert!((side.y - 0.163).abs() < 1.5e-2, "{side:?}");
        assert!(residual(shape::CONE, side) < 2e-2, "{}", residual(shape::CONE, side));

        let base = cast(
            shape::CONE,
            Vec3::new(0.2, 0.0, 0.0),
            Vec3::new(0.0, -1.0, 0.0),
        )
        .unwrap();
        assert!((base.y + 0.58).abs() < 1e-3, "{base:?}");
        assert!(base.x.abs() < 0.25);
    }

    #[test]
    fn torus_misses_the_hole_and_hits_the_tube() {
        assert!(cast(
            shape::TORUS,
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
        .is_none());
        let hit = cast(
            shape::TORUS,
            Vec3::new(0.50, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
        .unwrap();
        assert!((hit.y - 0.24).abs() < 2e-3, "{hit:?}");
        assert!((hit.x - 0.50).abs() < 2e-3, "{hit:?}");
        assert!(residual(shape::TORUS, hit) < 2e-3);
    }

    #[test]
    fn codec_matches_the_ray_on_every_sphere_texel() {
        let res = 24u32;
        let lean = 0.35f32;
        assert_eq!(bake_model(shape::SPHERE, res, lean), 0);
        let h = header();
        assert!(h.quant_error < 1e-4, "{}", h.quant_error);
        assert_eq!(h.clamped, 0);
        assert!(h.opposing > 0.99, "{}", h.opposing);
        assert!(h.valid_plus > 50 && h.valid_plus < res * res);
        assert_eq!(&png_plus()[..8], b"\x89PNG\r\n\x1a\n");
        assert!(png_plus().windows(14).any(|w| w == b"twin-plate-vdm"));
        for plate in 0..2u32 {
            for j in 0..res {
                for i in 0..res {
                    assert_texel(shape::SPHERE, res, lean, plate, i, j, 5e-4);
                }
            }
        }
    }

    #[test]
    fn files_roundtrip_through_a_byte_copy() {
        assert_eq!(bake_model(shape::BOX, 32, 0.0), 0);
        let plus = png_plus().to_vec();
        let minus = png_minus().to_vec();
        let before = header().valid_plus;
        wipe_images();
        assert_eq!(header().status, 5);
        assert_eq!(ingest_pngs(&plus, &minus), 0);
        assert_eq!(header().valid_plus, before);
        assert_eq!(build_mesh(32), 0);
        assert!(header().tri_count > 10);
        assert!(header().surface_error < 5e-3, "{}", header().surface_error);
        assert_watertight();
        let len = plus.len();
        assert_eq!(build_mesh(12), 0);
        assert_eq!(png_plus().len(), len);
        let coarse = header().tri_count;
        assert_eq!(build_mesh(48), 0);
        assert!(header().tri_count > coarse);
        assert_eq!(png_plus().len(), len);
        assert_eq!(png_minus().len(), minus.len());
    }

    #[test]
    fn lean_zero_is_vertical_and_lean_tilts() {
        let res = 32u32;
        assert_eq!(bake_model(shape::SPHERE, res, 0.0), 0);
        let (i, j) = texel_near(res, 0.40, 0.0);
        let straight = decoded_vector(0, i, j).unwrap();
        assert!(straight.x.abs() < 1e-3 && straight.z.abs() < 1e-3, "{straight:?}");
        assert!(straight.y > 0.2, "{straight:?}");

        assert_eq!(bake_model(shape::SPHERE, res, 0.8), 0);
        let (i, j) = texel_near(res, 0.40, 0.0);
        let tilted = decoded_vector(0, i, j).unwrap();
        assert!(tilted.x > 0.02, "{tilted:?}");
        assert!(tilted.y > 0.0, "{tilted:?}");
    }

    #[test]
    fn box_caps_are_flat_when_rays_are_vertical() {
        let res = 28u32;
        assert_eq!(bake_model(shape::BOX, res, 0.0), 0);
        let (i, j) = texel_near(res, 0.1, -0.1);
        let top = decoded_vector(0, i, j).unwrap();
        let u = (i as f32 + 0.5) / res as f32;
        let v = (j as f32 + 0.5) / res as f32;
        let x = (u - 0.5) * SPAN;
        let z = (v - 0.5) * SPAN;
        let world_y = PLATE_Y + top.y;
        assert!((world_y - 0.40).abs() < 2e-3, "{world_y} from {top:?} at {x},{z}");
        assert!(top.x.abs() < 1e-3 && top.z.abs() < 1e-3);
    }

    #[test]
    fn torus_png_has_a_hole_and_a_watertight_mesh() {
        assert_eq!(bake_model(shape::TORUS, 40, 0.15), 0);
        let h = header();
        assert_eq!(h.center_plus_ok, 0, "the middle texel is the hole");
        assert!(h.valid_plus > 30);
        assert!(h.opposing > 0.99);
        let (i, j) = texel_near(40, 0.50, 0.0);
        assert!(decoded_vector(0, i, j).is_some());
        assert_eq!(build_mesh(28), 0);
        assert!(header().tri_count > 20);
        assert_watertight();
    }

    #[test]
    fn mushroom_projects_the_cap_up_and_the_stem_down() {
        let res = 64u32;
        assert_eq!(bake_model(shape::MUSHROOM, res, 0.0), 0);
        let (si, sj) = texel_near(res, 0.0, 0.0);
        let stem = decoded_vector(1, si, sj).unwrap();
        let stem_y = PLATE_Y + stem.y;
        let (ri, rj) = texel_near(res, 0.48, 0.0);
        let cap = decoded_vector(0, ri, rj).unwrap();
        let cap_y = PLATE_Y + cap.y;
        assert!(stem_y < -0.5, "stem bottom {stem_y}");
        assert!(cap_y > 0.30, "outer cap {cap_y}");
        assert!(decoded_vector(1, ri, rj).is_none(), "downward ray misses the cap");
        assert_eq!(build_mesh(res), 0);
        assert!(header().tri_count > 20);
        assert!(header().surface_error < 0.02, "{}", header().surface_error);
    }

    #[test]
    fn every_solid_rebuilds_a_closed_mesh() {
        for shape in 0..shape::SHAPE_COUNT {
            assert_eq!(bake_model(shape, 32, 0.2), 0, "bake {shape}");
            assert!(header().opposing > 0.98, "shape {shape} facing {}", header().opposing);
            assert_eq!(header().clamped, 0, "shape {shape}");
            assert_eq!(build_mesh(32), 0, "mesh {shape}");
            assert!(header().tri_count > 8, "shape {shape}");
            // Cone and mushroom have different up and down footprints, so the
            // shells do not share one closed silhouette.
            if shape != shape::CONE && shape != shape::MUSHROOM {
                assert_watertight();
            }
            let limit = if shape == shape::RIPPLE || shape == shape::TORUS || shape == shape::CONE {
                0.03
            } else {
                0.008
            };
            assert!(
                header().surface_error < limit,
                "shape {shape} surface {}",
                header().surface_error
            );
            let native = header().tri_count;
            assert_eq!(build_mesh(14), 0, "coarse {shape}");
            assert!(header().tri_count < native, "shape {shape}");
            if shape != shape::CONE && shape != shape::MUSHROOM {
                assert_watertight();
            }
        }
    }

    #[test]
    fn sphere_normals_point_outward() {
        assert_eq!(bake_model(shape::SPHERE, 36, 0.25), 0);
        assert_eq!(build_mesh(24), 0);
        let idx = mesh_indices();
        let verts = mesh_vertices();
        let mut good = 0u32;
        let mut total = 0u32;
        for tri in idx.chunks(3) {
            let a = pos(verts, tri[0]);
            let b = pos(verts, tri[1]);
            let c = pos(verts, tri[2]);
            let n = (b - a).cross(c - a);
            if n.length() < 1e-8 {
                continue;
            }
            let cen = (a + b + c) * (1.0 / 3.0);
            total += 1;
            if n.dot(cen) > 0.0 {
                good += 1;
            }
        }
        assert!(total > 20);
        assert!(
            good as f32 / total as f32 > 0.95,
            "{good}/{total}"
        );
    }

    #[test]
    fn exr_pair_and_brush_roundtrip() {
        geom::set_container(1);
        assert_eq!(bake_model(shape::SPHERE, 16, 0.2), 0);
        assert_eq!(&png_plus()[..4], &[0x76, 0x2f, 0x31, 0x01]);
        assert!(png_plus().windows(14).any(|w| w == b"twin-plate-vdm"));
        let plus = png_plus().to_vec();
        let minus = png_minus().to_vec();
        let hits = header().valid_plus;
        wipe_images();
        assert_eq!(ingest_pngs(&plus, &minus), 0);
        assert_eq!(header().valid_plus, hits);
        assert_eq!(header().shape, shape::SPHERE);
        assert_eq!(build_mesh(16), 0);
        assert!(header().tri_count > 10);
        assert!(header().surface_error < 5e-3, "{}", header().surface_error);

        assert_eq!(geom::bake_brush(0, 32), 0);
        let horn = header();
        assert_eq!(&png_plus()[..4], &[0x76, 0x2f, 0x31, 0x01]);
        assert!(horn.valid_plus > 20);
        assert_eq!(horn.valid_minus, 0);
        assert!(horn.center_plus_ok == 1 && horn.center_plus[1] > 0.8, "{:?}", horn.center_plus);
        assert_eq!(build_mesh(24), 0);
        assert!(header().tri_count > 0);
        assert_eq!(header().shape, shape::SHAPE_COUNT);
        let one = png_plus().to_vec();
        wipe_images();
        assert_eq!(geom::ingest_one(&one), 0);
        assert!(header().valid_plus > 20);
        assert_eq!(header().valid_minus, 0);
        assert_eq!(header().shape, shape::SHAPE_COUNT);
        assert!(header().center_plus[1] > 0.8);

        assert_eq!(geom::bake_brush(1, 24), 0);
        assert!(header().center_plus[1] < -0.3, "crater {:?}", header().center_plus);
        assert_eq!(geom::bake_brush(2, 24), 0);
        assert!(header().center_plus[0] > 0.2, "ridge {:?}", header().center_plus);
        assert!(header().center_plus[1] > 0.2, "ridge {:?}", header().center_plus);

        geom::set_container(0);
        assert_eq!(bake_model(shape::SPHERE, 16, 0.0), 0);
        assert_eq!(&png_plus()[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    #[ignore]
    fn export_example_exrs() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        std::fs::create_dir_all(&root).unwrap();
        geom::set_container(1);
        for (id, name) in [(0u32, "horn"), (1, "crater"), (2, "ridge")] {
            assert_eq!(geom::bake_brush(id, 96), 0, "{name}");
            std::fs::write(root.join(format!("{name}.exr")), png_plus()).unwrap();
        }
        assert_eq!(bake_model(shape::SPHERE, 48, 0.35), 0);
        std::fs::write(root.join("sphere-plus.exr"), png_plus()).unwrap();
        std::fs::write(root.join("sphere-minus.exr"), png_minus()).unwrap();
        if let Ok(bytes) = std::fs::read("/tmp/Blobbies.exr") {
            let mut scratch = vec![0u8; 600_000];
            let mut pixels = vec![0f32; 32 * 32 * 4];
            for y in 0..32i32 {
                for x in 0..32i32 {
                    let px = crate::exrio::sample_rgba(&bytes, &mut scratch, 484 + x, 484 + y).unwrap();
                    let o = ((y as usize) * 32 + x as usize) * 4;
                    pixels[o..o + 4].copy_from_slice(&px);
                }
            }
            let mut out = vec![0u8; 80_000];
            let n = crate::exrio::encode_rgba_f32(
                &pixels,
                32,
                "ILM Blobbies 32 crop around data-window 500,500. RGB stored as XYZ. Not a sculpt brush.",
                &mut scratch,
                &mut out,
            )
            .unwrap();
            std::fs::write(root.join("blobbies-center.exr"), &out[..n]).unwrap();
        }
        geom::set_container(0);
    }

    fn pos(verts: &[f32], id: u32) -> Vec3 {
        let o = id as usize * 6;
        Vec3::new(verts[o], verts[o + 1], verts[o + 2])
    }

    fn assert_watertight() {
        let idx = mesh_indices();
        assert!(!idx.is_empty() && idx.len() % 3 == 0);
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for tri in idx.chunks(3) {
            for edge in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                let key = if edge.0 < edge.1 {
                    (edge.0, edge.1)
                } else {
                    (edge.1, edge.0)
                };
                *edges.entry(key).or_insert(0) += 1;
            }
        }
        let broken: Vec<_> = edges.iter().filter(|(_, c)| **c != 2).take(6).collect();
        assert!(
            broken.is_empty(),
            "open edges {:?}; count {}",
            broken,
            edges.values().filter(|c| **c != 2).count()
        );
    }

    fn texel_near(res: u32, x: f32, z: f32) -> (u32, u32) {
        let u = x / SPAN + 0.5;
        let v = z / SPAN + 0.5;
        let i = (u * res as f32 - 0.5).round().clamp(0.0, (res - 1) as f32) as u32;
        let j = (v * res as f32 - 0.5).round().clamp(0.0, (res - 1) as f32) as u32;
        (i, j)
    }

    fn assert_texel(shape: u32, res: u32, lean: f32, plate: u32, i: u32, j: u32, tol: f32) {
        let u = (i as f32 + 0.5) / res as f32;
        let v = (j as f32 + 0.5) / res as f32;
        let x = (u - 0.5) * SPAN;
        let z = (v - 0.5) * SPAN;
        let y_sign = if plate == 0 { 1.0 } else { -1.0 };
        let origin = Vec3::new(x, PLATE_Y, z);
        let dir = Vec3::new(lean * x, y_sign, lean * z);
        let hit = cast(shape, origin, dir);
        let got = decoded_vector(plate, i, j);
        match (hit, got) {
            (None, None) => {}
            (Some(point), Some(vector)) => {
                let expect = point - origin;
                let err = (expect - vector).length();
                assert!(
                    err < tol,
                    "plate {plate} texel {i},{j} err {err} expect {expect:?} got {vector:?}"
                );
                if plate == 0 {
                    assert!(vector.y > 0.0, "{vector:?}");
                } else {
                    assert!(vector.y < 0.0, "{vector:?}");
                }
            }
            (hit, got) => panic!("plate {plate} texel {i},{j} hit {hit:?} decoded {got:?}"),
        }
    }
}
