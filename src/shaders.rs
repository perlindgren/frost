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

#[cfg(test)]
mod tests {
    //! Shader validation without running anything: naga's WGSL frontend is
    //! the exact parser wgpu executes inside `Device::create_shader_module`,
    //! so if these tests pass, module creation in the examples cannot fail.

    use super::*;

    /// All four shaders parse as valid WGSL.
    #[test]
    fn all_shaders_parse_as_wgsl() {
        let shaders = [
            ("line.wgsl", LINE_SHADER),
            ("circle.wgsl", CIRCLE_SHADER),
            ("rectangle.wgsl", RECT_SHADER),
            ("shape.wgsl", SHAPE_SHADER),
        ];
        for (name, source) in shaders {
            naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|err| panic!("{name} is not valid WGSL: {err}"));
        }
    }

    /// The member offsets WGSL assigns to `ShapeUniforms` must match the
    /// CPU-side uniform writer in `shape_uniform_data` (mat2x2 @0, vec2 @16,
    /// vec2 @24, vec2 @32, vec3 @48 — 12 bytes, 16-byte aligned — vec2 @64,
    /// 80 bytes total). This is the GPU-side mirror of
    /// `shape_uniform_bytes_follow_the_wgsl_layout` in lib.rs, so a layout
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
        assert_eq!(offsets, [0, 16, 24, 32, 48, 64]);
        // The struct's total size must equal the CPU-side buffer length,
        // otherwise the buffer would be too short or carry dead bytes.
        assert_eq!(total_size, 80);
    }
}
