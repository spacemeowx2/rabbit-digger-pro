use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_FEATURE_WEBUI_EMBED").is_ok() {
        let webui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("webui");

        // Rebuild when webui source changes
        println!("cargo:rerun-if-changed=webui/src");
        println!("cargo:rerun-if-changed=webui/index.html");
        println!("cargo:rerun-if-changed=webui/package.json");

        let dist_dir = webui_dir.join("dist");
        if !dist_dir.join("index.html").exists() {
            eprintln!("Building webui...");
            let status = Command::new("pnpm")
                .arg("build")
                .current_dir(&webui_dir)
                .status()
                .expect("Failed to run pnpm build. Is pnpm installed?");
            assert!(status.success(), "pnpm build failed");
        }
    }
}
