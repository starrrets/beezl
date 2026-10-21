fn main() {
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=ui");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Setup build: CUURSOR_PAYLOAD points at a built cuursor.exe, which gets
    // embedded (compressed) into cuursor-ui.exe so it can install itself.
    println!("cargo:rerun-if-env-changed=CUURSOR_PAYLOAD");
    let payload = match std::env::var("CUURSOR_PAYLOAD") {
        Ok(path) => {
            println!("cargo:rerun-if-changed={path}");
            let exe = std::fs::read(&path).expect("CUURSOR_PAYLOAD");
            miniz_oxide::deflate::compress_to_vec(&exe, 10)
        }
        Err(_) => Vec::new(),
    };
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("daemon.deflate");
    std::fs::write(out, payload).unwrap();

    // Icons, version info and the manifest (per-monitor DPI v2, visual styles).
    embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_required().unwrap();
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent".into()),
    )
    .unwrap();
}
