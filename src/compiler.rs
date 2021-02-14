use crate::scratch;
use cranelift::prelude::*;
use cranelift_module::Module;
use std::collections::HashMap;

struct Compiler<M: Module> {
    module: M,
    data_id_counter: usize,
    var_id_counter: usize,
    scratch_vars: HashMap<String, cranelift_module::DataId>,
    procedures: HashMap<String, cranelift_module::FuncId>,
}

impl<M: Module> Compiler<M> {
    fn new(module: M) -> Compiler<M> {
        Compiler {
            module,
            data_id_counter: 0,
            var_id_counter: 0,
            scratch_vars: HashMap::new(),
            procedures: HashMap::new(),
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
        F: Fn(&mut Compiler<M>, &mut FunctionBuilder, cranelift_module::FuncId),
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
        ctx.func = cranelift::codegen::ir::Function::with_name_signature(
            ExternalName::testcase(name),
            sig,
        );

        let mut f = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);

        builder(self, &mut f, func_id);

        f.seal_all_blocks();
        f.finalize();

        cranelift::codegen::verifier::verify_function(&ctx.func, self.module.isa().flags())
            .unwrap();

        // println!("{}", ctx.func.display(None));

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

    fn create_scratch_var(&mut self, name: &str) {
        let data_id = self
            .module
            .declare_data(name, cranelift_module::Linkage::Local, true, false)
            .unwrap();
        let mut ctx = cranelift_module::DataContext::new();
        ctx.define(Box::new([0; std::mem::size_of::<f64>()]));
        self.module.define_data(data_id, &ctx).unwrap();
        self.scratch_vars.insert(name.to_owned(), data_id);
    }

    fn scratch_var_ptr(&mut self, name: &str, f: &mut FunctionBuilder) -> Value {
        let data_id = self.scratch_vars[name];
        let data_ref = self.module.declare_data_in_func(data_id, f.func);
        f.ins()
            .global_value(self.module.target_config().pointer_type(), data_ref)
    }

    fn load_scratch_var(&mut self, name: &str, f: &mut FunctionBuilder) -> Value {
        let ptr = self.scratch_var_ptr(name, f);
        f.ins().load(types::F64, MemFlags::new(), ptr, 0)
    }

    fn store_scratch_var(&mut self, name: &str, val: Value, f: &mut FunctionBuilder) {
        let ptr = self.scratch_var_ptr(name, f);
        f.ins().store(MemFlags::new(), val, ptr, 0);
    }
}

struct BlockCompiler<'a, 'b, M: Module> {
    c: &'b mut Compiler<M>,
    f: &'b mut FunctionBuilder<'a>,
    ends: Vec<Block>,
    args: HashMap<String, Variable>,
}

impl<'a, 'b, M: Module> BlockCompiler<'a, 'b, M> {
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
    fn build(&self, c: &mut BlockCompiler<impl Module>) -> Value {
        match self {
            scratch::Value::Number(n) => c.f.ins().f64const(*n),
            scratch::Value::String(s) => match s.parse::<f64>() {
                Ok(n) => c.f.ins().f64const(n),
                Err(_) => c.f.ins().f64const(0.0),
            },
            scratch::Value::Load(id) => c.c.load_scratch_var(id, c.f),
            scratch::Value::Expression(b) => match &**b {
                scratch::BlockExpression::OperatorEquals { left, right } => {
                    let a1 = left.build(c);
                    let a2 = right.build(c);
                    c.f.ins().fcmp(FloatCC::Equal, a1, a2)
                }
                scratch::BlockExpression::OperatorGT { left, right } => {
                    let a1 = left.build(c);
                    let a2 = right.build(c);
                    c.f.ins().fcmp(FloatCC::GreaterThan, a1, a2)
                }
                scratch::BlockExpression::OperatorAdd { left, right } => {
                    let a1 = left.build(c);
                    let a2 = right.build(c);
                    c.f.ins().fadd(a1, a2)
                }
                scratch::BlockExpression::OperatorSubtract { left, right } => {
                    let a1 = left.build(c);
                    let a2 = right.build(c);
                    c.f.ins().fsub(a1, a2)
                }
                scratch::BlockExpression::ArgumentReporterStringNumber { name } => {
                    let var = c.args[name];
                    c.f.use_var(var)
                }
            },
        }
    }
}

impl scratch::Block {
    fn build(&self, c: &mut BlockCompiler<impl Module>, block: Block) {
        c.f.switch_to_block(block);

        match &self.op {
            scratch::BlockOp::ControlRepeat { times, body } => {
                let head = c.f.create_block();
                let bbody = c.f.create_block();
                let bnext = c.f.create_block();
                let vtimes = c.c.new_var();

                {
                    c.f.declare_var(vtimes, types::I32);

                    let tmp = times.build(c);
                    let tmp = c.f.ins().fcvt_to_uint(types::I32, tmp);
                    let one = c.f.ins().iconst(types::I32, 1);
                    let tmp = c.f.ins().iadd(tmp, one);
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
                if alternative.is_some() {
                    c.f.ins().brz(tmp, balt, &[]);
                } else {
                    c.f.ins().brz(tmp, bnext, &[]);
                }
                c.f.ins().jump(bcons, &[]);

                c.ends.push(bnext);
                consequent.build(c, bcons);
                if let Some(alternative) = alternative {
                    alternative.build(c, balt);
                }
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
                let p = c.c.module.target_config().pointer_type();
                match s {
                    scratch::Value::String(s) => {
                        let libc_write = c.import_func("write", &[types::I32, p, p], Some(p));

                        let fd = c.f.ins().iconst(types::I32, 1);

                        let s = format!("{}\n", s);

                        let data = c.c.create_data(s.as_bytes().into());
                        let tmp = c.c.module.declare_data_in_func(data, &mut c.f.func);
                        let ptr = c.f.ins().global_value(p, tmp);

                        let len = c.f.ins().iconst(p, s.len() as i64);

                        c.f.ins().call(libc_write, &[fd, ptr, len]);
                    }
                    _ => {
                        let write_float = c.import_func("write_float", &[types::F64], None);
                        let tmp = s.build(c);
                        c.f.ins().call(write_float, &[tmp]);
                    }
                };
            }
            scratch::BlockOp::EventWhenFlagClicked => {}
            scratch::BlockOp::DataSetVariableTo { id, value } => {
                let val = value.build(c);
                c.c.store_scratch_var(id, val, c.f);
            }
            scratch::BlockOp::DataChangeVariableBy { id, value } => {
                let val = c.c.load_scratch_var(id, c.f);
                let dif = value.build(c);
                let val = c.f.ins().fadd(val, dif);
                c.c.store_scratch_var(id, val, c.f);
            }
            scratch::BlockOp::ProceduresCall { proc, args } => {
                let mut arguments = vec![];
                for v in args {
                    arguments.push(v.build(c));
                }
                let tmp =
                    c.c.module
                        .declare_func_in_func(c.c.procedures[proc], c.f.func);
                c.f.ins().call(tmp, &arguments);
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

pub fn compile(
    m: &mut impl Module,
    variables: &[String],
    procedures: &[scratch::Procedure],
    scripts: &[scratch::Block],
) {
    let mut compiler = Compiler::new(m);

    let mut script_funcs = vec![];

    for var in variables {
        compiler.create_scratch_var(var);
    }

    for proc in procedures {
        compiler.compile_func(
            &format!("proc_{}", proc.id),
            &vec![types::F64; proc.arguments.len()],
            None,
            false,
            |c, f, func_id| {
                c.procedures.insert(proc.id.clone(), func_id); // insert here to support recursion

                let block = f.create_block();
                f.append_block_params_for_function_params(block);
                f.switch_to_block(block);
                let mut args = HashMap::new();
                for (i, name) in proc.arguments.iter().enumerate() {
                    let var = c.new_var();
                    f.declare_var(var, types::F64);
                    let val = f.block_params(block)[i];
                    f.def_var(var, val);
                    args.insert(name.to_owned(), var);
                }

                let mut bc = BlockCompiler {
                    c,
                    f,
                    ends: Vec::new(),
                    args,
                };
                proc.body.build(&mut bc, block);
            },
        );
    }

    for (i, script) in scripts.iter().enumerate() {
        let func_id =
            compiler.compile_func(&format!("script_{}", i), &[], None, false, |c, f, _| {
                let mut bc = BlockCompiler {
                    c,
                    f,
                    ends: Vec::new(),
                    args: HashMap::new(),
                };
                let block = bc.f.create_block();
                script.build(&mut bc, block);
            });

        script_funcs.push(func_id);
    }

    compiler.compile_func("main", &[], None, true, |compiler, f, _| {
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
}
