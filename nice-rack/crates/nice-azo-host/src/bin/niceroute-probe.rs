//! niceroute-probe — the command-line friend of the virtual device.
//!
//! ```text
//! niceroute-probe register [--dll PATH] [--real NAME | --no-real] [--hw-in N] [--hw-out N]
//! niceroute-probe unregister
//! niceroute-probe list
//! niceroute-probe cocreate
//! niceroute-probe solo [SECONDS] [--tone HZ]
//! niceroute-probe loopback [SECONDS] [--tone HZ]
//! ```
//!
//! `register`/`unregister`/`list` touch only the registry — safe next to
//! a live REAPER. `loopback` is the selftest: it opens the device **twice
//! in one process** (one instance becomes master, the other an ordinary
//! client, exactly as if they were two hosts), streams a quiet sine out
//! of one and measures it coming back in the other. Each open walks the
//! same `DllGetClassObject` → `IClassFactory::CreateInstance` → vtable
//! call sequence a real host takes, but driven through `azo`'s host
//! bindings on the outside — the ABI is checked by a second independent
//! implementation, not only by the one that authored it. No registration
//! is needed for the selftest (the factory is called directly), and
//! `NICEASIO_NO_HW` is set first, so the run can never open real hardware
//! or make a sound out of a speaker: the tone exists only inside the bus.

#[cfg(windows)]
mod app {
    use std::os::raw::c_void;
    use std::path::PathBuf;
    use std::ptr;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    use azo::driver::{Driver, SafeHandle, UnsafeHandle};
    use azo::dto::ChannelId;
    use azo::sys::{self, Bool, Callbacks, IIASIORedecl, Time};
    use azo::windows_core::{self as wc, Interface};

    use niceroute::{DRIVER_NAME, co_clsid, register};

    const USAGE: &str = concat!(
        "niceroute-probe register [--dll PATH] [--real NAME | --no-real] [--hw-in N] [--hw-out N]\n",
        "niceroute-probe unregister\n",
        "niceroute-probe list\n",
        "niceroute-probe hwblock [FRAMES]\n",
        "    (NICEASIO_HW_BLOCK=FRAMES outranks the registry for one run)\n",
        "niceroute-probe cocreate\n",
        "niceroute-probe solo [SECONDS] [--tone HZ]\n",
        "niceroute-probe loopback [SECONDS] [--tone HZ]\n",
    );

    pub fn main() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let code = match args.first().map(String::as_str) {
            Some("register") => cmd_register(&args[1..]),
            Some("unregister") => match register::unregister() {
                Ok(()) => {
                    println!("unregistered '{DRIVER_NAME}' (config kept for the next install)");
                    0
                }
                Err(e) => fail(&format!("unregister: {e}")),
            },
            Some("list") => {
                for d in register::enumerate_drivers() {
                    println!(
                        "{}  {}  [{}]",
                        if d.per_user { "HKCU" } else { "HKLM" },
                        d.name,
                        d.clsid
                    );
                }
                0
            }
            Some("hwblock") => cmd_hwblock(&args[1..]),
            Some("loopback") => cmd_loopback(&args[1..]),
            Some("cocreate") => cmd_cocreate(),
            Some("solo") => cmd_solo(&args[1..]),
            _ => {
                eprint!("{USAGE}");
                2
            }
        };
        std::process::exit(code);
    }

    fn fail(msg: &str) -> i32 {
        eprintln!("niceroute-probe: {msg}");
        1
    }

    /// Set (or with `0` clear) the frame count the bus's clock runs at,
    /// and read the answer back. This is the knob a "messy at 64, clean
    /// at 512" report turns: the hardware's block is the master's whole
    /// callback budget, and every client is paced by it, so a size the
    /// real driver is happy with on its own can still be a deadline
    /// nobody in the chain keeps. Living in the registry means the A/B
    /// needs a device reopen and nothing else — no host's preferences
    /// dialog, and no idea what its block size "really" was.
    fn cmd_hwblock(args: &[String]) -> i32 {
        // Worth saying out loud when it is on: the registry can still
        // read 64 while the run is being clocked at 256, and a log line
        // that disagrees with `hwblock` is a bad way to find that out.
        let shown = |n: u32| {
            format!(
                "{}{}",
                if n == 0 {
                    "the driver's preferred size".into()
                } else {
                    format!("{n} frames")
                },
                if register::env_hw_block().is_some() {
                    " (from NICEASIO_HW_BLOCK)"
                } else {
                    ""
                }
            )
        };
        let Some(raw) = args.first() else {
            println!("hwblock: {}", shown(register::Config::load().hw_block));
            return 0;
        };
        let Ok(n) = raw.parse::<u32>() else {
            return fail("hwblock wants a frame count (0 = driver default)");
        };
        if let Err(e) = register::set_hw_block(n) {
            return fail(&format!("hwblock: {e}"));
        }
        let back = register::Config::load().hw_block;
        println!("hwblock: {}", shown(back));
        println!("the next host to open the device picks it up; a running one does not.");
        0
    }

    // ------------------------------------------------------- register

    /// The DLL that ships next to whichever executable installs it.
    fn default_dll() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("niceroute.dll")))
            .unwrap_or_else(|| PathBuf::from("niceroute.dll"))
    }

    fn cmd_register(args: &[String]) -> i32 {
        let mut dll = default_dll();
        let mut real: Option<&str> = None;
        let mut hw_in = None;
        let mut hw_out = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--dll" => {
                    i += 1;
                    let Some(p) = args.get(i) else {
                        return fail("--dll wants a path");
                    };
                    dll = PathBuf::from(p);
                }
                "--real" => {
                    i += 1;
                    let Some(n) = args.get(i) else {
                        return fail("--real wants a driver name");
                    };
                    real = Some(n.to_string().leak());
                }
                "--no-real" => real = Some(""),
                "--hw-in" | "--hw-out" => {
                    let want_out = args[i] == "--hw-out";
                    i += 1;
                    let Some(n) = args.get(i).and_then(|s| s.parse::<u32>().ok()) else {
                        return fail("--hw-in/--hw-out want a number");
                    };
                    if want_out {
                        hw_out = Some(n);
                    } else {
                        hw_in = Some(n);
                    }
                }
                other => return fail(&format!("unknown flag '{other}'")),
            }
            i += 1;
        }
        if !dll.exists() {
            return fail(&format!(
                "device DLL not found at '{}' — point --dll at niceroute.dll",
                dll.display()
            ));
        }
        if let Err(e) = register::register(&dll, real, hw_in, hw_out) {
            return fail(&format!("register: {e}"));
        }
        println!("registered '{DRIVER_NAME}'");
        println!("  dll:     {}", dll.display());
        let config = register::Config::load();
        if let Some(name) = real.filter(|s| !s.is_empty()) {
            if register::clsid_for_name(name).is_none() {
                println!("  hosting: '{name}' — NOT in the registry; master will go virtual-only");
            } else {
                println!("  hosting: '{name}'");
            }
        } else {
            println!(
                "  hosting: {}",
                match config.real_driver.as_deref() {
                    Some(n) => format!("'{n}' (from config)"),
                    None => "nothing — virtual-only device".into(),
                }
            );
        }
        println!(
            "  channels: up to {} in / {} out of the hosted driver",
            config.hw_in, config.hw_out
        );
        println!(
            "plus 8 virtual inputs ('Nice 1'..'8') and 8 virtual outputs ('Nice Out 1'..'8')."
        );
        0
    }

    // ------------------------------------------------------- cocreate

    /// `solo`'s callback: fill every buffer, count blocks. Normally every
    /// buffer is silence; with `--tone` the first output channel carries
    /// a quiet sine, which is what the `NICEASIO_SILENT_HW` pin measures
    /// at the master's hardware stage. The pin is switched on together
    /// with the tone (see `cmd_solo`), so the tone stops inside the bus
    /// and never reaches a speaker.
    static S: Side = Side::new();
    /// Solo tone, in milli-Hz; zero means "output silence".
    static S_TONE_MHZ: AtomicU64 = AtomicU64::new(0);
    unsafe extern "system" fn s_switch(half: i32, _direct: Bool) {
        let half = half.max(0) as usize % 2;
        let block = S.block.load(Ordering::Relaxed) as usize;
        let tone_hz = S_TONE_MHZ.load(Ordering::Relaxed) as f64 / 1e3;
        let rate = f64::from_bits(S.rate.load(Ordering::Relaxed)).max(1.0);
        let frame0 = S.frames.fetch_add(block as u64, Ordering::Relaxed);
        for (ch, pair) in S.half().iter().enumerate() {
            let dst = unsafe { std::slice::from_raw_parts_mut(pair[half], block) };
            if ch == 0 && tone_hz > 0.0 {
                for (i, d) in dst.iter_mut().enumerate() {
                    let t = (frame0 + i as u64) as f64 / rate;
                    *d = 0.25 * (std::f64::consts::TAU * tone_hz * t).sin() as f32;
                }
            } else {
                dst.fill(0.0);
            }
        }
        S.blocks.fetch_add(1, Ordering::Relaxed);
    }
    unsafe extern "system" fn s_time_info(time: *mut Time, half: i32, direct: Bool) -> *mut Time {
        unsafe { s_switch(half, direct) };
        time
    }
    static S_CB: Callbacks = Callbacks {
        buffer_switch: s_switch,
        sample_rate_did_change: idle_rate,
        asio_message: refuse_message,
        buffer_switch_time_info: s_time_info,
    };

    /// The REAPER shape: one host, one instance, **no virtual-only pin**
    /// — first instance becomes master and does exactly what the
    /// registration says: host the configured RealDriver, or fall back
    /// to virtual-only if it is busy. Both directions in one
    /// `createBuffers`, start, run for SECONDS, stop. The callback
    /// cadence is the verdict; the driver's own note (what
    /// `getErrorMessage` would tell the host) is printed too.
    ///
    /// `--tone HZ` plays a quiet sine out of the first output channel,
    /// which is the only way to exercise the leg that ends at the real
    /// driver's own buffers. It implies `NICEASIO_SILENT_HW=1`: the
    /// master logs the peak it is about to send and sends silence, so
    /// the run stays inaudible and the proof lands in the trace log
    /// (`%TEMP%\niceroute.log`) as a number — `hw-out peak 0.250…`.
    fn cmd_solo(args: &[String]) -> i32 {
        let mut secs: u64 = 3;
        let mut tone_hz: f64 = 0.0;
        let mut want_block: Option<i32> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--tone" => {
                    i += 1;
                    let Some(Ok(n)) = args.get(i).map(|s| s.parse::<f64>()) else {
                        return fail("--tone wants a frequency");
                    };
                    tone_hz = n.clamp(20.0, 20000.0);
                }
                // Ask for a block the driver did not offer as its
                // preference. A host picks its own size from its own
                // preferences — NiceRack runs at 64 because that is what
                // its settings page says — and a client smaller than the
                // hardware's quantum is a different shape of day for the
                // bus than one that matches it, so the mismatch has to be
                // runnable from one command line to be knowable at all.
                "--block" => {
                    i += 1;
                    let Some(Ok(n)) = args.get(i).map(|s| s.parse::<i32>()) else {
                        return fail("--block wants a frame count");
                    };
                    want_block = Some(n);
                }
                other => match other.parse::<u64>() {
                    Ok(n) if (1..=120).contains(&n) => secs = n,
                    _ => return fail(&format!("unexpected argument '{other}'")),
                },
            }
            i += 1;
        }
        if tone_hz > 0.0 {
            // A tone this host really would have played must never leave
            // the building: measure it, then mute it at the driver door.
            unsafe { std::env::set_var("NICEASIO_SILENT_HW", "1") };
            S_TONE_MHZ.store((tone_hz * 1e3) as u64, Ordering::Relaxed);
        }
        let a = match open("solo") {
            Ok(h) => h,
            Err(e) => return fail(&e),
        };
        if !a.init(None) {
            let err = a.last_error();
            return fail(&format!("init refused: {}", err.to_string_lossy()));
        }
        let counts = match a.channel_counts() {
            Ok(c) => c,
            Err(e) => return fail(&format!("getChannels: {e:?}")),
        };
        let rate = match a.get_sample_rate() {
            Ok(r) => r,
            Err(e) => return fail(&format!("getSampleRate: {e:?}")),
        };
        let sizes = match a.buffer_size() {
            Ok(s) => s,
            Err(e) => return fail(&format!("getBufferSize: {e:?}")),
        };
        let block = want_block
            .unwrap_or(sizes.preferred)
            .clamp(sizes.min.max(32), sizes.max.max(sizes.min));
        println!(
            "solo: {} in / {} out, {rate} Hz, block {block}",
            counts.in_, counts.out
        );
        S.block.store(block as u64, Ordering::Relaxed);
        S.rate.store(rate.max(1.0).to_bits(), Ordering::Relaxed);
        let ids: Vec<ChannelId> = (0..counts.out.max(1))
            .map(|index| ChannelId {
                input: false,
                index,
            })
            .chain((0..counts.in_.max(1)).map(|index| ChannelId { input: true, index }))
            .collect();
        let bufs = match create_channels("solo", &a, &S_CB, &ids, block) {
            Ok(v) => v,
            Err(e) => return fail(&e),
        };
        *S.bufs.lock().unwrap() = bufs;
        if let Err(e) = a.start() {
            let note = a.last_error();
            return fail(&format!(
                "start refused: {e:?} — driver note: '{}'",
                note.to_string_lossy()
            ));
        }
        let t0 = Instant::now();
        std::thread::sleep(std::time::Duration::from_secs(secs));
        let elapsed = t0.elapsed().as_secs_f64();
        let _ = a.stop();
        let blocks = S.blocks.load(Ordering::Relaxed);
        let expected = rate * elapsed / block as f64;
        let note = a.last_error();
        println!("solo note: '{}'", note.to_string_lossy());
        println!("solo: {blocks} callbacks in {elapsed:.1} s (~{expected:.0} expected)");
        if tone_hz > 0.0 {
            println!("solo: {tone_hz} Hz at 0.25 on output 1, muted at the driver door");
            println!("      the master's side of the story is in the trace log:");
            println!(
                "      '{}' — expect 'hw-out peak 0.250…'",
                niceroute::trace::path().display()
            );
        }
        let _ = a.dispose_buffers();
        let ok = blocks as f64 >= 0.5 * expected.max(1.0);
        println!("solo {}", if ok { "PASS" } else { "FAIL" });
        if ok { 0 } else { 1 }
    }

    /// The hop the loopback deliberately skips: activate the device the
    /// way a host does — `CoCreateInstance` with our CLSID, letting COM
    /// resolve `HKCU\Software\Classes\CLSID`, load the DLL, and hand
    /// back the driver interface. This is the test that a *registration*
    /// (any registration) is actually wired, not just a buildable DLL;
    /// `loopback` would happily pass with an unregistered device.
    /// Silent: `NICEASIO_NO_HW` pins the bus virtual-only, so the
    /// activation — and the master role it may take — can never touch
    /// hardware.
    fn cmd_cocreate() -> i32 {
        unsafe { std::env::set_var("NICEASIO_NO_HW", "1") };
        let guid = co_clsid();
        let handle = match SafeHandle::new(&guid) {
            Ok(h) => h,
            Err(e) => {
                return fail(&format!(
                    "CoCreateInstance({CLSID}): {e} — is the device registered?",
                    CLSID = niceroute::CLSID_STR
                ));
            }
        };
        if !handle.init(None) {
            return fail("init refused");
        }
        let counts = match handle.channel_counts() {
            Ok(c) => c,
            Err(e) => return fail(&format!("getChannels: {e:?}")),
        };
        let rate = handle.get_sample_rate().unwrap_or(0.0);
        println!(
            "'{DRIVER_NAME}' answered CoCreateInstance: {} in / {} out, {rate} Hz",
            counts.in_, counts.out
        );
        if counts.in_ == 0 || counts.out == 0 {
            return fail("the driver answered with a channel count no host can use");
        }
        println!("cocreate PASS");
        // Dropping the handle releases the COM object; the DLL stops
        // its instance — and the master role — on the way out.
        0
    }

    // ------------------------------------------------------- loopback

    /// One instance's worth of host-side state, written by its
    /// `bufferSwitch` callback and read by the report.
    struct Side {
        blocks: AtomicU64,
        frames: AtomicU64,
        /// f64 bit patterns: energy and peak of the measured channels.
        sumsq0: AtomicU64,
        sumsq1: AtomicU64,
        peak0: AtomicU64,
        peak1: AtomicU64,
        block: AtomicU64,
        rate: AtomicU64,
        bufs: Mutex<Vec<[*mut f32; 2]>>,
    }

    impl Side {
        const fn new() -> Side {
            Side {
                blocks: AtomicU64::new(0),
                frames: AtomicU64::new(0),
                sumsq0: AtomicU64::new(0),
                sumsq1: AtomicU64::new(0),
                peak0: AtomicU64::new(0),
                peak1: AtomicU64::new(0),
                block: AtomicU64::new(128),
                rate: AtomicU64::new(48000),
                bufs: Mutex::new(Vec::new()),
            }
        }
        fn half(&self) -> Vec<[*mut f32; 2]> {
            self.bufs.lock().unwrap().clone()
        }
    }

    // The pointers behind the mutex are device-owned buffer halves; the
    // audio threads that read them are exactly what they are for.
    unsafe impl Sync for Side {}

    /// Empty an energy/peak pair into plain numbers for the report.
    fn drain(sumsq: &AtomicU64, peak: &AtomicU64) -> (f64, f64) {
        (
            f64::from_bits(sumsq.swap(0, Ordering::AcqRel)),
            f64::from_bits(peak.swap(0, Ordering::AcqRel)),
        )
    }

    static A: Side = Side::new();
    /// A's own input buffers, measured to prove the bus never feeds a
    /// client back its own output (a REAPER on this device must be able
    /// to monitor the bus without launching a howling loop).
    static A_IN: Side = Side::new();
    static B: Side = Side::new();
    /// Sine frequency, in milli-Hz, so it fits an integer atom.
    static TONE_MHZ: AtomicU64 = AtomicU64::new(440_000);

    fn add_f64(slot: &AtomicU64, v: f64) {
        let mut cur = slot.load(Ordering::Relaxed);
        loop {
            let next = f64::from_bits(cur) + v;
            match slot.compare_exchange_weak(
                cur,
                next.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(c) => cur = c,
            }
        }
    }

    fn max_f64(slot: &AtomicU64, v: f64) {
        let mut cur = slot.load(Ordering::Relaxed);
        while v > f64::from_bits(cur) {
            match slot.compare_exchange_weak(cur, v.to_bits(), Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(c) => cur = c,
            }
        }
    }

    /// Energy and peak of one input half-buffer, accumulated into a
    /// side's atomics while the audio thread runs.
    unsafe fn measure_input(
        bufs: &[[*mut f32; 2]],
        ch: usize,
        half: usize,
        block: usize,
        sumsq: &AtomicU64,
        peak: &AtomicU64,
    ) {
        let Some(pair) = bufs.get(ch) else { return };
        let src = unsafe { std::slice::from_raw_parts(pair[half], block) };
        let mut e = 0.0f64;
        for &s in src {
            let a = f64::from(s);
            e += a * a;
            max_f64(peak, a.abs());
        }
        add_f64(sumsq, e);
    }

    /// Instance A: the sender. A quiet sine on output 0, silence on the
    /// rest — the silence doubles as the crosstalk control. A also
    /// listens to its own inputs: with B emitting nothing, anything
    /// arriving there is A's own output coming back, which the device
    /// must never do.
    unsafe extern "system" fn a_switch(half: i32, _direct: Bool) {
        let half = half.max(0) as usize % 2;
        let block = A.block.load(Ordering::Relaxed) as usize;
        let rate = f64::from_bits(A.rate.load(Ordering::Relaxed)).max(1.0);
        let freq = TONE_MHZ.load(Ordering::Relaxed) as f64 / 1e3;
        let frame0 = A.frames.fetch_add(block as u64, Ordering::Relaxed);
        for (ch, pair) in A.half().iter().enumerate() {
            let dst = unsafe { std::slice::from_raw_parts_mut(pair[half], block) };
            if ch == 0 {
                for (i, d) in dst.iter_mut().enumerate() {
                    let t = (frame0 + i as u64) as f64 / rate;
                    *d = 0.25 * (std::f64::consts::TAU * freq * t).sin() as f32;
                }
            } else {
                dst.fill(0.0);
            }
        }
        let in_bufs = A_IN.half();
        unsafe { measure_input(&in_bufs, 0, half, block, &A_IN.sumsq0, &A_IN.peak0) };
        A.blocks.fetch_add(1, Ordering::Relaxed);
    }

    /// Instance B: the listener. Measures virtual input 1 (A's output 1
    /// through the bus) and input 2, which carries nothing and must stay
    /// exactly at zero.
    unsafe extern "system" fn b_switch(half: i32, _direct: Bool) {
        let half = half.max(0) as usize % 2;
        let block = B.block.load(Ordering::Relaxed) as usize;
        let bufs = B.half();
        unsafe { measure_input(&bufs, 0, half, block, &B.sumsq0, &B.peak0) };
        unsafe { measure_input(&bufs, 1, half, block, &B.sumsq1, &B.peak1) };
        B.blocks.fetch_add(1, Ordering::Relaxed);
    }

    unsafe extern "system" fn idle_rate(_rate: f64) {}
    unsafe extern "system" fn refuse_message(
        _sel: sys::MessageSelector,
        _v: i32,
        _m: *const c_void,
        _o: *const f64,
    ) -> i32 {
        0
    }
    unsafe extern "system" fn a_time_info(time: *mut Time, half: i32, direct: Bool) -> *mut Time {
        unsafe { a_switch(half, direct) };
        time
    }
    unsafe extern "system" fn b_time_info(time: *mut Time, half: i32, direct: Bool) -> *mut Time {
        unsafe { b_switch(half, direct) };
        time
    }

    static A_CB: Callbacks = Callbacks {
        buffer_switch: a_switch,
        sample_rate_did_change: idle_rate,
        asio_message: refuse_message,
        buffer_switch_time_info: a_time_info,
    };
    static B_CB: Callbacks = Callbacks {
        buffer_switch: b_switch,
        sample_rate_did_change: idle_rate,
        asio_message: refuse_message,
        buffer_switch_time_info: b_time_info,
    };

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const i8) -> *const c_void;
        fn GetLastError() -> u32;
    }

    /// `DllGetClassObject`'s C signature; the iid/clsid pointers are
    /// `GUID`s read through, so `c_void` keeps the binding honest about
    /// "opaque struct pointer" without naming a type.
    type GetClassObject = unsafe extern "system" fn(
        rclsid: *const c_void,
        riid: *const c_void,
        ppv: *mut *mut c_void,
    ) -> i32;

    /// `IClassFactory`, spelled as the vtable slots it actually is:
    /// QueryInterface/AddRef/Release/CreateInstance/LockServer. The
    /// index applies to the *vtable* — the object's first word — not to
    /// the object itself, which is the difference between a slot call
    /// and whatever a deref-precedence slip reads instead.
    unsafe fn vtable_slot(this: *mut c_void, index: usize) -> usize {
        let vtable: *const usize = unsafe { *(this as *const *const usize) };
        unsafe { *vtable.add(index) }
    }

    /// `IID_IClassFactory`, by its bytes: this probe deliberately shares
    /// no generated constants with anything.
    static IID_ICLASS_FACTORY: wc::GUID = wc::GUID {
        data1: 0x0000_0001,
        data2: 0,
        data3: 0,
        data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46],
    };

    fn load_device() -> Result<*mut c_void, String> {
        let path = default_dll();
        let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain([0]).collect();
        let module = unsafe { LoadLibraryW(wide.as_ptr()) };
        if module.is_null() {
            return Err(format!(
                "LoadLibraryW('{}') failed: {}",
                path.display(),
                unsafe { GetLastError() }
            ));
        }
        Ok(module)
    }

    /// Open the device the way COM opens it — `DllGetClassObject` on the
    /// real `niceroute.dll`, `IClassFactory::CreateInstance`, the CLSID
    /// doubling as the interface id — and hand back an `azo` host handle,
    /// so every call after this one crosses the hand-written vtable from
    /// the side that did not write it.
    fn open(who: &str) -> Result<UnsafeHandle, String> {
        let module = load_device()?;
        let entry = unsafe { GetProcAddress(module, c"DllGetClassObject".as_ptr().cast::<i8>()) };
        if entry.is_null() {
            return Err(format!("{who}: niceroute.dll exports no DllGetClassObject"));
        }
        let get_class_object: GetClassObject = unsafe { std::mem::transmute(entry) };
        let clsid = co_clsid();
        let mut factory: *mut c_void = ptr::null_mut();
        let code = unsafe {
            get_class_object(
                ptr::addr_of!(clsid).cast::<c_void>(),
                ptr::addr_of!(IID_ICLASS_FACTORY).cast::<c_void>(),
                &mut factory,
            )
        };
        if code != 0 || factory.is_null() {
            return Err(format!("{who}: DllGetClassObject -> {code:#010X}"));
        }
        let create_instance: unsafe extern "system" fn(
            this: *mut c_void,
            outer: *mut c_void,
            iid: *const c_void,
            obj: *mut *mut c_void,
        ) -> i32 = unsafe { std::mem::transmute(vtable_slot(factory, 3)) };
        let release: unsafe extern "system" fn(this: *mut c_void) -> u32 =
            unsafe { std::mem::transmute(vtable_slot(factory, 2)) };
        let mut object: *mut c_void = ptr::null_mut();
        let code = unsafe {
            create_instance(
                factory,
                ptr::null_mut(),
                ptr::addr_of!(clsid).cast::<c_void>(),
                &mut object,
            )
        };
        // The factory is immortal, but a good client still says thanks.
        unsafe { release(factory) };
        if code != 0 || object.is_null() {
            return Err(format!("{who}: CreateInstance -> {code:#010X}"));
        }
        // Safety: `CreateInstance` returned a pointer owning one strong
        // reference; `from_raw` adopts exactly that reference, and the
        // handle's drop releases it — the contract azo itself honors.
        let handle = unsafe { UnsafeHandle(IIASIORedecl::from_raw(object)) };
        if !handle.init(None) {
            let err = handle.last_error();
            return Err(format!("{who}: init refused: {}", err.to_string_lossy()));
        }
        Ok(handle)
    }

    /// One `createBuffers` call — the ASIO contract allows exactly one
    /// per device — and hand back the buffer halves in `ids` order.
    fn create_channels(
        who: &str,
        handle: &impl Driver,
        cb: &'static Callbacks,
        ids: &[ChannelId],
        block: i32,
    ) -> Result<Vec<[*mut f32; 2]>, String> {
        let buffers: Vec<[*mut c_void; 2]> =
            unsafe { handle.create_buffers(ids.iter().copied(), block, ptr::addr_of!(*cb)) }
                .map_err(|e| format!("{who}: createBuffers: {e:?}"))?
                .collect();
        if buffers.len() != ids.len() {
            return Err(format!("{who}: createBuffers returned {}", buffers.len()));
        }
        Ok(buffers
            .iter()
            .map(|h| [h[0] as *mut f32, h[1] as *mut f32])
            .collect())
    }

    fn open_channels(
        who: &str,
        handle: &impl Driver,
        side: &'static Side,
        cb: &'static Callbacks,
        input: bool,
        count: i32,
        block: i32,
    ) -> Result<(), String> {
        let ids: Vec<ChannelId> = (0..count).map(|index| ChannelId { input, index }).collect();
        *side.bufs.lock().unwrap() = create_channels(who, handle, cb, &ids, block)?;
        Ok(())
    }

    fn cmd_loopback(args: &[String]) -> i32 {
        let mut secs: u64 = 5;
        let mut tone_hz: f64 = 440.0;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--tone" => {
                    i += 1;
                    let Some(Ok(n)) = args.get(i).map(|s| s.parse::<f64>()) else {
                        return fail("--tone wants a frequency");
                    };
                    tone_hz = n.clamp(20.0, 20000.0);
                }
                other => match other.parse::<u64>() {
                    Ok(n) if (1..=120).contains(&n) => secs = n,
                    _ => return fail(&format!("unexpected argument '{other}'")),
                },
            }
            i += 1;
        }

        // The safety pin: even if this user's master is configured to host
        // real hardware, this process's master goes virtual-only, so the
        // probe cannot open a device somebody else is holding, and cannot
        // make a sound outside the bus.
        unsafe { std::env::set_var("NICEASIO_NO_HW", "1") };
        TONE_MHZ.store((tone_hz * 1e3) as u64, Ordering::Relaxed);

        let a = match open("A") {
            Ok(h) => h,
            Err(e) => return fail(&e),
        };
        // The first instance in this process is now the master; whatever
        // it decided (virtual-only, per the pin above) is the second
        // instance's world.
        let counts = match a.channel_counts() {
            Ok(c) => c,
            Err(e) => return fail(&format!("A: getChannels: {e:?}")),
        };
        let rate = match a.get_sample_rate() {
            Ok(r) => r,
            Err(e) => return fail(&format!("A: getSampleRate: {e:?}")),
        };
        let sizes = match a.buffer_size() {
            Ok(s) => s,
            Err(e) => return fail(&format!("A: getBufferSize: {e:?}")),
        };
        let block = sizes
            .preferred
            .clamp(sizes.min.max(32), sizes.max.max(sizes.min));
        println!(
            "A: '{}' reports {} in / {} out, {} Hz, block {}",
            a.name().to_string_lossy(),
            counts.in_,
            counts.out,
            rate,
            block
        );

        let b = match open("B") {
            Ok(h) => h,
            Err(e) => return fail(&e),
        };
        let b_counts = match b.channel_counts() {
            Ok(c) => c,
            Err(e) => return fail(&format!("B: getChannels: {e:?}")),
        };
        let b_rate = b.get_sample_rate().unwrap_or(0.0);
        println!(
            "B: second open reports {} in / {} out, {} Hz {}",
            b_counts.in_,
            b_counts.out,
            b_rate,
            if (b_rate - rate).abs() < 0.5 {
                "(same)"
            } else {
                "(DIFFERENT — FAIL)"
            }
        );

        for side in [&A, &B] {
            side.block.store(block as u64, Ordering::Relaxed);
            side.rate.store(rate.max(1.0).to_bits(), Ordering::Relaxed);
        }
        // A's outputs and A's own inputs in one `createBuffers` call
        // (the one-call-per-device contract); the inputs exist to catch
        // an echo of A's own tone coming back down the bus.
        let a_outs = counts.out.max(2);
        let a_ins = counts.in_.max(2);
        let a_ids: Vec<ChannelId> = (0..a_outs)
            .map(|index| ChannelId {
                input: false,
                index,
            })
            .chain((0..a_ins).map(|index| ChannelId { input: true, index }))
            .collect();
        let a_bufs = match create_channels("A", &a, &A_CB, &a_ids, block) {
            Ok(v) => v,
            Err(e) => return fail(&e),
        };
        let (a_out_bufs, a_in_bufs) = a_bufs.split_at(a_outs as usize);
        *A.bufs.lock().unwrap() = a_out_bufs.to_vec();
        *A_IN.bufs.lock().unwrap() = a_in_bufs.to_vec();
        if let Err(e) = open_channels("B", &b, &B, &B_CB, true, b_counts.in_.max(2), block) {
            return fail(&e);
        }
        if let Err(e) = a.start().and_then(|()| b.start()) {
            return fail(&format!("start: {e:?}"));
        }

        println!("streaming {secs} s of {tone_hz} Hz at 0.25 amplitude through the bus...");
        let t0 = Instant::now();
        std::thread::sleep(std::time::Duration::from_secs(secs));
        let elapsed = t0.elapsed().as_secs_f64();
        let _ = a.stop();
        let _ = b.stop();

        let a_blocks = A.blocks.load(Ordering::Relaxed);
        let b_blocks = B.blocks.load(Ordering::Relaxed);
        let frames_b = b_blocks.saturating_mul(block as u64).max(1);
        let (ss0, pk0) = drain(&B.sumsq0, &B.peak0);
        let (ss1, pk1) = drain(&B.sumsq1, &B.peak1);
        let rms0 = (ss0 / frames_b as f64).sqrt();
        let rms1 = (ss1 / frames_b as f64).sqrt();
        // A's own inputs: B emits nothing, so anything here is A's own
        // tone returned by the bus — the one thing it must never get.
        let frames_a = a_blocks.saturating_mul(block as u64).max(1);
        let (ain_ss, ain_pk) = drain(&A_IN.sumsq0, &A_IN.peak0);
        let ain_rms = (ain_ss / frames_a as f64).sqrt();

        let expected = (rate * elapsed / block as f64).max(1.0);
        let want_rms = 0.25 / std::f64::consts::SQRT_2;
        let checks: &[(&str, bool)] = &[
            (
                &format!("A callbacks {a_blocks} of ~{expected:.0}"),
                a_blocks as f64 >= 0.5 * expected,
            ),
            (
                &format!("B callbacks {b_blocks} of ~{expected:.0}"),
                b_blocks as f64 >= 0.5 * expected,
            ),
            (
                &format!("B input 1 RMS {rms0:.4}, expected ~{want_rms:.4}"),
                (0.5..=2.0).contains(&(rms0 / want_rms)),
            ),
            (
                &format!("B input 2 RMS {rms1:.6} / peak {pk1:.6} (crosstalk)"),
                pk1 == 0.0,
            ),
            (&format!("B input 1 peak {pk0:.4}"), pk0 > 0.1),
            (
                &format!("A hears only others: own input 1 RMS {ain_rms:.6} / peak {ain_pk:.6}"),
                ain_pk == 0.0,
            ),
        ];
        let mut ok = true;
        for (line, passed) in checks {
            println!("  {} {line}", if *passed { "ok  " } else { "FAIL" });
            ok &= passed;
        }
        // Leave the device the way we found it: buffers disposed before
        // the handles drop, master role ending with this process.
        let _ = a.dispose_buffers();
        let _ = b.dispose_buffers();
        drop(b);
        drop(a);
        println!("loopback {}", if ok { "PASS" } else { "FAIL" });
        if ok { 0 } else { 1 }
    }
}

#[cfg(windows)]
fn main() {
    app::main();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("niceroute-probe is a Windows tool: the device it drives is a COM server.");
    std::process::exit(2);
}
