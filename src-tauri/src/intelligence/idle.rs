//! Platform probes for the opt-in idle backlog: OS user-idle time and power source.
//! Both are observations only; policy lives in `maintenance`. Unsupported hosts
//! report `None`, which callers must treat as "do not start backlog work".
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerSource {
    Ac,
    Battery,
    Unknown,
}

/// Whole seconds since the last user input event (keyboard, pointer, etc.).
pub fn user_idle_seconds() -> Option<u64> {
    imp::idle_seconds()
}

/// Current external power state, or `None` when the platform cannot report it.
pub fn power_source() -> Option<PowerSource> {
    imp::power_source()
}

/// Backlog work requires the user to be away long enough that inference will not
/// compete with active use. Unknown idle time fails closed.
pub fn user_is_idle(minimum: Duration) -> bool {
    user_idle_seconds().is_some_and(|seconds| Duration::from_secs(seconds) >= minimum)
}

/// Unknown power state fails closed, matching the "stop on battery" decision.
pub fn on_external_power() -> bool {
    matches!(power_source(), Some(PowerSource::Ac))
}

#[cfg(target_os = "macos")]
mod imp {
    use super::PowerSource;
    use std::ffi::{c_char, c_void, CStr};

    type CGEventSourceStateId = u32;
    type CGEventType = u32;
    const COMBINED_SESSION_STATE: CGEventSourceStateId = 0;
    const ANY_INPUT_EVENT: CGEventType = u32::MAX;
    // kCFStringEncodingUTF8. kIOPSACPowerValue/kIOPSBatteryPowerValue are those
    // literals; they are not linkable data symbols.
    const UTF8_ENCODING: u32 = 0x0800_0100;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(
            state: CGEventSourceStateId,
            event_type: CGEventType,
        ) -> f64;
    }
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> *mut c_void;
        fn IOPSGetProvidingPowerSourceType(blob: *mut c_void) -> *mut c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCString(
            string: *mut c_void,
            buffer: *mut c_char,
            capacity: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(value: *mut c_void);
    }

    pub fn idle_seconds() -> Option<u64> {
        // The API reports seconds, and a negative or non-finite value is invalid.
        let seconds = unsafe {
            CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT)
        };
        if seconds.is_finite() && seconds >= 0.0 {
            Some(seconds as u64)
        } else {
            None
        }
    }

    pub fn power_source() -> Option<PowerSource> {
        unsafe {
            // `IOPSCopyPowerSourcesInfo` follows the Copy rule: release our reference.
            let blob = IOPSCopyPowerSourcesInfo();
            if blob.is_null() {
                return None;
            }
            let source = IOPSGetProvidingPowerSourceType(blob);
            let mut buffer = [0 as c_char; 32];
            let copied = !source.is_null()
                && CFStringGetCString(
                    source,
                    buffer.as_mut_ptr(),
                    buffer.len() as isize,
                    UTF8_ENCODING,
                ) != 0;
            let result = if !copied {
                None
            } else {
                // Documented values of kIOPSACPowerValue / kIOPSBatteryPowerValue.
                match CStr::from_ptr(buffer.as_ptr()).to_str() {
                    Ok("AC Power") => Some(PowerSource::Ac),
                    Ok("Battery Power") => Some(PowerSource::Battery),
                    _ => Some(PowerSource::Unknown),
                }
            };
            CFRelease(blob);
            result
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::PowerSource;

    #[repr(C)]
    struct LastInputInfo {
        cb_size: u32,
        dw_time: u32,
    }
    #[repr(C)]
    struct SystemPowerStatus {
        ac_line_status: u8,
        battery_flag: u8,
        battery_life_percent: u8,
        system_status_flag: u8,
        battery_life_time: u32,
        battery_full_life_time: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
        fn GetTickCount() -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
    }

    pub fn idle_seconds() -> Option<u64> {
        let mut info = LastInputInfo {
            cb_size: std::mem::size_of::<LastInputInfo>() as u32,
            dw_time: 0,
        };
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return None;
        }
        // Both values are 32-bit millisecond ticks; wrapping subtraction covers
        // the documented ~49.7 day tick rollover.
        let elapsed = unsafe { GetTickCount() }.wrapping_sub(info.dw_time);
        Some(u64::from(elapsed / 1000))
    }

    pub fn power_source() -> Option<PowerSource> {
        let mut status = SystemPowerStatus {
            ac_line_status: 255,
            battery_flag: 255,
            battery_life_percent: 255,
            system_status_flag: 0,
            battery_life_time: u32::MAX,
            battery_full_life_time: u32::MAX,
        };
        if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
            return None;
        }
        match status.ac_line_status {
            0 => Some(PowerSource::Battery),
            1 => Some(PowerSource::Ac),
            _ => Some(PowerSource::Unknown),
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use super::PowerSource;

    pub fn idle_seconds() -> Option<u64> {
        None
    }

    pub fn power_source() -> Option<PowerSource> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_probe_reports_a_value_on_supported_hosts() {
        #[cfg(any(target_os = "macos", windows))]
        {
            assert!(user_idle_seconds().is_some());
            // The API can report `Unknown`, but a supported host must answer.
            assert!(power_source().is_some());
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            assert_eq!(user_idle_seconds(), None);
            assert_eq!(power_source(), None);
        }
    }

    #[test]
    fn unknown_observations_never_admit_backlog_work() {
        // Unsupported hosts must fail closed for both policy predicates.
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            assert!(!user_is_idle(Duration::ZERO));
            assert!(!on_external_power());
        }
        assert!(!user_is_idle(Duration::from_secs(u64::MAX)));
    }
}
