//! Parses and validates the shaders with naga alone. `gpu.rs` skips itself on a machine
//! with no GPU adapter (most CI); this still catches a WGSL error there.

use wgpu::naga::front::wgsl;
use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

#[test]
fn shaders_are_valid_wgsl_without_optional_capabilities() {
    for (name, source) in [("glass", include_str!("../src/shaders/glass.wgsl")), ("pyramid", include_str!("../src/shaders/pyramid.wgsl"))] {
        let module = wgsl::parse_str(source).unwrap_or_else(|e| panic!("{name}.wgsl: {}", e.emit_to_string(source)));
        // No capabilities: the shaders must run on baseline WebGPU.
        Validator::new(ValidationFlags::all(), Capabilities::empty()).validate(&module).unwrap_or_else(|e| panic!("{name}.wgsl: {e:?}"));
    }
}
