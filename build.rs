fn main() {
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=ui");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Icons, version info and the manifest (per-monitor DPI v2, visual styles).
    embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_required().unwrap();
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .unwrap();
}
