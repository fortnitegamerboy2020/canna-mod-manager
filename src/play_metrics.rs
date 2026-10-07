//! Local process working-set samples. Identifiers and measurements never leave the device.
#[derive(Clone, Copy, Debug)]
pub struct Memory {
    pub current: u64,
    pub peak: u64,
}
#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct Counters {
    cb: u32,
    faults: u32,
    peak: usize,
    current: usize,
    quota_peak_paged: usize,
    quota_paged: usize,
    quota_peak_nonpaged: usize,
    quota_nonpaged: usize,
    pagefile: usize,
    peak_pagefile: usize,
}
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, size: u32) -> i32;
}
pub fn sample(handle: isize) -> Option<Memory> {
    #[cfg(windows)]
    {
        let mut counters = Counters {
            cb: std::mem::size_of::<Counters>() as u32,
            ..Default::default()
        };
        if unsafe {
            K32GetProcessMemoryInfo(
                handle,
                &mut counters,
                std::mem::size_of::<Counters>() as u32,
            )
        } != 0
        {
            Some(Memory {
                current: counters.current as u64,
                peak: counters.peak as u64,
            })
        } else {
            None
        }
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        None
    }
}
pub fn app() -> Option<Memory> {
    #[cfg(windows)]
    {
        sample(unsafe { GetCurrentProcess() })
    }
    #[cfg(not(windows))]
    {
        None
    }
}
#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn samples_only_the_requested_process() {
        let memory = super::app().unwrap();
        assert!(memory.current > 0 && memory.peak >= memory.current);
        assert!(super::sample(0).is_none());
    }
}
