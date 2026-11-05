fn main() {
    // 图标改了就重新跑构建脚本，否则 exe 里的资源还是旧的
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    tauri_build::build()
}
