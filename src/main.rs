fn main() {
    let file = std::env::args().nth(1).unwrap();
    let out_name = std::env::args().nth(2).unwrap();
    let file = std::fs::File::open(file).unwrap();

    scratch2bin::compile_native(file, &out_name);
}
