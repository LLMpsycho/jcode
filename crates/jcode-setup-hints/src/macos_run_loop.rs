// Minimal Carbon/ApplicationServices bindings to (a) make this faceless
// launchd process eligible to receive global hotkeys and (b) run the Carbon
// application event loop that dispatches `RegisterEventHotKey` events.
//
// We deliberately avoid pulling in a heavier `core-foundation`/`cocoa`
// dependency just for these few calls.

#[repr(C)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

// `kCurrentProcess` from MacTypes / Process Manager.
const K_CURRENT_PROCESS: u32 = 2;
// `kProcessTransformToUIElementApplication` from ApplicationServices.
// Promotes a background (faceless) process to a UIElement app so it has a
// connection to the window server and can receive Carbon hotkey events,
// without showing a Dock icon or menu bar.
const K_PROCESS_TRANSFORM_TO_UI_ELEMENT_APPLICATION: u32 = 4;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn TransformProcessType(psn: *const ProcessSerialNumber, transform_state: u32) -> i32;
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn RunApplicationEventLoop();
}

/// Promote this process to a UIElement application.
///
/// A LaunchAgent started without an app bundle runs as a faceless background
/// process with no window-server connection, so Carbon `RegisterEventHotKey`
/// events are never delivered to it. Transforming the process type gives it
/// the connection it needs while keeping it out of the Dock and menu bar.
///
/// Returns the raw OSStatus (0 == `noErr`).
pub fn promote_to_ui_element() -> i32 {
    let psn = ProcessSerialNumber {
        high: 0,
        low: K_CURRENT_PROCESS,
    };
    // SAFETY: `psn` points at a valid ProcessSerialNumber for the lifetime of
    // the call; the transform constant is a documented Process Manager value.
    unsafe { TransformProcessType(&psn, K_PROCESS_TRANSFORM_TO_UI_ELEMENT_APPLICATION) }
}

/// Block forever on the Carbon application event loop, dispatching hotkey
/// (and other Carbon) events as they arrive.
///
/// This must run on the real main thread that created the hotkey manager.
/// `RunApplicationEventLoop` installs the standard application event handlers
/// and pumps the main run loop; unlike a bare `CFRunLoopRun()` it guarantees
/// the Carbon event target that `RegisterEventHotKey` dispatches through is
/// actually serviced, and it does not return spuriously when no Core
/// Foundation input source happens to be installed yet.
pub fn run_forever() {
    // SAFETY: takes no arguments; runs the calling (main) thread's event loop.
    unsafe { RunApplicationEventLoop() };
}
