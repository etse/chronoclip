fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    #[allow(clippy::expect_used)] // A broken .slint file must fail the build.
    slint_build::compile_with_config("ui/main.slint", config).expect("failed to compile Slint UI");
}
