use crate::scratch;
use cranelift::prelude::*;
use cranelift_module::Module;
use std::collections::HashMap;

struct Compiler {
    flags: settings::Flags,
    module: cranelift_object::ObjectModule,
    data_id_counter: usize,
    var_id_counter: usize,
    scratch_vars: HashMap<String, cranelift_module::DataId>,
}

impl Compiler {
    fn new() -> Compiler {
        let mut flag_builder = settings::builder();
        flag_builder.set("is_pic", "true").unwrap();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let flags = settings::Flags::new(flag_builder);

        let isa = cranelift_native::builder().unwrap().finish(flags.clone());

        let module = cranelift_object::ObjectModule::new(
            cranelift_object::ObjectBuilder::new(
                isa,
                "",
                cranelift_module::default_libcall_names(),
            )
            .unwrap(),
        );

        Compiler {
            flags,
            module,
            data_id_counter: 0,
            var_id_counter: 0,
            scratch_vars: HashMap::new(),
        }
    }

    fn compile_func<F>(
        &mut self,
        name: &str,
        params: &[Type],
        ret: Option<Type>,
        export: bool,
        builder: F,
    ) -> cranelift_module::FuncId
    where
        F: Fn(&mut Compiler, &mut FunctionBuilder),
    {
        let mut sig = self.module.make_signature();
        for param in params {
            sig.params.push(AbiParam::new(*param));
        }
        if let Some(ret) = ret {
            sig.returns.push(AbiParam::new(ret));
        }

        let func_id = self
            .module
            .declare_function(
                name,
                if export {
                    cranelift_module::Linkage::Export
                } else {
                    cranelift_module::Linkage::Local
                },
                &sig,
            )
            .unwrap();

        let mut ctx = self.module.make_context();
        let mut fn_builder_ctx = FunctionBuilderContext::new();
        ctx.func =
            cranelift::codegen::ir::Function::with_name_signature(ExternalName::user(0, 0), sig);

        let mut f = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);

        builder(self, &mut f);

        f.seal_all_blocks();
        f.finalize();

        cranelift::codegen::verifier::verify_function(&ctx.func, &self.flags).unwrap();

        self.module
            .define_function(
                func_id,
                &mut ctx,
                &mut cranelift::codegen::binemit::NullTrapSink {},
            )
            .unwrap();

        func_id
    }

    fn new_var(&mut self) -> Variable {
        let id = self.var_id_counter;
        self.var_id_counter += 1;
        Variable::new(id)
    }

    fn create_data(&mut self, data: Box<[u8]>) -> cranelift_module::DataId {
        let data_id = self
            .module
            .declare_data(
                &format!("data_{}", {
                    let id = self.data_id_counter;
                    self.data_id_counter += 1;
                    id
                }),
                cranelift_module::Linkage::Local,
                false,
                false,
            )
            .unwrap();
        let mut ctx = cranelift_module::DataContext::new();
        ctx.define(data);
        self.module.define_data(data_id, &ctx).unwrap();
        data_id
    }

    fn import_func(
        &mut self,
        name: &str,
        params: &[Type],
        ret: Option<Type>,
        f: &mut FunctionBuilder,
    ) -> cranelift::codegen::ir::FuncRef {
        let mut sig = self.module.make_signature();
        for param in params {
            sig.params.push(AbiParam::new(*param));
        }
        if let Some(ret) = ret {
            sig.returns.push(AbiParam::new(ret));
        }
        let func = self
            .module
            .declare_function(name, cranelift_module::Linkage::Import, &sig)
            .unwrap();
        self.module.declare_func_in_func(func, f.func)
    }

    fn scratch_var_ptr(&mut self, name: &str, f: &mut FunctionBuilder) -> Value {
        let data_id = self.scratch_vars[name];
        let data_ref = self.module.declare_data_in_func(data_id, f.func);
        f.ins()
            .global_value(self.module.target_config().pointer_type(), data_ref)
    }
}

struct BlockCompiler<'a, 'b> {
    c: &'b mut Compiler,
    f: &'b mut FunctionBuilder<'a>,
    ends: Vec<Block>,
}

impl<'a, 'b> BlockCompiler<'a, 'b> {
    fn fall_off_end(&mut self) {
        match self.ends.last() {
            Some(b) => self.f.ins().jump(*b, &[]),
            None => self.f.ins().return_(&[]),
        };
    }

    fn import_func(
        &mut self,
        name: &str,
        params: &[Type],
        ret: Option<Type>,
    ) -> cranelift::codegen::ir::FuncRef {
        self.c.import_func(name, params, ret, self.f)
    }
}

impl scratch::Value {
    fn build(&self, c: &mut BlockCompiler) -> Value {
        match self {
            scratch::Value::Number(n) => c.f.ins().f64const(*n),
            scratch::Value::String(s) => match s.parse::<f64>() {
                Ok(n) => c.f.ins().f64const(n),
                Err(_) => c.f.ins().f64const(0.0),
            },
            scratch::Value::Load(id) => {
                let ptr = c.c.scratch_var_ptr(id, c.f);
                c.f.ins().load(types::F64, MemFlags::new(), ptr, 0)
            }
        }
    }
}

impl scratch::BlockExpression {
    fn build(&self, c: &mut BlockCompiler) -> Value {
        match self {
            scratch::BlockExpression::OperatorEquals { left, right } => {
                let a1 = left.build(c);
                let a2 = right.build(c);
                c.f.ins().fcmp(FloatCC::Equal, a1, a2)
            }
        }
    }
}

impl scratch::Block {
    fn build(&self, c: &mut BlockCompiler, block: Block) {
        c.f.switch_to_block(block);

        match &self.op {
            scratch::BlockOp::ControlRepeat { times, body } => {
                let head = c.f.create_block();
                let bbody = c.f.create_block();
                let bnext = c.f.create_block();
                let vtimes = c.c.new_var();

                {
                    c.f.declare_var(vtimes, types::I32);

                    let tmp = c.f.ins().iconst(types::I32, times.as_number() as i64 + 1);
                    c.f.def_var(vtimes, tmp);

                    c.f.ins().jump(head, &[]);
                }

                {
                    c.f.switch_to_block(head);
                    let a1 = c.f.use_var(vtimes);
                    let a2 = c.f.ins().iconst(types::I32, 1);
                    let tmp = c.f.ins().isub(a1, a2);
                    c.f.def_var(vtimes, tmp);
                    c.f.ins().brz(tmp, bnext, &[]);
                    c.f.ins().jump(bbody, &[]);
                }

                c.ends.push(head);
                body.build(c, bbody);
                c.ends.pop();

                if let Some(next) = &self.next {
                    next.build(c, bnext);
                } else {
                    c.f.switch_to_block(bnext);
                    c.fall_off_end();
                }
            }
            scratch::BlockOp::ControlForever(body) => {
                c.ends.push(block);
                body.build(c, block);
                c.ends.pop();
            }
            scratch::BlockOp::ControlWait(delay) => {
                let libc_sleep = c.import_func("sleep", &[types::I32], None);

                let tmp = delay.build(c);
                let tmp = c.f.ins().fcvt_to_uint(types::I32, tmp);
                c.f.ins().call(libc_sleep, &[tmp]);
            }
            scratch::BlockOp::ControlIfElse {
                condition,
                consequent,
                alternative,
            } => {
                let bcons = c.f.create_block();
                let balt = c.f.create_block();
                let bnext = c.f.create_block();

                let tmp = condition.build(c);
                c.f.ins().brz(tmp, balt, &[]);
                c.f.ins().jump(bcons, &[]);

                c.ends.push(bnext);
                consequent.build(c, bcons);
                alternative.build(c, balt);
                c.ends.pop();

                if let Some(next) = &self.next {
                    next.build(c, bnext);
                } else {
                    c.f.switch_to_block(bnext);
                    c.fall_off_end();
                }
            }
            scratch::BlockOp::ControlStopAll => {
                let libc_exit = c.import_func("exit", &[types::I32], None);
                let detach_scripts = c.import_func("detach_scripts", &[], None);

                c.f.ins().call(detach_scripts, &[]);

                let tmp = c.f.ins().iconst(types::I32, 0);
                c.f.ins().call(libc_exit, &[tmp]);

                c.f.ins().trap(TrapCode::UnreachableCodeReached);
            }
            scratch::BlockOp::ControlStopScript => {
                c.f.ins().return_(&[]);
            }
            scratch::BlockOp::LooksSay(s) => {
                let s = format!("{}\n", s.as_str());

                let p = c.c.module.target_config().pointer_type();
                let libc_write = c.import_func("write", &[types::I32, p, p], Some(p));

                let fd = c.f.ins().iconst(types::I32, 1);

                let data = c.c.create_data(s.as_bytes().into());
                let tmp = c.c.module.declare_data_in_func(data, &mut c.f.func);
                let ptr = c.f.ins().global_value(p, tmp);

                let len = c.f.ins().iconst(p, s.len() as i64);

                c.f.ins().call(libc_write, &[fd, ptr, len]);
            }
            scratch::BlockOp::EventWhenFlagClicked => {}
            scratch::BlockOp::DataSetVariableTo { id, value } => {
                let ptr = c.c.scratch_var_ptr(id, c.f);
                let val = value.build(c);
                c.f.ins().store(MemFlags::new(), val, ptr, 0);
            }
            scratch::BlockOp::DataChangeVariableBy { id, value } => {
                let ptr = c.c.scratch_var_ptr(id, c.f);
                let val = c.f.ins().load(types::F64, MemFlags::new(), ptr, 0);
                let dif = value.build(c);
                let val = c.f.ins().fadd(val, dif);
                c.f.ins().store(MemFlags::new(), val, ptr, 0);
            }
        }

        if !c.f.is_filled() {
            if let Some(next) = &self.next {
                let bnext = c.f.create_block();
                c.f.ins().jump(bnext, &[]);
                next.build(c, bnext);
            } else {
                c.fall_off_end();
            }
        }
    }
}

pub fn compile(variables: &[String], scripts: &[scratch::Block]) -> Vec<u8> {
    let mut compiler = Compiler::new();

    let mut script_funcs = vec![];

    for var in variables {
        let data_id = compiler
            .module
            .declare_data(var, cranelift_module::Linkage::Local, true, false)
            .unwrap();
        let mut ctx = cranelift_module::DataContext::new();
        ctx.define(Box::new([0, 0, 0, 0]));
        compiler.module.define_data(data_id, &ctx).unwrap();
        compiler.scratch_vars.insert(var.to_owned(), data_id);
    }

    for (i, script) in scripts.iter().enumerate() {
        let func_id = compiler.compile_func(&format!("script_{}", i), &[], None, false, |c, f| {
            let mut bc = BlockCompiler {
                c,
                f,
                ends: Vec::new(),
            };
            let block = bc.f.create_block();
            script.build(&mut bc, block);
        });

        script_funcs.push(func_id);
    }

    compiler.compile_func("main", &[], None, true, |compiler, f| {
        let block = f.create_block();
        f.switch_to_block(block);

        let extern_spawn_script = compiler.import_func(
            "spawn_script",
            &[compiler.module.target_config().pointer_type()],
            None,
            f,
        );

        for func_id in &script_funcs {
            let tmp = compiler.module.declare_func_in_func(*func_id, &mut f.func);
            let tmp = f
                .ins()
                .func_addr(compiler.module.target_config().pointer_type(), tmp);
            f.ins().call(extern_spawn_script, &[tmp]);
        }

        let extern_join_scripts = compiler.import_func("join_scripts", &[], None, f);
        f.ins().call(extern_join_scripts, &[]);

        f.ins().return_(&[]);
    });

    compiler.module.finish().emit().unwrap()
}
