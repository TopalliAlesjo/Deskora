fn main() {
    println!("cargo:rerun-if-changed=assets/deskora.rc");
    println!("cargo:rerun-if-changed=assets/deskora.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile_for("assets/deskora.rc", ["deskora"], embed_resource::NONE);
    }
}
