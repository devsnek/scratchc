fn main() {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=./support.rs");
    let o = std::process::Command::new("rustc")
        .args(&[
            "-O",
            "./support.rs",
            "-o",
            out_dir.join("libsupport.a").to_str().unwrap(),
            "--print",
            "native-static-libs",
        ])
        .output()
        .unwrap();

    if !o.status.success() {
        panic!("{}", String::from_utf8(o.stderr).unwrap());
    }

    let stderr = String::from_utf8(o.stderr).unwrap();
    let static_libs: Vec<String> = stderr
        .lines()
        .find(|l| l.starts_with("note: native-static-libs:"))
        .unwrap()
        .split(": ")
        .nth(2)
        .unwrap()
        .split(' ')
        .map(|s| s.to_owned())
        .collect();

    std::fs::write(
        out_dir.join("libsupport_libs.rs"),
        format!("&{:?}", static_libs),
    )
    .unwrap();
}
