mod compiler;
mod scratch;

pub fn compile(
    module: &mut impl cranelift_module::Module,
    file: impl std::io::Read + std::io::Seek,
) {
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

    compiler::compile(module, &variables, &procedures, &scripts);
}

pub fn compile_native(file: impl std::io::Read + std::io::Seek, out_name: &str) {
    let o = {
        use cranelift::prelude::*;

        let mut flag_builder = settings::builder();
        flag_builder.set("is_pic", "true").unwrap();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let flags = settings::Flags::new(flag_builder);

        let isa = cranelift_native::builder().unwrap().finish(flags);

        let mut module = cranelift_object::ObjectModule::new(
            cranelift_object::ObjectBuilder::new(
                isa,
                "",
                cranelift_module::default_libcall_names(),
            )
            .unwrap(),
        );

        compile(&mut module, file);

        module.finish().emit().unwrap()
    };

    // FIXME: this is terrible
    let tmp = std::env::temp_dir();
    std::fs::write(tmp.join("out.o"), o).unwrap();
    std::fs::write(tmp.join("support.cc"), include_bytes!("./support.cc")).unwrap();

    let r = std::process::Command::new("clang++")
        .args(&[
            "-O3",
            tmp.join("out.o").to_str().unwrap(),
            tmp.join("support.cc").to_str().unwrap(),
            "-pthread",
            "-o",
            &out_name,
        ])
        .output()
        .unwrap();

    if !r.status.success() {
        panic!("{}", String::from_utf8(r.stderr).unwrap());
    }
}
