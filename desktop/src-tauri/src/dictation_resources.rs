//! One current-process reading per processing boundary; no worker or sampling timer.

#[derive(Clone, Debug)]
pub struct ProcessResources {
    pub working_set_bytes: Option<u64>,
    /// Cumulative kernel + user CPU time for the current process, in milliseconds.
    pub process_cpu_ms: Option<u64>,
}

#[cfg(not(target_os = "windows"))]
pub fn read_current_process() -> Option<ProcessResources> {
    None
}

#[cfg(target_os = "windows")]
pub fn read_current_process() -> Option<ProcessResources> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    fn ticks(value: FILETIME) -> u64 {
        ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
    }

    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let mut memory = PROCESS_MEMORY_COUNTERS {
        cb: size,
        ..Default::default()
    };
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: The pseudo-handle always refers to this process and must not be
    // closed. All output pointers refer to initialized, correctly sized values.
    let process = unsafe { GetCurrentProcess() };
    let working_set_bytes = unsafe { GetProcessMemoryInfo(process, &mut memory, size) }
        .is_ok()
        .then_some(memory.WorkingSetSize as u64);
    let process_cpu_ms =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) }
            .is_ok()
            .then(|| ticks(kernel).saturating_add(ticks(user)) / 10_000);
    Some(ProcessResources {
        working_set_bytes,
        process_cpu_ms,
    })
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn current_process_reading_has_memory_and_monotonic_cpu_without_a_sampler() {
        let before = read_current_process().expect("Windows supports process readings");
        let after = read_current_process().expect("Windows supports process readings");
        assert!(before.working_set_bytes.is_some_and(|bytes| bytes > 0));
        assert!(after.working_set_bytes.is_some_and(|bytes| bytes > 0));
        assert!(after.process_cpu_ms.unwrap() >= before.process_cpu_ms.unwrap());
    }
}
