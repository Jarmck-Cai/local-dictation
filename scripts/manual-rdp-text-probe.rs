// Appended inside the production Win32 module by check-protected-paste.py.
// Diagnostic only: publishes fixed text for 20 seconds; NEVER sends a key.
// prepare_fixed() reuses production text/marker allocation at the marker below.
impl WinClipboard {
    fn prepare_fixed(&mut self, text: &str) -> Result<(), &'static str> {
        // PRODUCTION_ALLOCATIONS
        let baseline = text_memory(PROBE_BASELINE)?;
        self.saved = vec![Saved {
            format: UNICODE_TEXT,
            data: Some(SavedData::Global(
                baseline,
                (PROBE_BASELINE.encode_utf16().count() + 1) * 2,
            )),
            source: HANDLE(std::ptr::null_mut()),
        }];
        self.restore_snapshot = true;
        self.held = Some(ClipboardLock::open(self.window.hwnd)?);
        self.current_sequence()?;
        Ok(())
    }
}

const PROBE_BASELINE: &str = "QINGSHU_TEST_CLIPBOARD_BASE_0123";
const PROBE_TEXT: &str = "轻述远程输入测试 RDP 0123";

pub(super) fn manual_probe(materialize: bool) -> Result<(), &'static str> {
    let mut clipboard = WinClipboard::new(false)?;
    // Does NOT call production prepare()/snapshot()/GetClipboardData: unknown
    // clipboard contents are discarded only after the user's explicit consent.
    clipboard.prepare_fixed(PROBE_TEXT)?;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        clipboard.publish()?;
        clipboard.begin_injection()?; // lease validation only; no key injection
        if materialize {
            let lock = ClipboardLock::open(clipboard.window.hwnd)?;
            unsafe { render(clipboard.window.hwnd, &clipboard.context, false); }
            lock.close()?;
            if clipboard.context.render_failed.get() { return Err(RENDER_FAILED); }
        }
        println!("ready=true manual_paste_window_seconds=20 automatic_keys=0 reads_existing_clipboard_payload=false materialized={}", materialize);
        let started = Instant::now();
        let mut announced = false;
        while started.elapsed() < Duration::from_secs(20) {
            let observation = clipboard.observe()?;
            if observation.ownership_lost { return Err("probe_new_copy_preserved"); }
            if observation.read && !announced {
                println!("local_delayed_render_read=true remote_insertion_confirmed=false");
                announced = true;
            }
            // Fixed diagnostic window, NOT a proposed production paste delay.
            thread::sleep(POLL);
        }
        Ok(())
    })).unwrap_or(Err("probe_panic"));
    // Exact production settlement: owner+sequence under lock, preserve new copy.
    clipboard.settle(false, &|| false)?;
    if clipboard.ownership_lost() {
        return Err("probe_new_copy_preserved");
    }
    outcome
}
