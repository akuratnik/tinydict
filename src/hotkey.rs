//! Global Ctrl+Cmd+X on macOS via Carbon's RegisterEventHotKey: no
//! Accessibility prompt, no third-party hotkey app. Runs as its own KeepAlive
//! LaunchAgent so the daemon can still idle-exit.

use crate::cli;
use anyhow::{bail, Result};
use std::ffi::c_void;
use std::ptr;

const EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const EVENT_HOT_KEY_PRESSED: u32 = 5;
const CMD_KEY: u32 = 1 << 8;
const CONTROL_KEY: u32 = 1 << 12;
const KEY_X: u32 = 0x07; // kVK_ANSI_X
const HOT_KEY_EXCLUSIVE: u32 = 1 << 0;

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: Handler,
        num_types: usize, // ItemCount is unsigned long
        types: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: *mut c_void,
        options: u32,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn RunApplicationEventLoop();
}

pub fn run() -> Result<()> {
    let pressed = EventTypeSpec {
        class: EVENT_CLASS_KEYBOARD,
        kind: EVENT_HOT_KEY_PRESSED,
    };
    let id = EventHotKeyId {
        signature: u32::from_be_bytes(*b"tndc"),
        id: 1,
    };
    let mut hotkey = ptr::null_mut();
    unsafe {
        let target = GetApplicationEventTarget();
        let err = InstallEventHandler(
            target,
            on_hotkey,
            1,
            &pressed,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if err != 0 {
            bail!("InstallEventHandler failed ({err})");
        }
        let err = RegisterEventHotKey(
            KEY_X,
            CONTROL_KEY | CMD_KEY,
            id,
            target,
            HOT_KEY_EXCLUSIVE,
            &mut hotkey,
        );
        if err != 0 {
            bail!("RegisterEventHotKey failed ({err}); is Ctrl+Cmd+X taken?");
        }
        RunApplicationEventLoop();
    }
    Ok(())
}

extern "C" fn on_hotkey(_: *mut c_void, _: *mut c_void, _: *mut c_void) -> i32 {
    match cli::request("toggle") {
        Ok(reply) if reply.starts_with("error") => eprint!("tinydict hotkey: {reply}"),
        Ok(_) => {}
        Err(e) => eprintln!("tinydict hotkey: {e:#}"),
    }
    0 // noErr
}
