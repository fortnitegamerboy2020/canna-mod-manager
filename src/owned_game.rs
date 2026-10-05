//! Retains a Windows process handle, so PID reuse cannot redirect a Stop action.
use anyhow::{Result, bail};
use std::path::Path;

pub struct OwnedGame {
    handle: isize,
    pub pid: u32,
}
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn CloseHandle(handle: isize) -> i32;
    fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32;
    fn TerminateProcess(handle: isize, code: u32) -> i32;
    fn QueryFullProcessImageNameW(handle: isize, flags: u32, name: *mut u16, len: *mut u32) -> i32;
    fn GetProcessTimes(
        handle: isize,
        created: *mut u64,
        exited: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
    fn GetSystemTimeAsFileTime(time: *mut u64);
}
impl OwnedGame {
    #[cfg(windows)]
    pub fn now() -> u64 {
        let mut time = 0;
        unsafe { GetSystemTimeAsFileTime(&mut time) };
        time
    }
    #[cfg(windows)]
    pub fn capture(pid: u32, root: &Path, earliest: u64) -> Result<Self> {
        let handle = unsafe { OpenProcess(0x100000 | 0x1000 | 1, 0, pid) };
        if handle == 0 {
            bail!(
                "Cannot retain game process: {}",
                std::io::Error::last_os_error()
            );
        }
        let owned = Self { handle, pid };
        let mut name = vec![0u16; 32768];
        let mut len = name.len() as u32;
        let mut created = 0;
        let mut exited = 0;
        let mut kernel = 0;
        let mut user = 0;
        if unsafe { QueryFullProcessImageNameW(handle, 0, name.as_mut_ptr(), &mut len) } == 0
            || unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
                == 0
        {
            bail!("Cannot verify launched game identity");
        }
        let executable = std::path::PathBuf::from(String::from_utf16_lossy(&name[..len as usize]));
        let parent = executable.parent().and_then(|p| p.canonicalize().ok());
        if parent != Some(root.canonicalize()?) || created < earliest || !owned.running() {
            bail!("Process does not belong to this launch");
        }
        Ok(owned)
    }
    pub fn running(&self) -> bool {
        #[cfg(windows)]
        {
            unsafe { WaitForSingleObject(self.handle, 0) == 258 }
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
    pub fn stop(&self) -> Result<()> {
        if !self.running() {
            return Ok(());
        }
        #[cfg(windows)]
        if unsafe { TerminateProcess(self.handle, 0) } == 0 {
            bail!("Could not stop game: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }
}
impl Drop for OwnedGame {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn stops_owned_process_and_rejects_old_or_wrong_root() {
        use std::os::windows::process::CommandExt;
        let executable = Path::new("C:/Windows/System32/ping.exe");
        let earliest = OwnedGame::now();
        let mut child = std::process::Command::new(executable)
            .args(["-t", "127.0.0.1"])
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        let mut unrelated = std::process::Command::new(executable)
            .args(["-t", "127.0.0.1"])
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        assert!(OwnedGame::capture(child.id(), Path::new("C:/"), earliest).is_err());
        assert!(OwnedGame::capture(child.id(), executable.parent().unwrap(), u64::MAX).is_err());
        let owned = OwnedGame::capture(child.id(), executable.parent().unwrap(), earliest).unwrap();
        owned.stop().unwrap();
        child.wait().unwrap();
        assert!(!owned.running());
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
    }
}
