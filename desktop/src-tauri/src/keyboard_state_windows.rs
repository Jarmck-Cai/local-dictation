//! Pass-through Raw Input observer. Only high-function held bits survive a call.
use super::keyboard_state_policy::{self as policy, Decision, State};
use std::cell::{Cell, RefCell};
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::Input::{
    GetRawInputData, GetRegisteredRawInputDevices, RegisterRawInputDevices, HRAWINPUT, RAWINPUT,
    RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_DEVNOTIFY, RIDEV_INPUTSINK, RIDEV_REMOVE, RID_INPUT,
    RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    GetMessageW, GetQueueStatus, GetWindowLongPtrW, PeekMessageW, PostMessageW, RegisterClassW,
    SetWindowLongPtrW, UnregisterClassW, EVENT_SYSTEM_DESKTOPSWITCH, GWLP_USERDATA, HWND_MESSAGE,
    MSG, PM_REMOVE, QS_RAWINPUT, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_INPUT,
    WM_INPUT_DEVICE_CHANGE, WM_QUIT, WM_WTSSESSION_CHANGE, WNDCLASSW,
};

const UNAVAILABLE: &str = "keyboard_unavailable";
const QUERY_TIMEOUT: Duration = Duration::from_millis(40);
const TOKEN_LIFETIME: Duration = Duration::from_millis(100);
const WAKE: u32 = WM_APP + 103;
const DRAIN_LIMIT: usize = 256;
static OBSERVER: OnceLock<Result<Observer, &'static str>> = OnceLock::new();
static NEXT_QUERY: AtomicU64 = AtomicU64::new(1);
thread_local! { static DESKTOP_RESET: Cell<bool> = const { Cell::new(false) }; }

struct Observer {
    hwnd: usize,
    requests: mpsc::Sender<Request>,
    generation: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
}
struct Request {
    id: u64,
    bound: u16,
    deadline: Instant,
    reply: mpsc::SyncSender<(u64, Result<Prepared, &'static str>)>,
}
/// Short-lived evidence prepared before the transaction's final lease check.
pub(crate) struct Prepared {
    generation: u64,
    high: u16,
    at: Instant,
}
struct Context {
    state: RefCell<State>,
    generation: Arc<AtomicU64>,
}
impl Context {
    fn changed(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
    fn fail(&self) {
        self.changed();
        self.state.borrow_mut().fail();
    }
    fn revoke(&self) {
        self.changed();
        self.state.borrow_mut().revoke();
    }
    fn reset(&self) {
        self.changed();
        self.state.borrow_mut().reset();
    }
}

fn async_high() -> u16 {
    (policy::FIRST_HIGH..=policy::LAST_HIGH).fold(0, |mask, vk| {
        if unsafe { GetAsyncKeyState(i32::from(vk)) } < 0 {
            mask | (1 << (vk - policy::FIRST_HIGH))
        } else {
            mask
        }
    })
}
fn ordinary_down() -> bool {
    (0x08..=0xfe).any(|vk| {
        !(i32::from(policy::FIRST_HIGH)..=i32::from(policy::LAST_HIGH)).contains(&vk)
            && unsafe { GetAsyncKeyState(vk) } < 0
    })
}

unsafe fn read_raw(context: &Context, handle: HRAWINPUT) {
    let mut input = RAWINPUT::default();
    let mut bytes = size_of::<RAWINPUT>() as u32;
    let read = GetRawInputData(
        handle,
        RID_INPUT,
        Some((&mut input as *mut RAWINPUT).cast()),
        &mut bytes,
        size_of::<RAWINPUTHEADER>() as u32,
    );
    if read == u32::MAX
        || read < size_of::<RAWINPUTHEADER>() as u32
        || read > size_of::<RAWINPUT>() as u32
    {
        context.fail();
        return;
    }
    if input.header.dwType == RIM_TYPEKEYBOARD.0 {
        if read
            < (size_of::<RAWINPUTHEADER>() + size_of::<windows::Win32::UI::Input::RAWKEYBOARD>())
                as u32
        {
            context.fail();
            return;
        }
        let key = input.data.keyboard;
        if key.MakeCode == 0xff || key.Flags & !7 != 0 {
            context.fail();
            return;
        }
        context.changed();
        // No ordinary VK, scancode, character or event sequence is stored.
        context.state.borrow_mut().raw(
            input.header.hDevice.0 as usize,
            policy::high_index(key.VKey),
            key.Flags & 1 == 0,
        );
    }
}
unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Context;
    if let Some(context) = ptr.as_ref() {
        match message {
            WM_INPUT => {
                read_raw(context, HRAWINPUT(l.0 as *mut _));
                // Required foreground raw-input cleanup, including filtered keys
                // and failed reads. Data has already been read before cleanup.
                return DefWindowProcW(hwnd, message, w, l);
            }
            WM_INPUT_DEVICE_CHANGE => {
                context.changed();
                if w.0 == 2 {
                    context.state.borrow_mut().remove_device(l.0 as usize);
                } else {
                    context.state.borrow_mut().device_changed();
                }
            }
            WM_WTSSESSION_CHANGE => context.reset(),
            _ => {}
        }
    }
    DefWindowProcW(hwnd, message, w, l)
}
unsafe extern "system" fn desktop_event(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    DESKTOP_RESET.with(|flag| flag.set(true));
}

fn registrations() -> Result<Vec<RAWINPUTDEVICE>, &'static str> {
    let mut count = 0;
    if unsafe { GetRegisteredRawInputDevices(None, &mut count, size_of::<RAWINPUTDEVICE>() as u32) }
        == u32::MAX
        || count > 64
    {
        return Err(UNAVAILABLE);
    }
    let mut devices = vec![RAWINPUTDEVICE::default(); count as usize];
    if count != 0 {
        let read = unsafe {
            GetRegisteredRawInputDevices(
                Some(devices.as_mut_ptr()),
                &mut count,
                size_of::<RAWINPUTDEVICE>() as u32,
            )
        };
        if read == u32::MAX || read as usize > devices.len() {
            return Err(UNAVAILABLE);
        }
        devices.truncate(read as usize);
    }
    Ok(devices)
}
fn ours(hwnd: HWND) -> bool {
    registrations().is_ok_and(|devices| {
        devices.iter().any(|d| {
            d.usUsagePage == 1
                && d.usUsage == 6
                && d.hwndTarget == hwnd
                // Windows can omit DEVNOTIFY when reading a registration back.
                // The registration call still requests it; identity and the
                // background sink must remain exactly ours.
                && d.dwFlags.contains(RIDEV_INPUTSINK)
        })
    })
}

/// A posted/sent query can precede hardware messages. Explicitly drain the raw
/// range, not merely the message which woke the thread. No RefCell borrow spans
/// DispatchMessage (sent window messages can reenter the procedure).
fn drain_raw(hwnd: HWND, context: &Context) -> bool {
    let mut message = MSG::default();
    for _ in 0..DRAIN_LIMIT {
        if !unsafe {
            PeekMessageW(
                &mut message,
                Some(hwnd),
                WM_INPUT_DEVICE_CHANGE,
                WM_INPUT,
                PM_REMOVE,
            )
        }
        .as_bool()
        {
            if unsafe { GetQueueStatus(QS_RAWINPUT) } >> 16 != 0 {
                // A packet can arrive after the empty Peek. Retry within the
                // same bounded drain; only exhaustion means synchronization failed.
                continue;
            }
            if DESKTOP_RESET.with(|flag| flag.replace(false)) {
                context.reset();
            }
            return true;
        }
        unsafe {
            DispatchMessageW(&message);
        }
    }
    context.fail();
    false
}

struct Resources {
    hwnd: HWND,
    instance: HINSTANCE,
    desktop_hook: HWINEVENTHOOK,
    session: bool,
}
impl Drop for Resources {
    fn drop(&mut self) {
        unsafe {
            if self.session {
                let _ = WTSUnRegisterSessionNotification(self.hwnd);
            }
            if !self.desktop_hook.is_invalid() {
                let _ = UnhookWinEvent(self.desktop_hook);
            }
            if ours(self.hwnd) {
                let _ = RegisterRawInputDevices(
                    &[RAWINPUTDEVICE {
                        usUsagePage: 1,
                        usUsage: 6,
                        dwFlags: RIDEV_REMOVE,
                        hwndTarget: HWND::default(),
                    }],
                    size_of::<RAWINPUTDEVICE>() as u32,
                );
            }
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
            let _ = UnregisterClassW(w!("QingshuHighFunctionObserver"), Some(self.instance));
        }
    }
}
fn run(
    requests: mpsc::Receiver<Request>,
    ready: mpsc::SyncSender<Result<usize, &'static str>>,
    generation: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
) -> Result<(), &'static str> {
    // Tao's Always device-event filter removes its registration before setup.
    // Never steal another component's process-wide keyboard receiver.
    if registrations()?
        .iter()
        .any(|d| d.usUsagePage == 1 && d.usUsage == 6)
    {
        return Err(UNAVAILABLE);
    }
    let context = Box::new(Context {
        state: RefCell::new(State::new()),
        generation,
    });
    let instance = unsafe { GetModuleHandleW(None) }.map_err(|_| UNAVAILABLE)?;
    let instance = HINSTANCE(instance.0);
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: w!("QingshuHighFunctionObserver"),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(UNAVAILABLE);
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("QingshuHighFunctionObserver"),
            w!(""),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|_| UNAVAILABLE)?;
    let mut resources = Resources {
        hwnd,
        instance,
        desktop_hook: HWINEVENTHOOK::default(),
        session: false,
    };
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*context as *const Context as isize);
    }
    unsafe {
        RegisterRawInputDevices(
            &[RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 6,
                dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
                hwndTarget: hwnd,
            }],
            size_of::<RAWINPUTDEVICE>() as u32,
        )
    }
    .map_err(|_| UNAVAILABLE)?;
    unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) }
        .map_err(|_| UNAVAILABLE)?;
    resources.session = true;
    resources.desktop_hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_DESKTOPSWITCH,
            EVENT_SYSTEM_DESKTOPSWITCH,
            None,
            Some(desktop_event),
            0,
            0,
            0,
        )
    };
    if resources.desktop_hook.is_invalid() {
        return Err(UNAVAILABLE);
    }
    let began = Instant::now();
    alive.store(true, Ordering::Release);
    ready.send(Ok(hwnd.0 as usize)).map_err(|_| UNAVAILABLE)?;
    loop {
        let mut message = MSG::default();
        let mut drained = false;
        for _ in 0..DRAIN_LIMIT {
            if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                drained = true;
                break;
            }
            if message.message == WM_QUIT {
                return Err(UNAVAILABLE);
            }
            unsafe {
                DispatchMessageW(&message);
            }
        }
        if !drained {
            context.fail();
        }
        if DESKTOP_RESET.with(|flag| flag.replace(false)) {
            context.reset();
        }
        if drain_raw(hwnd, &context) && !unsafe { GetForegroundWindow() }.is_invalid() {
            let generation = context.generation.load(Ordering::Acquire);
            let high = async_high();
            if context.generation.load(Ordering::Acquire) == generation {
                context.state.borrow_mut().baseline(high);
            }
        }
        for _ in 0..16 {
            let Ok(request) = requests.try_recv() else {
                break;
            };
            let result = if Instant::now() >= request.deadline {
                context.revoke();
                Err(UNAVAILABLE)
            } else if !ours(hwnd) || !drain_raw(hwnd, &context) {
                context.fail();
                Err(UNAVAILABLE)
            } else {
                let generation = context.generation.load(Ordering::Acquire);
                let high = async_high();
                let ordinary = ordinary_down();
                let available = context.state.borrow().available();
                let result = context.state.borrow_mut().query(
                    ordinary,
                    high,
                    request.bound,
                    began.elapsed().as_millis() as u64,
                );
                if !available {
                    Err(UNAVAILABLE)
                } else if Instant::now() >= request.deadline
                    || unsafe { GetQueueStatus(QS_RAWINPUT) } >> 16 != 0
                    || context.generation.load(Ordering::Acquire) != generation
                {
                    context.revoke();
                    Err(UNAVAILABLE)
                } else if result == Decision::Ready {
                    Ok(Prepared {
                        generation,
                        high,
                        at: Instant::now(),
                    })
                } else {
                    Err("keys_held")
                }
            };
            let _ = request.reply.send((request.id, result));
        }
        // Idle work is event-driven. The caller's bounded release grace sends
        // explicit query wake-ups; no continuous keyboard polling timer runs.
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            return Err(UNAVAILABLE);
        }
        unsafe {
            DispatchMessageW(&message);
        }
    }
}

pub(crate) fn start() -> Result<(), &'static str> {
    OBSERVER
        .get_or_init(|| {
            let (send, receive) = mpsc::channel();
            let (ready_send, ready_receive) = mpsc::sync_channel(1);
            let generation = Arc::new(AtomicU64::new(1));
            let alive = Arc::new(AtomicBool::new(false));
            let worker_generation = Arc::clone(&generation);
            let worker_alive = Arc::clone(&alive);
            thread::Builder::new()
                .name("high-function-state".into())
                .spawn(move || {
                    let failure_reply = ready_send.clone();
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if let Err(error) = run(
                            receive,
                            ready_send,
                            Arc::clone(&worker_generation),
                            Arc::clone(&worker_alive),
                        ) {
                            let _ = failure_reply.try_send(Err(error));
                        }
                    }));
                    worker_alive.store(false, Ordering::Release);
                    worker_generation.fetch_add(1, Ordering::AcqRel);
                })
                .map_err(|_| UNAVAILABLE)?;
            let hwnd = ready_receive
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| UNAVAILABLE)??;
            Ok(Observer {
                hwnd,
                requests: send,
                generation,
                alive,
            })
        })
        .as_ref()
        .map(|_| ())
        .map_err(|error| *error)
}

pub(crate) fn prepare(bound: u16) -> Result<Prepared, &'static str> {
    if unsafe { GetForegroundWindow() }.is_invalid() {
        return Err(UNAVAILABLE);
    }
    let high = async_high();
    if ordinary_down() || high & bound != 0 {
        return Err("keys_held");
    }
    let observer = match OBSERVER.get() {
        Some(Ok(observer)) => {
            if !observer.alive.load(Ordering::Acquire) {
                return Err(UNAVAILABLE);
            }
            observer
        }
        // An unavailable optional observer can never relax a high-function
        // state. With all async keys up the old conservative guard still works.
        _ if high == 0 => {
            return Ok(Prepared {
                generation: 0,
                high: 0,
                at: Instant::now(),
            })
        }
        _ => return Err(UNAVAILABLE),
    };
    let (send, receive) = mpsc::sync_channel(1);
    let id = NEXT_QUERY.fetch_add(1, Ordering::Relaxed);
    let deadline = Instant::now() + QUERY_TIMEOUT;
    observer
        .requests
        .send(Request {
            id,
            bound,
            deadline,
            reply: send,
        })
        .map_err(|_| UNAVAILABLE)?;
    unsafe {
        PostMessageW(
            Some(HWND(observer.hwnd as *mut _)),
            WAKE,
            WPARAM(0),
            LPARAM(0),
        )
    }
    .map_err(|_| UNAVAILABLE)?;
    let (received_id, result) = receive
        .recv_timeout(QUERY_TIMEOUT)
        .map_err(|_| UNAVAILABLE)?;
    if received_id != id || Instant::now() >= deadline {
        return Err(UNAVAILABLE);
    }
    result
}

/// No observer IPC after this point. The caller performs the final clipboard
/// lease check after prepare and invokes this only at the single key boundary.
pub(crate) fn fast_check(prepared: &Prepared, bound: u16) -> Result<bool, &'static str> {
    if unsafe { GetForegroundWindow() }.is_invalid() {
        return Err(UNAVAILABLE);
    }
    let ordinary = ordinary_down();
    let high = async_high();
    if ordinary || high & bound != 0 {
        return Ok(true);
    }
    if prepared.generation == 0 {
        return Ok(high != 0 || prepared.at.elapsed() > TOKEN_LIFETIME);
    }
    let observer = OBSERVER
        .get()
        .ok_or(UNAVAILABLE)?
        .as_ref()
        .map_err(|e| *e)?;
    if !observer.alive.load(Ordering::Acquire)
        || prepared.at.elapsed() > TOKEN_LIFETIME
        || observer.generation.load(Ordering::Acquire) != prepared.generation
        || high != prepared.high
    {
        return Ok(true);
    }
    Ok(false)
}
