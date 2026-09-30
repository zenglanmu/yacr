fn main() {
    // Compile the shared Slint shell. `slint_build` embeds the generated Rust
    // into OUT_DIR; `include_modules!()` in src/lib.rs picks it up.
    slint_build::compile("ui/app.slint").expect("failed to compile ui/app.slint");
}
