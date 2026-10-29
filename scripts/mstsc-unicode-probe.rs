// Appended to a frozen copy of the production native adapter. Build-only by
// default; the valid mode requires a separately agreed remote editor window.
pub(crate) fn fixed_probe(pid: u32) -> Result<(), String> {
    crate::keyboard_state_windows::start()?;
    println!("countdown_seconds=5 fixed_case=U2 one_unicode_batch=true clipboard_untouched=true");
    std::thread::sleep(std::time::Duration::from_secs(5));
    // Let the user return from the confirmation UI before capturing the actual
    // current output target. The final production gates still run at submission.
    let target = capture()?.ok_or("probe_not_system_mstsc")?;
    if target.pid != pid {
        return Err("probe_target_pid_changed".into());
    }
    if !target.current() {
        return Err("input_target_changed".into());
    }
    submit(
        target,
        "轻述远程输入测试 RDP U2 😀 𠀀",
        false,
        || false,
        || crate::keyboard_state_windows::prepare(0),
        |prepared| match crate::keyboard_state_windows::fast_check(prepared, 0)? {
            true => Err("keys_held"),
            false => Ok(()),
        },
    )?;
    println!("os_accepted_all=true remote_insertion_unconfirmed=true clipboard_api_calls=0");
    Ok(())
}
