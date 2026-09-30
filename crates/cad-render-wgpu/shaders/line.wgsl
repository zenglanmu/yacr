// Candidate shader only; no pipeline, capability or golden-image validation yet.
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
};
@vertex fn vs_main(@location(0) position: vec3<f32>) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(position, 1.0);
    return output;
}
@fragment fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}
