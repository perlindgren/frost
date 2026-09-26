//! The WGSL shader sources, embedded at compile time from the `.wgsl` files
//! in the crate's `shaders/` directory.

/// The line shader source.
pub(crate) const LINE_SHADER: &str = include_str!("../shaders/line.wgsl");

/// The polyline (batched line) shader source.
pub(crate) const POLYLINE_SHADER: &str = include_str!("../shaders/polyline.wgsl");

/// The circle shader source.
pub(crate) const CIRCLE_SHADER: &str = include_str!("../shaders/circle.wgsl");

/// The rectangle shader source.
pub(crate) const RECT_SHADER: &str = include_str!("../shaders/rectangle.wgsl");

/// The shape (scene circle/rectangle) shader source.
pub(crate) const SHAPE_SHADER: &str = include_str!("../shaders/shape.wgsl");

/// The sprite (texture) shader source.
pub(crate) const SPRITE_SHADER: &str = include_str!("../shaders/sprite.wgsl");

/// The batched particle shader source.
pub(crate) const PARTICLES_SHADER: &str = include_str!("../shaders/particles.wgsl");

#[cfg(test)]
mod tests {
    //! Shader validation without running anything: naga's WGSL frontend and
    //! validator are the two stages wgpu executes inside
    //! `Device::create_shader_module` — parse, then validate — so a shader
    //! that fails either stage here fails at runtime there.

    use super::*;
    use crate::backend::{OCCLUDER_FIELD_BUFFER_MIN, LIGHT_FIELD_BUFFER_MIN};

    /// The seven shader sources with their file names.
    fn all_shaders() -> [(&'static str, &'static str); 7] {
        [
            ("line.wgsl", LINE_SHADER),
            ("polyline.wgsl", POLYLINE_SHADER),
            ("circle.wgsl", CIRCLE_SHADER),
            ("rectangle.wgsl", RECT_SHADER),
            ("shape.wgsl", SHAPE_SHADER),
            ("sprite.wgsl", SPRITE_SHADER),
            ("particles.wgsl", PARTICLES_SHADER),
        ]
    }

    /// All seven shaders parse as valid WGSL.
    #[test]
    fn all_shaders_parse_as_wgsl() {
        for (name, source) in all_shaders() {
            naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|err| panic!("{name} is not valid WGSL: {err}"));
        }
    }

    /// All seven shaders must pass naga's *validator*, not just its parser.
    /// Parsing only proves the WGSL is syntactically valid; the validator is
    /// the second stage wgpu runs inside `Device::create_shader_module`, and
    /// it enforces the address-space layout rules the parser never checks —
    /// e.g., in the uniform space an array member's stride must be a
    /// multiple of 16 bytes. The polyline's original
    /// `array<vec2<f32>, 256>` parsed fine but was rejected by the device
    /// for exactly this reason, after the parse-only test had passed.
    #[test]
    fn all_shaders_pass_device_side_validation() {
        for (name, source) in all_shaders() {
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|err| panic!("{name} is not valid WGSL: {err}"));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|err| panic!("{name} fails device-side validation: {err}"));
        }
    }

    /// The member offsets WGSL assigns to `ShapeUniforms` must match the
    /// CPU-side uniform writer in `shape_uniform_data` (mat2x2 @0, vec2 @16,
    /// vec2 @24, vec2 @32, vec4 @48 — 16 bytes, 16-byte aligned — vec2 @64,
    /// f32 @72, vec4 @80; 96 bytes total). This is the GPU-side mirror of
    /// `shape_uniform_bytes_follow_the_wgsl_layout` in backend.rs, so a layout
    /// drift on either side fails a test.
    #[test]
    fn shape_uniform_offsets_match_the_cpu_layout() {
        let module = naga::front::wgsl::parse_str(SHAPE_SHADER)
            .expect("shape.wgsl should parse (see all_shaders_parse_as_wgsl)");
        let ty = module
            .types
            .iter()
            .find_map(|(_, ty)| match &ty.inner {
                naga::TypeInner::Struct { .. } if ty.name.as_deref() == Some("ShapeUniforms") => {
                    Some(ty)
                }
                _ => None,
            })
            .expect("shape.wgsl should declare the ShapeUniforms struct");
        let (offsets, total_size) = match &ty.inner {
            naga::TypeInner::Struct { members, span } => (
                members.iter().map(|m| m.offset).collect::<Vec<u32>>(),
                *span,
            ),
            _ => unreachable!("ShapeUniforms must be a struct"),
        };
        // `lit` rides in the 4 bytes the struct already padded after `misc`;
        // the 16-byte-aligned `glow` vec4 follows at 80.
        assert_eq!(offsets, [0, 16, 24, 32, 48, 64, 72, 80]);
        // The struct's total size must equal the CPU-side buffer length,
        // otherwise the buffer would be too short or carry dead bytes.
        assert_eq!(total_size, 96);
    }

    /// The member offsets WGSL assigns to `SpriteUniforms` must match the
    /// CPU-side uniform writer in `sprite_uniform_data` (mat2x2 @0, vec2 @16,
    /// vec2 @24, vec4 @32 — 16 bytes, 16-byte aligned — f32 @48, f32 @52,
    /// and vec4 @64, vec4 @80; 96 bytes total). This is the GPU-side mirror of
    /// `sprite_uniform_bytes_follow_the_wgsl_layout` in backend.rs, so a
    /// layout drift on either side fails a test.
    #[test]
    fn sprite_uniform_offsets_match_the_cpu_layout() {
        let module = naga::front::wgsl::parse_str(SPRITE_SHADER)
            .expect("sprite.wgsl should parse (see all_shaders_parse_as_wgsl)");
        let ty = module
            .types
            .iter()
            .find_map(|(_, ty)| match &ty.inner {
                naga::TypeInner::Struct { .. } if ty.name.as_deref() == Some("SpriteUniforms") => {
                    Some(ty)
                }
                _ => None,
            })
            .expect("sprite.wgsl should declare the SpriteUniforms struct");
        let (offsets, total_size) = match &ty.inner {
            naga::TypeInner::Struct { members, span } => (
                members.iter().map(|m| m.offset).collect::<Vec<u32>>(),
                *span,
            ),
            _ => unreachable!("SpriteUniforms must be a struct"),
        };
        // `lit` rides in the padding the struct already kept between
        // `alpha` and the 16-byte-aligned `uv_rect`; `glow` follows the
        // uv_rect at 80.
        assert_eq!(offsets, [0, 16, 24, 32, 48, 52, 64, 80]);
        // The struct's total size must equal the CPU-side buffer length,
        // otherwise the buffer would be too short or carry dead bytes.
        assert_eq!(total_size, 96);
    }

    /// The member offsets WGSL assigns to `ParticlesUniforms` must match the
    /// CPU-side uniform writer in `particles_uniform_data` (vec2 @0, vec4 @16
    /// — 16 bytes, 16-byte aligned — vec2 @32, f32 @40, 48 bytes total).
    /// This is the GPU-side mirror of the writer, so a layout drift on either
    /// side fails a test.
    #[test]
    fn particles_uniform_offsets_match_the_cpu_layout() {
        let module = naga::front::wgsl::parse_str(PARTICLES_SHADER)
            .expect("particles.wgsl should parse (see all_shaders_parse_as_wgsl)");
        let ty = module
            .types
            .iter()
            .find_map(|(_, ty)| match &ty.inner {
                naga::TypeInner::Struct { .. } if ty.name.as_deref() == Some("ParticlesUniforms") => {
                    Some(ty)
                }
                _ => None,
            })
            .expect("particles.wgsl should declare the ParticlesUniforms struct");
        let (offsets, total_size) = match &ty.inner {
            naga::TypeInner::Struct { members, span } => (
                members.iter().map(|m| m.offset).collect::<Vec<u32>>(),
                *span,
            ),
            _ => unreachable!("ParticlesUniforms must be a struct"),
        };
        // `lit` rides in the 4 bytes the struct already padded after `misc`.
        assert_eq!(offsets, [0, 16, 32, 40]);
        // The struct's total size must equal the CPU-side buffer length,
        // otherwise the buffer would be too short or carry dead bytes.
        assert_eq!(total_size, 48);
    }

    /// The member offsets WGSL assigns to `PolylineUniforms` must match the
    /// CPU-side uniform writer in `polyline_uniform_data`:
    /// array<vec4<f32>, 128> @0 — 128 points at 16 bytes each, 2048 bytes —
    /// then vec4 @2048 (16 bytes, 16-byte aligned), u32 @2064, and f32
    /// @2068; 2080 bytes total. The points array must stay fixed at 128: the
    /// buffer size is compiled into the pipeline. Its elements must stay
    /// vec4s: the uniform address space requires an array member's stride to
    /// be a multiple of 16 bytes, and a vec2's stride is only 8 (the
    /// device-side validation rejects it even though it parses). This is the
    /// GPU-side mirror of the writer, so a layout drift on either side fails
    /// a test.
    #[test]
    fn polyline_uniform_offsets_match_the_cpu_layout() {
        let module = naga::front::wgsl::parse_str(POLYLINE_SHADER)
            .expect("polyline.wgsl should parse (see all_shaders_parse_as_wgsl)");
        let ty = module
            .types
            .iter()
            .find_map(|(_, ty)| match &ty.inner {
                naga::TypeInner::Struct { .. }
                    if ty.name.as_deref() == Some("PolylineUniforms") =>
                {
                    Some(ty)
                }
                _ => None,
            })
            .expect("polyline.wgsl should declare the PolylineUniforms struct");
        let (offsets, total_size, first) = match &ty.inner {
            naga::TypeInner::Struct { members, span } => (
                members.iter().map(|m| m.offset).collect::<Vec<u32>>(),
                *span,
                members
                    .first()
                    .expect("PolylineUniforms should declare its members"),
            ),
            _ => unreachable!("PolylineUniforms must be a struct"),
        };
        assert_eq!(offsets, [0, 2048, 2064, 2068]);
        // The struct's total size must equal the CPU-side buffer length,
        // otherwise the buffer would be too short or carry dead bytes.
        assert_eq!(total_size, 2080);
        // The points array is fixed-size (not dynamic): its 128 elements
        // are part of the pipeline's binding layout, and each element must
        // be a vec4<f32> (the uniform-space stride rule, see above).
        let points_ty = &module.types[first.ty];
        match &points_ty.inner {
            naga::TypeInner::Array {
                size: naga::ArraySize::Constant(n),
                base,
                ..
            } => {
                assert_eq!(n.get(), 128);
                let base_ty = &module.types[*base];
                assert!(
                    matches!(
                        base_ty.inner,
                        naga::TypeInner::Vector {
                            size: naga::VectorSize::Quad,
                            scalar: naga::Scalar {
                                kind: naga::ScalarKind::Float,
                                width: 4,
                            },
                        },
                    ),
                    "the points array element should be vec4<f32>, got {:?}",
                    base_ty.inner,
                );
            }
            other => panic!(
                "the points array should be fixed at 128 elements, got {other:?}"
            ),
        }
    }

    /// The member offsets WGSL assigns to `LightField` must match the
    /// CPU-side packer in `pack_light_field`: u32 @0, array<u32, 3> @4,
    /// vec4 @16 — the 32-byte header — and the light array tail at 32. The
    /// tail must be an unsized (dynamic) array: its length comes from the
    /// bound buffer, so the CPU can hold any light count with no
    /// MAX_LIGHTS. This is the GPU-side mirror of the packer's offset tests
    /// in backend/tests.rs.
    #[test]
    fn light_field_header_offsets_match_the_packer() {
        let module = naga::front::wgsl::parse_str(SHAPE_SHADER)
            .expect("shape.wgsl should parse (see all_shaders_parse_as_wgsl)");
        let ty = module
            .types
            .iter()
            .find_map(|(_, ty)| match &ty.inner {
                naga::TypeInner::Struct { .. } if ty.name.as_deref() == Some("LightField") => {
                    Some(ty)
                }
                _ => None,
            })
            .expect("shape.wgsl should declare the LightField struct");
        let (offsets, span, last) = match &ty.inner {
            naga::TypeInner::Struct { members, span } => (
                members.iter().map(|m| m.offset).collect::<Vec<u32>>(),
                *span,
                members
                    .last()
                    .expect("LightField should declare its members"),
            ),
            _ => unreachable!("LightField must be a struct"),
        };
        assert_eq!(offsets, [0, 4, 16, 32]);
        // Naga books a dynamic array's footprint as one element (its
        // creation-time minimum), so the span is the 32-byte header plus
        // one 16-byte vec4 — wgpu's minimum binding size for the struct.
        // The real buffer length comes from the bound buffer, sized by the
        // CPU to fit the frame's lights, but never below this span.
        assert_eq!(span as usize, LIGHT_FIELD_BUFFER_MIN);
        // The tail must be a dynamic array, sized by the bound buffer.
        let last_ty = &module.types[last.ty];
        match &last_ty.inner {
            naga::TypeInner::Array {
                size: naga::ArraySize::Dynamic,
                ..
            } => {}
            other => panic!(
                "the LightField tail should be a dynamic array, got {other:?}"
            ),
        }
    }

    /// The member offsets WGSL assigns to `OccluderField` must match the
    /// CPU-side packer in `pack_occluder_field`: u32 @0, array<u32, 3> @4 —
    /// the 16-byte header — and the occluder array tail at 16. The tail must
    /// be an unsized (dynamic) array: its length comes from the bound
    /// buffer, so the CPU can hold any occluder count. This is the GPU-side
    /// mirror of the packer's offset tests in backend/tests.rs.
    #[test]
    fn occluder_field_header_offsets_match_the_packer() {
        let module = naga::front::wgsl::parse_str(SHAPE_SHADER)
            .expect("shape.wgsl should parse (see all_shaders_parse_as_wgsl)");
        let ty = module
            .types
            .iter()
            .find_map(|(_, ty)| match &ty.inner {
                naga::TypeInner::Struct { .. } if ty.name.as_deref() == Some("OccluderField") => {
                    Some(ty)
                }
                _ => None,
            })
            .expect("shape.wgsl should declare the OccluderField struct");
        let (offsets, span, last) = match &ty.inner {
            naga::TypeInner::Struct { members, span } => (
                members.iter().map(|m| m.offset).collect::<Vec<u32>>(),
                *span,
                members
                    .last()
                    .expect("OccluderField should declare its members"),
            ),
            _ => unreachable!("OccluderField must be a struct"),
        };
        assert_eq!(offsets, [0, 4, 16]);
        // Naga books a dynamic array's footprint as one element (its
        // creation-time minimum), so the span is the 16-byte header plus
        // one 16-byte vec4 — wgpu's minimum binding size for the struct.
        // The real buffer length comes from the bound buffer, sized by the
        // CPU to fit the frame's occluders, but never below this span.
        assert_eq!(span as usize, OCCLUDER_FIELD_BUFFER_MIN);
        // The tail must be a dynamic array, sized by the bound buffer.
        let last_ty = &module.types[last.ty];
        match &last_ty.inner {
            naga::TypeInner::Array {
                size: naga::ArraySize::Dynamic,
                ..
            } => {}
            other => panic!(
                "the OccluderField tail should be a dynamic array, got {other:?}"
            ),
        }
    }
}
