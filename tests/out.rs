#[macro_use]
extern crate pretty_assertions;

#[test_generator::test_resources("tests/out/*.sb3")]
fn test(test: &str) {
    let test = std::path::PathBuf::from(test);
    let file = std::fs::File::open(&test).unwrap();

    let tmp = std::env::temp_dir()
        .join(test.file_name().unwrap())
        .to_str()
        .unwrap()
        .to_owned();

    scratchc::compile_native(file, &tmp);

    let o = std::process::Command::new(&tmp).output().unwrap();

    assert!(o.status.success());

    let mut out = test;
    out.set_extension("out");
    assert_eq!(
        String::from_utf8(o.stdout).unwrap(),
        std::fs::read_to_string(&out).unwrap()
    );
}
