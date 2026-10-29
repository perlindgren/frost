//! The guest path's promises, measured on this machine's adapter.
//!
//! A host builds the wgpu device and frost draws on it, so the one thing
//! frost can get wrong before any frame exists is the limits its host should
//! have asked for. The shape pipeline binds storage buffers visible to the
//! fragment stage; a device requested with a downlevel or WebGL2 profile
//! zeroes those, and wgpu answers the first `create_bind_group_layout` with
//! a validation panic — inside somebody else's application.
//! [`device_limits`](crate::backend::embed::device_limits) is frost's answer
//! to "what should I request"; these tests are why the answer has the shape
//! it does.

use wgpu::{
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType, BufferBindingType, ShaderStages,
};

use crate::backend::embed::EmbeddedConfig;
use crate::backend::embed::device_limits;
use crate::backend::tests::machine::machine;
use crate::objects::SpriteFilter;

/// What [`Embedded::new`](crate::backend::embed::Embedded::new) asserts at
/// construction, checked against the adapter rather than asserted in the
/// same breath as the code that asserts it: a host that asks for what
/// [`device_limits`] says gets a device carrying the two storage buffers
/// visible to the fragment stage that the shape pipeline binds — and the
/// count is not just high enough, it is the count the engine's own device
/// runs with, which is the device frost was written against.
#[test]
fn the_helper_asks_for_the_buffers_the_shape_pipeline_binds() {
    let Some(machine) = machine() else {
        return;
    };
    let asked = device_limits(machine.adapter());
    assert!(
        asked.max_storage_buffers_per_shader_stage >= 2,
        "device_limits asks for a device with {} storage buffers per shader stage; \
         the shape pipeline binds 2 visible to the fragment stage",
        asked.max_storage_buffers_per_shader_stage
    );
    assert_eq!(
        asked.max_storage_buffers_per_shader_stage,
        machine.device.limits().max_storage_buffers_per_shader_stage,
        "device_limits and the device the engine draws on disagree about storage \
         buffers per shader stage — a host following the helper would get a device \
         frost's own pipelines were never built on"
    );

    // And the layout that panics on a device without them is laid out for
    // real, on that device: two read-only storage buffers, both visible to
    // the fragment stage, which is the shape of the bind group the shape
    // pipeline asks wgpu to describe.
    let storage = BindGroupLayoutEntry {
        binding: 0,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            // The count is the question, not the size: a device without the
            // buffers fails the creation whatever the size says, and sizing
            // to the WGSL's minimum would only re-test the alignment rules.
            min_binding_size: None,
        },
        count: None,
    };
    let _layout = machine
        .device
        .create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("two fragment-visible storage buffers"),
            entries: &[
                storage,
                BindGroupLayoutEntry {
                    binding: 1,
                    ..storage
                },
            ],
        });
}

/// The helper is frost's uplevel defaults with one thing borrowed from the
/// adapter: its resolution limits, because a swap chain may be the largest
/// image in the application. `using_resolution` in wgpu 30 copies exactly
/// `max_texture_dimension_1d`, `_2d` and `_3d` and nothing else, so that is
/// the whole of the clamp, and the rest of the request must still be
/// frost's own — a helper that quietly lowered the storage-buffer count
/// while raising a texture size would undo the guarantee above.
#[test]
fn the_helper_borrows_the_adapters_sizes_and_nothing_else() {
    let Some(machine) = machine() else {
        return;
    };
    let asked = device_limits(machine.adapter());
    let sizes = machine.adapter().limits();
    assert_eq!(
        asked.max_texture_dimension_1d,
        sizes.max_texture_dimension_1d
    );
    assert_eq!(
        asked.max_texture_dimension_2d,
        sizes.max_texture_dimension_2d
    );
    assert_eq!(
        asked.max_texture_dimension_3d,
        sizes.max_texture_dimension_3d
    );

    let defaults = wgpu::Limits::defaults();
    for (name, asked, default) in [
        (
            "max_storage_buffers_per_shader_stage",
            asked.max_storage_buffers_per_shader_stage,
            defaults.max_storage_buffers_per_shader_stage,
        ),
        (
            "max_bind_groups",
            asked.max_bind_groups,
            defaults.max_bind_groups,
        ),
        (
            "max_bindings_per_bind_group",
            asked.max_bindings_per_bind_group,
            defaults.max_bindings_per_bind_group,
        ),
        (
            "max_compute_workgroup_size_x",
            asked.max_compute_workgroup_size_x,
            defaults.max_compute_workgroup_size_x,
        ),
    ] {
        assert_eq!(
            asked, default,
            "device_limits changed {name} from frost's defaults ({default}) to {asked}; \
             only the three texture dimensions are the adapter's to set"
        );
    }
    // Apart, because a buffer's size is 64-bit where a bind group count is
    // 32, and a list cannot hold both.
    assert_eq!(
        asked.max_buffer_size, defaults.max_buffer_size,
        "device_limits changed the buffer size frost asks a host to request"
    );
}

/// The settings a host gets when it asks for nothing: the surface's own
/// resolution, bilinear where a buffer is stretched, and neither glow nor
/// headroom. A guest's first frame should look like the scene the host
/// authored, not like a filter chain it never asked for.
#[test]
fn embedded_config_defaults_to_the_surfaces_own_shape() {
    let config = EmbeddedConfig::default();
    assert_eq!(config.render_size, None);
    assert!(matches!(config.blit_filter, SpriteFilter::Linear));
    assert_eq!(config.bloom, 0.0);
    assert_eq!(config.tone_map, 0.0);
}
