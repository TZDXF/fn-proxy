use crate::{text::Text, types::LogEntry};
use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::Mutex,
};

const MAX_MEMORY_ENTRIES: usize = 200;
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_BACKUPS: usize = 3;
const LOG_FILE_NAME: &str = "fn-proxy.log";

#[derive(Default)]
pub struct RuntimeLogs {
    inner: Mutex<LogState>,
}

#[derive(Default)]
struct LogState {
    entries: VecDeque<LogEntry>,
    file: Option<RotatingLog>,
    file_error_reported: bool,
}

impl RuntimeLogs {
    pub fn enable_file(&self, directory: PathBuf) -> io::Result<()> {
        let file = RotatingLog::new(directory, MAX_FILE_BYTES, MAX_BACKUPS)?;
        self.inner.lock().unwrap().file = Some(file);
        Ok(())
    }

    /// Keep UI logging available even when the disk is full or inaccessible.
    /// Return a warning only once per consecutive sequence of write failures.
    pub fn push(&self, entry: &LogEntry) -> Option<LogEntry> {
        let mut state = self.inner.lock().unwrap();
        state.push_memory(entry.clone());
        let result = state.file.as_ref().map(|file| file.append(entry));
        match result {
            Some(Err(error)) if !state.file_error_reported => {
                state.file_error_reported = true;
                eprintln!("FN Proxy could not write runtime log: {error}");
                let warning = LogEntry {
                    time: entry.time,
                    level: "warn".to_owned(),
                    label: "FN Proxy".to_owned(),
                    message: Text::new("logs.fileLoggingFailed"),
                };
                state.push_memory(warning.clone());
                Some(warning)
            }
            Some(Ok(())) => {
                state.file_error_reported = false;
                None
            }
            _ => None,
        }
    }

    pub fn entries(&self) -> Vec<LogEntry> {
        self.inner.lock().unwrap().entries.iter().cloned().collect()
    }
}

impl LogState {
    fn push_memory(&mut self, entry: LogEntry) {
        self.entries.push_back(entry);
        if self.entries.len() > MAX_MEMORY_ENTRIES {
            self.entries.pop_front();
        }
    }
}

struct RotatingLog {
    directory: PathBuf,
    max_file_bytes: u64,
    max_backups: usize,
}

impl RotatingLog {
    fn new(directory: PathBuf, max_file_bytes: u64, max_backups: usize) -> io::Result<Self> {
        fs::create_dir_all(&directory)?;
        let log = Self {
            directory,
            max_file_bytes,
            max_backups,
        };
        // Recover files that already exceed the limit (e.g. from an older version).
        // Do not move an oversized file into a backup and violate the total bound.
        for index in 0..=max_backups {
            let path = log.path(index);
            match fs::metadata(&path) {
                Ok(metadata) if metadata.len() > max_file_bytes => {
                    OpenOptions::new().write(true).open(path)?.set_len(0)?;
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(log.path(0))?;
        Ok(log)
    }

    fn path(&self, index: usize) -> PathBuf {
        self.directory.join(if index == 0 {
            LOG_FILE_NAME.to_owned()
        } else {
            format!("{LOG_FILE_NAME}.{index}")
        })
    }

    fn append(&self, entry: &LogEntry) -> io::Result<()> {
        // Only persist the existing locale-neutral UI event, never request bodies,
        // authentication responses, cookies, credentials or raw transport errors.
        let mut line = serde_json::to_vec(entry)?;
        line.push(b'\n');
        if line.len() as u64 > self.max_file_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime log entry exceeds the file size limit",
            ));
        }
        let path = self.path(0);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        if size > self.max_file_bytes - line.len() as u64 {
            // Windows cannot rename a file while our write handle is open.
            drop(file);
            self.rotate()?;
            file = OpenOptions::new().create(true).append(true).open(path)?;
        }
        file.write_all(&line)?;
        file.flush()
    }

    fn rotate(&self) -> io::Result<()> {
        Self::remove_if_present(&self.path(self.max_backups))?;
        for index in (0..self.max_backups).rev() {
            match fs::rename(self.path(index), self.path(index + 1)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn remove_if_present(path: &std::path::Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "fn-proxy-log-test-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            )))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn entry(time: u64) -> LogEntry {
        LogEntry {
            time,
            level: "info".to_owned(),
            label: "测试 NAS".to_owned(),
            message: Text::with("logs.domainsUpdated", [("count", "2".to_owned())]),
        }
    }

    fn lines(log: &RotatingLog, index: usize) -> Vec<serde_json::Value> {
        fs::read_to_string(log.path(index))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn appends_structured_events_across_restarts() {
        let directory = TestDirectory::new();
        let log = RotatingLog::new(directory.0.clone(), 1024, 3).unwrap();
        log.append(&entry(1)).unwrap();
        let log = RotatingLog::new(directory.0.clone(), 1024, 3).unwrap();
        log.append(&entry(2)).unwrap();
        let events = lines(&log, 0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], serde_json::to_value(entry(1)).unwrap());
        assert_eq!(events[1], serde_json::to_value(entry(2)).unwrap());
    }

    #[test]
    fn rotates_before_exceeding_limit_and_removes_oldest_backup() {
        let directory = TestDirectory::new();
        let size = serde_json::to_vec(&entry(0)).unwrap().len() as u64 + 1;
        let log = RotatingLog::new(directory.0.clone(), size * 2, 3).unwrap();
        for time in 0..10 {
            log.append(&entry(time)).unwrap();
        }
        for index in 0..=3 {
            assert_eq!(fs::metadata(log.path(index)).unwrap().len(), size * 2);
            let events = lines(&log, index);
            assert_eq!(events[0]["time"], (8 - index * 2) as u64);
            assert_eq!(events[1]["time"], (9 - index * 2) as u64);
        }
        assert!(!log.path(4).exists());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 4);
    }

    #[test]
    fn rejects_oversized_event_without_modifying_files() {
        let directory = TestDirectory::new();
        let size = serde_json::to_vec(&entry(0)).unwrap().len() as u64 + 1;
        let log = RotatingLog::new(directory.0.clone(), size, 3).unwrap();
        log.append(&entry(1)).unwrap();
        let mut oversized = entry(2);
        oversized.label = "x".repeat(size as usize);
        assert_eq!(
            log.append(&oversized).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(lines(&log, 0).len(), 1);
        assert!(!log.path(1).exists());
    }

    #[test]
    fn recovers_oversized_existing_files_without_exceeding_bound() {
        let directory = TestDirectory::new();
        fs::create_dir_all(&directory.0).unwrap();
        for name in [LOG_FILE_NAME, "fn-proxy.log.1"] {
            fs::write(directory.0.join(name), vec![b'x'; 2048]).unwrap();
        }
        let log = RotatingLog::new(directory.0.clone(), 1024, 3).unwrap();
        log.append(&entry(1)).unwrap();
        assert_eq!(lines(&log, 0).len(), 1);
        assert_eq!(fs::metadata(log.path(1)).unwrap().len(), 0);
    }

    #[test]
    fn keeps_latest_200_events_in_memory_but_persists_all() {
        let directory = TestDirectory::new();
        let logs = RuntimeLogs::default();
        logs.enable_file(directory.0.clone()).unwrap();
        for time in 0..205 {
            assert!(logs.push(&entry(time)).is_none());
        }
        let events = logs.entries();
        assert_eq!(events.len(), 200);
        assert_eq!(events[0].time, 5);
        assert_eq!(events[199].time, 204);
        assert_eq!(
            fs::read_to_string(directory.0.join(LOG_FILE_NAME))
                .unwrap()
                .lines()
                .count(),
            205
        );
    }

    #[test]
    fn concurrent_writers_produce_complete_json_lines() {
        let directory = TestDirectory::new();
        let logs = Arc::new(RuntimeLogs::default());
        logs.enable_file(directory.0.clone()).unwrap();
        let workers: Vec<_> = (0..8)
            .map(|worker| {
                let logs = logs.clone();
                std::thread::spawn(move || {
                    for index in 0..25 {
                        assert!(logs.push(&entry(worker * 25 + index)).is_none());
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let contents = fs::read_to_string(directory.0.join(LOG_FILE_NAME)).unwrap();
        let mut times: Vec<_> = contents
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["time"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        times.sort_unstable();
        assert_eq!(times, (0..200).collect::<Vec<_>>());
    }

    #[test]
    fn disk_failure_warns_once_preserves_memory_and_can_recover() {
        let directory = TestDirectory::new();
        let logs = RuntimeLogs::default();
        logs.enable_file(directory.0.clone()).unwrap();
        let path = directory.0.join(LOG_FILE_NAME);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let warning = logs.push(&entry(1)).unwrap();
        assert_eq!(warning.message.code, "logs.fileLoggingFailed");
        assert!(logs.push(&entry(2)).is_none());
        assert_eq!(logs.entries().len(), 3);
        fs::remove_dir(&path).unwrap();
        assert!(logs.push(&entry(3)).is_none());
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 1);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(logs.push(&entry(4)).is_some());
    }

    #[test]
    fn initialization_failure_does_not_disable_memory_logging() {
        let directory = TestDirectory::new();
        fs::create_dir_all(&directory.0).unwrap();
        let path = directory.0.join("not-a-directory");
        fs::write(&path, "file").unwrap();
        let logs = RuntimeLogs::default();
        assert!(logs.enable_file(path).is_err());
        assert!(logs.push(&entry(1)).is_none());
        assert_eq!(logs.entries()[0].time, 1);
    }
}
