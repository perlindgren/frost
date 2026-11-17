//! Real main-thread test of independently linked wrapper copies; no GPU/window.
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]

// Exercise the real private implementation on the OS main thread, including
// ownership transitions that an ordinary threaded Rust test cannot run there.
#[cfg(target_os = "macos")]
#[path = "../src/callback.rs"]
// The source's ordinary #[test] imports are unused in this harness=false binary.
#[allow(unused_imports)]
mod callback;
#[cfg(target_os = "macos")]
use callback::Callback;
#[cfg(target_os = "macos")]
#[path = "../src/target.rs"]
#[allow(unused_imports)]
mod target;

#[cfg(target_os = "macos")]
mod native {
    use objc2::{MainThreadMarker, Message, msg_send, rc::autoreleasepool, runtime::AnyClass, sel};
    use objc2_app_kit::NSView;
    use std::{
        cell::{Cell, RefCell},
        ffi::{CStr, CString, c_char, c_int, c_void},
        fs,
        path::{Path, PathBuf},
        process::Command,
        rc::Rc,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[link(name = "System")]
    unsafe extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
    }

    struct Library {
        handle: *mut c_void,
        create: extern "C" fn() -> *mut c_void,
        destroy: unsafe extern "C" fn(*mut c_void),
        dropped: extern "C" fn() -> usize,
    }
    impl Library {
        fn open(path: &Path) -> Self {
            let path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            // SAFETY: these are our freshly built test fixtures, not arbitrary
            // native code. The C string is live; NOW|LOCAL are Darwin's flags.
            let handle = unsafe { dlopen(path.as_ptr(), 2 | 4) };
            assert!(!handle.is_null(), "could not load {path:?}");
            let symbol = |name: &CStr| {
                // SAFETY: handle is live and name is a null-terminated string.
                let pointer = unsafe { dlsym(handle, name.as_ptr()) };
                assert!(!pointer.is_null(), "missing fixture symbol {name:?}");
                pointer
            };
            // SAFETY: fixtures/entry.rs declares these exact exported ABIs.
            let (create, destroy, dropped) = unsafe {
                (
                    std::mem::transmute::<*mut c_void, extern "C" fn() -> *mut c_void>(symbol(
                        c"review_create",
                    )),
                    std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void)>(symbol(
                        c"review_destroy",
                    )),
                    std::mem::transmute::<*mut c_void, extern "C" fn() -> usize>(symbol(
                        c"review_dropped",
                    )),
                )
            };
            Self {
                handle,
                create,
                destroy,
                dropped,
            }
        }
        fn instance(&self) -> Instance<'_> {
            let pointer = (self.create)();
            assert!(
                !pointer.is_null(),
                "fixture could not create a display callback"
            );
            Instance {
                library: self,
                pointer,
            }
        }
    }
    impl Drop for Library {
        fn drop(&mut self) {
            // SAFETY: Instance's borrow prevents unloading with a live driver.
            assert_eq!(unsafe { dlclose(self.handle) }, 0);
        }
    }
    struct Instance<'a> {
        library: &'a Library,
        pointer: *mut c_void,
    }
    impl Drop for Instance<'_> {
        fn drop(&mut self) {
            // SAFETY: this allocation came from this live library and is
            // returned exactly once, on the main thread.
            unsafe { (self.library.destroy)(self.pointer) };
        }
    }
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixtures() -> Scratch {
        let root = std::env::temp_dir().join(format!(
            "frost-display-sync-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let wrapper = env!("CARGO_MANIFEST_DIR");
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"one\", \"two\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        for package in ["one", "two"] {
            let directory = root.join(package);
            fs::create_dir_all(directory.join("src")).unwrap();
            fs::write(
                directory.join("src/lib.rs"),
                include_str!("fixtures/entry.rs"),
            )
            .unwrap();
            let manifest = format!(
                r#"[package]
name = "display-fixture-{package}"
version = "0.0.0"
edition = "2024"
[lib]
crate-type = ["cdylib"]
[dependencies]
macos-frame-sync = {{ path = {wrapper:?} }}
objc2 = "=0.6.4"
objc2-app-kit = {{ version = "=0.3.2", default-features = false, features = ["std", "NSView", "NSResponder"] }}
raw-window-handle = "0.6"
"#
            );
            fs::write(directory.join("Cargo.toml"), manifest).unwrap();
        }
        let output = Command::new(env!("CARGO"))
            .args(["build", "--workspace", "--offline", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(root.join("target"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture build failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Scratch(root)
    }
    fn classes() -> usize {
        AnyClass::classes()
            .iter()
            .filter(|class| {
                let name = class.name().to_bytes();
                name.starts_with(b"FrostDisplaySyncTarget_")
                    || name == b"FrostNiceTunerDisplaySyncTarget"
            })
            .count()
    }
    fn callback_lifetime() {
        use super::{Callback, target::Target};
        struct Witness(Rc<Cell<usize>>);
        impl Drop for Witness {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        let main = MainThreadMarker::new().unwrap();
        let view = NSView::new(main);
        let slot = Rc::new(RefCell::new(None::<Target>));
        let during_callback = slot.clone();
        let calls = Rc::new(Cell::new(0));
        let called = calls.clone();
        let drops = Rc::new(Cell::new(0));
        let witness = Witness(drops.clone());
        let target = Target::new(
            Callback::new(move || {
                called.set(called.get() + 1);
                during_callback.borrow_mut().take();
                assert_eq!(
                    witness.0.get(),
                    0,
                    "in-flight callback must retain its state"
                );
                true // stopping during invocation must override this return value
            }),
            main,
        )
        .unwrap();
        let object = target.object().retain();
        // SAFETY: our private target implements this exact display-link ABI.
        let link = unsafe { view.displayLinkWithTarget_selector(&object, sel!(frame:)) };
        *slot.borrow_mut() = Some(target);
        // SAFETY: the live private target and sender have the registered types.
        unsafe {
            let _: () = msg_send![&*object, frame: &*link];
        }
        assert_eq!(calls.get(), 1);
        assert!(slot.borrow().is_none());
        assert_eq!(
            drops.get(),
            1,
            "native retains must not keep Rust state alive"
        );
        // A native target may be retained after its Rust driver is gone. Its
        // cleared callback pointer must make even a late manual dispatch inert.
        // SAFETY: object and link remain retained with the same correct ABI.
        unsafe {
            let _: () = msg_send![&*object, frame: &*link];
        }
        assert_eq!(calls.get(), 1);

        let target = Target::new(Callback::new(|| panic!("native callback test")), main).unwrap();
        // SAFETY: as above, the target implements frame: for a CADisplayLink.
        let link = unsafe { view.displayLinkWithTarget_selector(target.object(), sel!(frame:)) };
        // SAFETY: both native objects are live and the method ABI matches.
        unsafe {
            let _: () = msg_send![target.object(), frame: &*link];
        }
        assert!(target.has_panicked());
        target.stop();
        link.invalidate();
    }
    pub fn run() {
        assert!(MainThreadMarker::new().is_some());
        if !objc2::available!(macos = 14.0) {
            println!("Native multi-library check not run: requires macOS 14 or newer");
            return;
        }
        autoreleasepool(|_| callback_lifetime());
        let scratch = fixtures();
        let baseline = classes();
        // Native autoreleases can outlive destruction of an editor and even
        // library unloading. Pool draining must not invoke a Rust destructor
        // from either unloaded fixture.
        autoreleasepool(|_| {
            let one = Library::open(&scratch.0.join("target/debug/libdisplay_fixture_one.dylib"));
            let a = one.instance();
            let b = one.instance();
            assert_eq!(
                classes(),
                baseline + 1,
                "one class per library, not per editor"
            );
            let two = Library::open(&scratch.0.join("target/debug/libdisplay_fixture_two.dylib"));
            let c = two.instance();
            let d = two.instance();
            assert_eq!(
                classes(),
                baseline + 2,
                "independent libraries must not share callback IMPs"
            );
            drop(a);
            assert_eq!((one.dropped)(), 1);
            drop(b);
            assert_eq!((one.dropped)(), 2);
            let reopened = one.instance();
            assert_eq!(
                classes(),
                baseline + 2,
                "reopening must reuse its own class"
            );
            drop(reopened);
            assert_eq!((one.dropped)(), 3);
            drop(c);
            drop(d);
            assert_eq!((two.dropped)(), 2);
            // one/two unload here, before the autorelease pool drains.
        });
        println!(
            "Two independent libraries, multiple instances, reopen, callback release and native unload passed"
        );
    }
}

#[cfg(target_os = "macos")]
fn main() {
    native::run();
}
#[cfg(not(target_os = "macos"))]
fn main() {
    println!("macOS-only multi-library display callback check");
}
