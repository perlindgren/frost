//! The WGSL shader sources, embedded at compile time from the `.wgsl` files
//! in the crate's `shaders/` directory.

/// The line shader source.
pub(crate) const LINE_SHADER: &str = include_str!("../shaders/line.wgsl");

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
    //! Shader validation without running anything: naga's WGSL frontend is
    //! the exact parser wgpu executes inside `Device::create_shader_module`,
    //! so if these tests pass, module creation in the examples cannot fail.

    use super::*;

    /// All six shaders parse as valid WGSL.
    #[test]
    fn all_shaders_parse_as_wgsl() {
        let shaders = [
            ("line.wgsl", LINE_SHADER),
            ("circle.wgsl", CIRCLE_SHADER),
            ("rectangle.wgsl", RECT_SHADER),
            ("shape.wgsl", SHAPE_SHADER),
            ("sprite.wgsl", SPRITE_SHADER),
            ("particles.wgsl", PARTICLES_SHADER),
        ];
        for (name, source) in shaders {
            naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|err| panic!("{name} is not valid WGSL: {err}"));
        }
    }

    /// The member offsets WGSL assigns to `ShapeUniforms` must match the
    /// CPU-side uniform writer in `shape_uniform_data` (mat2x2 @0, vec2 @16,
    /// vec2 @24, vec2 @32, vec4 @48 — 16 bytes, 16-byte aligned — vec2 @64,
    /// f32 @72, 80 bytes total). This is the GPU-side mirror of
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
        // `lit` rides in the 4 bytes the struct already padded after `misc`.
        assert_eq!(offsets, [0, 16, 24, 32, 48, 64, 72]);
        // The struct's total size must equal the CPU-side buffer length,
        // otherwise the buffer would be too short or carry dead bytes.
        assert_eq!(total_size, 80);
    }

    /// The member offsets WGSL assigns to `SpriteUniforms` must match the
    /// CPU-side uniform writer in `sprite_uniform_data` (mat2x2 @0, vec2 @16,
    /// vec2 @24, vec4 @32 — 16 bytes, 16-byte aligned — f32 @48, f32 @52,
    /// and vec4 @64; 80 bytes total). This is the GPU-side mirror of
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
        // `alpha` and the 16-byte-aligned `uv_rect`.
        assert_eq!(offsets, [0, 16, 24, 32, 48, 52, 64]);
        // The struct's total size must equal the CPU-side buffer length,
        // otherwise the buffer would be too short or carry dead bytes.
        assert_eq!(total_size, 80);
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
        // one 16-byte vec4 — the real buffer length comes from the bound
        // buffer, sized by the CPU to fit the frame's lights.
        assert_eq!(span, 32 + 16);
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
        // one 16-byte vec4 — the real buffer length comes from the bound
        // buffer, sized by the CPU to fit the frame's occluders.
        assert_eq!(span, 16 + 16);
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
