use std::{env, fs, path::Path};

fn main() {
    if env::var_os("CARGO_FEATURE_APPLIANCE").is_none() {
        return;
    }
    println!("cargo:rerun-if-changed=web/dist");
    let root = env::current_dir().expect("repository").join("web/dist");
    assert!(
        root.join("index.html").exists(),
        "panel assets missing: run `make web-build` before building the appliance"
    );
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    let mut generated = String::from("static ASSETS: &[(&str, &[u8], &str)] = &[\n");
    for file in files {
        let relative = file.strip_prefix(&root).unwrap().to_str().unwrap();
        let mime = match file.extension().and_then(|s| s.to_str()) {
            Some("html") => "text/html; charset=utf-8",
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("svg") => "image/svg+xml",
            Some("png") => "image/png",
            Some("woff2") => "font/woff2",
            _ => "application/octet-stream",
        };
        generated.push_str(&format!(
            "({relative:?}, include_bytes!({:?}), {mime:?}),\n",
            file.to_str().unwrap()
        ));
    }
    generated.push_str("];\n");
    fs::write(
        Path::new(&env::var_os("OUT_DIR").unwrap()).join("panel_assets.rs"),
        generated,
    )
    .unwrap();
}

fn collect(path: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}
