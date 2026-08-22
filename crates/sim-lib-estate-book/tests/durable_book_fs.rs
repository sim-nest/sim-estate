use sim_lib_estate_book::{Book, BookError, EventKind, Key, Table};
use std::{
    fmt::Write,
    fs,
    fs::OpenOptions,
    io,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

struct FsTable {
    root: PathBuf,
}
impl FsTable {
    fn open(root: &Path) -> Self {
        fs::create_dir_all(root).unwrap();
        Self { root: root.into() }
    }
    fn path(&self, key: &Key) -> PathBuf {
        let encoded = key
            .0
            .as_bytes()
            .iter()
            .fold(String::new(), |mut out, byte| {
                write!(out, "{byte:02x}").unwrap();
                out
            });
        self.root.join(encoded)
    }
    fn locked<R>(&self, action: impl FnOnce() -> R) -> R {
        let lock = self.root.join(".cas-lock");
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&lock) {
                Ok(file) => {
                    let result = action();
                    drop(file);
                    fs::remove_file(lock).unwrap();
                    return result;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("cannot acquire filesystem CAS lock: {error}"),
            }
        }
    }
}
impl Table for FsTable {
    fn get(&self, key: &Key) -> Result<Option<Vec<u8>>, BookError> {
        match fs::read(self.path(key)) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(BookError::Storage(e.to_string())),
        }
    }
    fn put_absent(&self, key: &Key, value: &[u8]) -> Result<(), BookError> {
        self.locked(|| match self.get(key)? {
            Some(v) if v == value => Ok(()),
            Some(v) => Err(BookError::Conflict {
                key: key.clone(),
                observed: Some(v),
            }),
            None => fs::write(self.path(key), value).map_err(|e| BookError::Storage(e.to_string())),
        })
    }
    fn compare_and_swap(
        &self,
        key: &Key,
        expected: Option<&[u8]>,
        value: &[u8],
    ) -> Result<(), BookError> {
        self.locked(|| {
            let observed = self.get(key)?;
            if observed.as_deref() != expected {
                return Err(BookError::Conflict {
                    key: key.clone(),
                    observed,
                });
            }
            fs::write(self.path(key), value).map_err(|e| BookError::Storage(e.to_string()))
        })
    }
    fn scan_prefix(&self, _: &str) -> Result<Vec<(Key, Vec<u8>)>, BookError> {
        unimplemented!("book projections traverse linked heads")
    }
}

#[test]
fn approval_process_worker() {
    let Ok(root) = std::env::var("SIM_ESTATE_APPROVAL_ROOT") else {
        return;
    };
    let book = Book::new(FsTable::open(Path::new(&root)));
    book.consume_approval("reviewed", "run").unwrap();
}

#[test]
fn one_approval_is_consumed_once_across_two_processes() {
    let root = std::env::temp_dir().join(format!("estate-approval-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let book = Book::new(FsTable::open(&root));
    book.issue_approval(
        "reviewed",
        &sim_lib_estate_book::ApprovalUse {
            plan: Key("plan".into()),
            reviewer: "reviewer".into(),
            consumed_by: "run".into(),
        },
    )
    .unwrap();
    let exe = std::env::current_exe().unwrap();
    let mut children = (0..2)
        .map(|_| {
            Command::new(&exe)
                .args(["--exact", "approval_process_worker"])
                .env("SIM_ESTATE_APPROVAL_ROOT", &root)
                .spawn()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let successes = children
        .iter_mut()
        .map(|child| child.wait().unwrap().success())
        .filter(|success| *success)
        .count();
    assert_eq!(successes, 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn filesystem_table_rebuilds_byte_identical_projection() {
    let root = std::env::temp_dir().join(format!("estate-book-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let book = Book::new(FsTable::open(&root));
    let plan = book.intern(&("plan", 7)).unwrap();
    book.append("run", EventKind::Opened { plan }).unwrap();
    book.append("run", EventKind::DispatchIntent).unwrap();
    book.append(
        "run",
        EventKind::Final {
            state: "process-complete/unverified".into(),
        },
    )
    .unwrap();
    assert_eq!(
        book.projection("run").unwrap(),
        book.projection("run").unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}
