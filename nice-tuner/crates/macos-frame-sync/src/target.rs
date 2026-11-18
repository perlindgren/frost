//! A per-library class with a non-owning pointer to per-editor callback state.
use super::Callback;
use objc2::{
    ClassType, MainThreadMarker, Message, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, ClassBuilder, Sel},
    sel,
};
use objc2_foundation::NSObject;
use objc2_quartz_core::CADisplayLink;
use std::{
    ffi::{CString, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    rc::Rc,
    sync::OnceLock,
};

static CLASS: OnceLock<Result<&'static AnyClass, String>> = OnceLock::new();

fn register_class(identity: usize) -> Result<&'static AnyClass, String> {
    // The static's address distinguishes separately linked copies, including
    // different builds of one plugin. A suffix handles an occupied name after
    // unload/reload at the same address, without adopting foreign method IMPs.
    for generation in 0..128 {
        let name = CString::new(format!("FrostDisplaySyncTarget_{identity:x}_{generation}"))
            .map_err(|error| error.to_string())?;
        if AnyClass::get(&name).is_some() {
            continue;
        }
        let Some(mut builder) = ClassBuilder::new(&name, NSObject::class()) else {
            continue;
        };
        builder.add_ivar::<*const c_void>(c"frostCallback");
        // SAFETY: frame has Objective-C's receiver/selector arguments, a void
        // return and one CADisplayLink argument. The private class inherits
        // NSObject's initialization and destruction without Rust drop hooks.
        unsafe {
            builder.add_method(
                sel!(frame:),
                frame as unsafe extern "C" fn(*mut AnyObject, Sel, *mut CADisplayLink),
            );
        }
        return Ok(builder.register());
    }
    Err("Could not register a unique display callback class".into())
}

fn class() -> Result<&'static AnyClass, String> {
    CLASS
        .get_or_init(|| register_class(ptr::addr_of!(CLASS) as usize))
        .as_ref()
        .copied()
        .map_err(Clone::clone)
}

pub(super) struct Target {
    object: Retained<AnyObject>,
    // Rc makes Target and its owning DisplaySync !Send and !Sync. Only this
    // Rust owner and an in-flight frame invocation own the callback state.
    callback: Rc<Callback>,
}

impl Target {
    pub(super) fn new(callback: Callback, _main: MainThreadMarker) -> Result<Self, String> {
        let class = class()?;
        // SAFETY: this is our NSObject subclass, with no custom new/init.
        // Its pointer ivar starts zeroed; no method is invoked until initialized.
        let object: Option<Retained<AnyObject>> = unsafe { msg_send![class, new] };
        let object = object.ok_or("Could not allocate display callback target")?;
        let callback = Rc::new(callback);
        set_callback(&object, Rc::as_ptr(&callback).cast());
        Ok(Self { object, callback })
    }

    pub(super) fn object(&self) -> &AnyObject {
        &self.object
    }

    pub(super) fn has_panicked(&self) -> bool {
        self.callback.panicked.get()
    }

    pub(super) fn stop(&self) {
        self.callback.stopped.set(true);
        set_callback(&self.object, ptr::null());
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        // Detach before dropping the Rc, even if native code retains the target
        // until a later autorelease-pool drain. Native dealloc runs no Rust code.
        self.stop();
    }
}

fn set_callback(object: &AnyObject, callback: *const c_void) {
    let ivar = object
        .class()
        .instance_variable(c"frostCallback")
        .expect("our target ivar");
    // SAFETY: only our private target class reaches this function. The ivar was
    // declared with this exact pointer type. Creation, reads and writes happen
    // on the main thread; no reference to the ivar is kept across a write.
    unsafe { ivar.load_ptr::<*const c_void>(object).write(callback) };
}

unsafe extern "C" fn frame(object: *mut AnyObject, _: Sel, link: *mut CADisplayLink) {
    // SAFETY: Objective-C calls this registered IMP with its live receiver and
    // CADisplayLink sender. Neither reference escapes the invocation.
    let (object, link) = unsafe { (&*object, &*link) };
    // Both native arguments must survive a callback dropping its DisplaySync.
    let _target = object.retain();
    let _link = link.retain();
    let result = catch_unwind(AssertUnwindSafe(|| {
        if MainThreadMarker::new().is_none() {
            return false;
        }
        let Some(ivar) = object.class().instance_variable(c"frostCallback") else {
            return false;
        };
        // SAFETY: this method is registered only on our class, with the exact
        // ivar type below. A stopped target contains null. Otherwise Target
        // still owns an Rc to Callback; main-thread dispatch cannot race Drop.
        let pointer = unsafe { *ivar.load::<*const c_void>(object) }.cast::<Callback>();
        if pointer.is_null() {
            return false;
        }
        // SAFETY: the live Target owns this Rc allocation as established above.
        // Take a separate strong reference before invoking user code, which may
        // drop Target. from_raw consumes precisely the increment performed here.
        let callback = unsafe {
            Rc::increment_strong_count(pointer);
            Rc::from_raw(pointer)
        };
        callback.invoke()
    }));
    // The outer catch also contains panicking captured-value destructors when
    // the driver was dropped from its own callback and this was the last Rc.
    match result {
        Ok(true) => {}
        Ok(false) => link.invalidate(),
        Err(payload) => {
            // An unexpected panic payload may itself have a panicking Drop.
            // Do not run it across the native boundary. This rare path stops
            // the link, so at most one payload is leaked per failed driver.
            std::mem::forget(payload);
            link.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_name_is_not_reused() {
        let identity = an_existing_name_is_not_reused as *const () as usize;
        let first = register_class(identity).unwrap();
        let second = register_class(identity).unwrap();
        assert!(!ptr::eq(first, second));
        assert_ne!(first.name(), second.name());
    }

    #[test]
    fn library_registration_is_cached() {
        assert!(ptr::eq(class().unwrap(), class().unwrap()));
    }
}
