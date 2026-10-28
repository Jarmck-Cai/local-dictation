//! Coalesced background reconciliation. No app-data/body logging or network work.
use super::{collected_store, with_connection};
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Mutex,
};
use tauri::{AppHandle, Emitter};

#[derive(Default)]
struct Worker {
    running: bool,
    pending: bool,
}
static WORKER: Mutex<Worker> = Mutex::new(Worker {
    running: false,
    pending: false,
});
static EMITTED_REVISION: AtomicI64 = AtomicI64::new(-1);

/// Root calls after startup/history events/expiry. Page reads also provide a fallback.
/// Every call is nonblocking; only one worker reconciles, in bounded transactions.
pub fn schedule_collected_terms_refresh(app: &AppHandle) {
    let Ok(mut worker) = WORKER.lock() else {
        return;
    };
    worker.pending = true;
    if worker.running {
        return;
    }
    worker.running = true;
    drop(worker);
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || loop {
        if let Ok(mut worker) = WORKER.lock() {
            worker.pending = false;
        }
        let result = (|| -> Result<(), String> {
            loop {
                let processed =
                    with_connection(&app, |db| collected_store::backfill_batch(db, 16))?;
                if processed == 0 {
                    break;
                }
                std::thread::yield_now();
            }
            let revision = with_connection(&app, |db| collected_store::revision(db))?;
            if EMITTED_REVISION.swap(revision, Ordering::AcqRel) != revision {
                // All writes committed before the event; unchanged reads never loop.
                let _ = app.emit("collected-terms-updated", ());
            }
            Ok(())
        })();
        if result.is_err() {
            log::warn!("Could not reconcile collected terms");
        }
        let Ok(mut worker) = WORKER.lock() else {
            break;
        };
        if worker.pending {
            continue;
        }
        worker.running = false;
        break;
    });
}
