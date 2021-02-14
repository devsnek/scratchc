mod compiler;
mod scratch;

fn main() {
    let file = std::env::args().nth(1).unwrap();
    let out_name = std::env::args().nth(2).unwrap();
    let file = std::fs::File::open(file).unwrap();
    let project = scratch::ProjectInfo::new(file).unwrap();

    let mut variables = vec![];
    let mut procedures = vec![];
    let mut scripts = vec![];

    for target in project.targets {
        let target = scratch::Target::hydrate(target);
        for var in target.variables.keys() {
            variables.push(var.clone());
        }

        for proc in target.procedures {
            procedures.push(proc);
        }

        for script in target.scripts {
            scripts.push(script);
        }
    }

    let o = compiler::compile(&variables, &procedures, &scripts);

    // FIXME: this is terrible
    std::fs::write("./out.o", o).unwrap();
    std::fs::write("./support.cc", include_bytes!("./support.cc")).unwrap();

    let r = std::process::Command::new("clang++")
        .args(&[
            "-O3",
            "./out.o",
            "./support.cc",
            "-pthread",
            "-o",
            &out_name,
        ])
        .output()
        .unwrap();

    std::fs::remove_file("./out.o").unwrap();
    std::fs::remove_file("./support.cc").unwrap();

    if !r.status.success() {
        eprintln!("{}", String::from_utf8(r.stderr).unwrap());
    }
}
