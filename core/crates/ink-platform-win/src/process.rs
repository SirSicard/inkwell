//! Processes: names by pid, and the root of an app's process tree.
//!
//! Windows identifies an app by its executable name (`Zoom.exe`, `chrome.exe`). A browser or a
//! call app is a tree of processes with the same name, and the one holding the microphone or
//! playing the call is usually a child (a browser's audio service). Per-process loopback captures
//! a process *and its tree*, so it needs the tree's root: the highest ancestor with the same name.
//!
//! The table comes from one Toolhelp snapshot; the decisions over it are pure and tested.
#![cfg(windows)]

use std::collections::HashMap;

use ink_core::PlatformError;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

/// One process in a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessEntry {
    pub(crate) pid: u32,
    pub(crate) parent: u32,
    /// The executable's file name, as Windows reports it (`Zoom.exe`).
    pub(crate) exe: String,
}

/// Every process at one moment, by pid.
#[derive(Clone, Debug, Default)]
pub(crate) struct ProcessTable {
    by_pid: HashMap<u32, ProcessEntry>,
}

/// The deepest tree walked before giving up: pids are reused, so a parent link can point at an
/// unrelated newer process and form a loop.
const MAX_DEPTH: usize = 64;

impl ProcessTable {
    /// A table over `entries` (for tests, and the snapshot below).
    pub(crate) fn from_entries(entries: impl IntoIterator<Item = ProcessEntry>) -> Self {
        Self {
            by_pid: entries.into_iter().map(|e| (e.pid, e)).collect(),
        }
    }

    /// Every process now. **Worker.**
    pub(crate) fn snapshot() -> Result<Self, PlatformError> {
        // SAFETY: a process snapshot of the whole system; the handle is closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|e| PlatformError::Failed(format!("listing processes: {e}")))?;
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut entries = Vec::new();
        // SAFETY: a live snapshot and an entry with `dwSize` set, as the API requires.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            entries.push(ProcessEntry {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
        }
        // SAFETY: the snapshot handle is ours and closed once.
        let _ = unsafe { CloseHandle(snapshot) };
        Ok(Self::from_entries(entries))
    }

    /// The executable name of `pid`, if it is running.
    pub(crate) fn exe(&self, pid: u32) -> Option<&str> {
        self.by_pid.get(&pid).map(|e| e.exe.as_str())
    }

    /// The root of `pid`'s tree: the highest ancestor with the same executable name, or `pid`
    /// itself. `None` if `pid` is not running.
    pub(crate) fn root_of(&self, pid: u32) -> Option<u32> {
        let start = self.by_pid.get(&pid)?;
        let mut root = start;
        for _ in 0..MAX_DEPTH {
            match self.by_pid.get(&root.parent) {
                Some(parent)
                    if parent.pid != root.pid && parent.exe.eq_ignore_ascii_case(&start.exe) =>
                {
                    root = parent;
                }
                _ => break,
            }
        }
        Some(root.pid)
    }

    /// The roots of every tree of processes named `exe` (case-insensitive), lowest pid first.
    pub(crate) fn roots_named(&self, exe: &str) -> Vec<u32> {
        let mut roots: Vec<u32> = self
            .by_pid
            .values()
            .filter(|e| e.exe.eq_ignore_ascii_case(exe))
            .filter_map(|e| self.root_of(e.pid))
            .collect();
        roots.sort_unstable();
        roots.dedup();
        roots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, parent: u32, exe: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent,
            exe: exe.into(),
        }
    }

    fn browser() -> ProcessTable {
        ProcessTable::from_entries([
            entry(4, 0, "System"),
            entry(100, 4, "explorer.exe"),
            entry(200, 100, "chrome.exe"),
            entry(210, 200, "chrome.exe"),
            entry(211, 210, "Chrome.EXE"),
            entry(300, 100, "chrome.exe"),
            entry(400, 100, "Zoom.exe"),
            entry(401, 400, "CptHost.exe"),
        ])
    }

    #[test]
    fn a_child_climbs_to_the_root_of_its_own_name() {
        let table = browser();
        assert_eq!(table.root_of(211), Some(200), "case does not matter");
        assert_eq!(table.root_of(200), Some(200));
        assert_eq!(
            table.root_of(401),
            Some(401),
            "a helper of another name is its own root"
        );
        assert_eq!(table.root_of(999), None);
        assert_eq!(table.exe(400), Some("Zoom.exe"));
    }

    #[test]
    fn every_separate_tree_is_a_root() {
        let table = browser();
        assert_eq!(table.roots_named("CHROME.exe"), vec![200, 300]);
        assert_eq!(table.roots_named("zoom.exe"), vec![400]);
        assert!(table.roots_named("absent.exe").is_empty());
    }

    #[test]
    fn a_parent_loop_from_pid_reuse_ends() {
        let table = ProcessTable::from_entries([entry(10, 11, "a.exe"), entry(11, 10, "a.exe")]);
        let root = table.root_of(10).expect("running");
        assert!(root == 10 || root == 11);
        let own_parent = ProcessTable::from_entries([entry(12, 12, "a.exe")]);
        assert_eq!(own_parent.root_of(12), Some(12));
    }

    /// Runs on CI: the snapshot needs no permission, and this test process is in it.
    #[test]
    fn the_snapshot_holds_this_process() {
        let table = ProcessTable::snapshot().expect("snapshot");
        let exe = table.exe(std::process::id()).expect("this process");
        assert!(exe.to_ascii_lowercase().ends_with(".exe"), "{exe}");
    }
}
