#![crate_type = "staticlib"]

type ScriptFn = unsafe extern "C" fn() -> ();

static mut THREADS: Vec<std::thread::JoinHandle<()>> = Vec::new();

#[no_mangle]
extern "C" fn support_spawn_script(f: ScriptFn) {
    unsafe {
        THREADS.push(std::thread::spawn(move || {
            f()
        }));
    }
}

#[no_mangle]
extern "C" fn support_detach_scripts() {
    unsafe { THREADS.clear() }
}

#[no_mangle]
extern "C" fn support_join_scripts() {
    unsafe {
        for t in THREADS.drain(0..) {
            t.join().unwrap();
        }
    }
}

#[no_mangle]
extern "C" fn support_write_float(f: f64) {
    println!("{}", f);
}

#[no_mangle]
extern "C" fn support_sleep(s: f64) {
    std::thread::sleep(std::time::Duration::from_secs_f64(s))
}
