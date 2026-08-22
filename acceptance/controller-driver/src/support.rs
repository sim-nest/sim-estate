use serde::Serialize;
use sha2::{Digest, Sha256};
use sim_estate_core::{EstateError, ProjectFingerprint, Symbol};
use sim_lib_estate::{Clock, EffectGate, EffectRequest, Error};
use sim_lib_estate_book::{BookError, Key, Table};
use sim_lib_exec::{
    BindingValue, DispatchEvidence, PrivateArtifactRef, ProcResult, ProcessAttempt, ProcessBudget,
    ProcessCancellation, ProcessPort, ProcessReceipt, ProcessRefusal, ProcessRequest, ProgramRef,
    ProjectRootRef,
};
use sim_site_estate_ansible::PrivateArtifacts;
use sim_site_estate_command::{ProjectInput, fingerprint_project};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::Instant,
};

#[derive(Default, Clone)]
pub struct SharedTable(std::sync::Arc<Mutex<BTreeMap<Key, Vec<u8>>>>);
impl Table for SharedTable {
    fn get(&self, key: &Key) -> Result<Option<Vec<u8>>, BookError> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn put_absent(&self, key: &Key, value: &[u8]) -> Result<(), BookError> {
        let mut table = self.0.lock().unwrap();
        match table.get(key) {
            Some(old) if old == value => Ok(()),
            Some(old) => Err(BookError::Conflict {
                key: key.clone(),
                observed: Some(old.clone()),
            }),
            None => {
                table.insert(key.clone(), value.to_vec());
                Ok(())
            }
        }
    }
    fn compare_and_swap(
        &self,
        key: &Key,
        expected: Option<&[u8]>,
        value: &[u8],
    ) -> Result<(), BookError> {
        let mut table = self.0.lock().unwrap();
        let observed = table.get(key).cloned();
        if observed.as_deref() != expected {
            return Err(BookError::Conflict {
                key: key.clone(),
                observed,
            });
        }
        table.insert(key.clone(), value.to_vec());
        Ok(())
    }
    fn scan_prefix(&self, prefix: &str) -> Result<Vec<(Key, Vec<u8>)>, BookError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(key, _)| key.0.starts_with(prefix))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

pub struct NativePort {
    programs: BTreeMap<ProgramRef, PathBuf>,
    roots: BTreeMap<ProjectRootRef, PathBuf>,
    artifacts: BTreeMap<PrivateArtifactRef, PathBuf>,
    attempts: Mutex<Vec<AttemptSummary>>,
}
impl NativePort {
    pub fn new(private_root: &Path, work: &Path) -> Result<Self, String> {
        let artifacts = [
            ("artifact/callback", work.join("callback")),
            ("artifact/events", work.join("events.jsonl")),
            ("artifact/human", work.join("human.log")),
            ("artifact/local-temp", work.join("local-temp")),
        ]
        .into_iter()
        .map(|(key, path)| {
            Ok((
                PrivateArtifactRef::new(key).map_err(|e| e.to_string())?,
                path,
            ))
        })
        .collect::<Result<_, String>>()?;
        Ok(Self {
            programs: [
                ("program/ansible-inventory", which("ansible-inventory")?),
                ("program/ansible-config", which("ansible-config")?),
                ("program/make", which("make")?),
            ]
            .into_iter()
            .map(|(key, path)| Ok((ProgramRef::new(key).map_err(|e| e.to_string())?, path)))
            .collect::<Result<_, String>>()?,
            roots: [(
                ProjectRootRef::new("project/private").map_err(|e| e.to_string())?,
                private_root.to_path_buf(),
            )]
            .into(),
            artifacts,
            attempts: Mutex::default(),
        })
    }
    pub fn attempts(&self) -> Result<Vec<AttemptSummary>, String> {
        Ok(self.attempts.lock().map_err(|e| e.to_string())?.clone())
    }
}
impl ProcessPort for NativePort {
    fn run(&self, request: &ProcessRequest, cancel: &ProcessCancellation) -> ProcessAttempt {
        if cancel.is_cancelled() {
            return refusal("cancelled before dispatch");
        }
        let Some(program) = self.programs.get(&request.program) else {
            return refusal("program unavailable");
        };
        let Some(root) = self.roots.get(&request.root) else {
            return refusal("root unavailable");
        };
        let env = match render_env(request, &self.artifacts) {
            Ok(value) => value,
            Err(value) => return value,
        };
        let started = Instant::now();
        let output = Command::new(program)
            .args(request.argv.iter().map(|arg| arg.as_str()))
            .current_dir(root)
            .env_clear()
            .envs(env.iter())
            .output();
        match output {
            Ok(output) => {
                let elapsed = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
                let (stdout, stderr, truncated) =
                    bounded_output(&output.stdout, &output.stderr, &request.budget);
                let receipt = ProcessReceipt {
                    provider: "xtask/native-process-port".into(),
                    elapsed_mono_ns: elapsed,
                    result: ProcResult {
                        stdout: stdout.clone(),
                        stderr: stderr.clone(),
                        exit_code: output.status.code().unwrap_or(-1),
                        truncated,
                    },
                };
                self.record(request, &env, &receipt);
                ProcessAttempt::Completed { receipt }
            }
            Err(error) => ProcessAttempt::UnknownAfterDispatch {
                evidence: DispatchEvidence {
                    provider: "xtask/native-process-port".into(),
                    stage: "spawn".into(),
                    detail: error.to_string(),
                },
            },
        }
    }
}
impl NativePort {
    fn record(&self, request: &ProcessRequest, env: &BTreeMap<String, String>, r: &ProcessReceipt) {
        let summary = AttemptSummary {
            program: request.program.as_str().into(),
            argv_sha256: hash_json(
                &request
                    .argv
                    .iter()
                    .map(|arg| arg.as_str())
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default(),
            environment_sha256: hash_json(env).unwrap_or_default(),
            exit_code: r.result.exit_code,
            stdout_sha256: hash_text(&r.result.stdout),
            stderr_sha256: hash_text(&r.result.stderr),
            truncated: r.result.truncated,
        };
        self.attempts.lock().unwrap().push(summary);
    }
}

#[derive(Clone, Serialize)]
pub struct AttemptSummary {
    program: String,
    argv_sha256: String,
    environment_sha256: String,
    exit_code: i32,
    stdout_sha256: String,
    stderr_sha256: String,
    truncated: bool,
}

pub struct FsArtifacts {
    values: BTreeMap<PrivateArtifactRef, PathBuf>,
}
impl FsArtifacts {
    pub fn new(work: &Path) -> Result<Self, String> {
        Ok(Self {
            values: [
                ("artifact/events", work.join("events.jsonl")),
                ("artifact/human", work.join("human.log")),
                ("artifact/callback", work.join("callback")),
                ("artifact/local-temp", work.join("local-temp")),
            ]
            .into_iter()
            .map(|(key, path)| {
                Ok((
                    PrivateArtifactRef::new(key).map_err(|e| e.to_string())?,
                    path,
                ))
            })
            .collect::<Result<_, String>>()?,
        })
    }
}
impl PrivateArtifacts for FsArtifacts {
    fn read(&self, artifact: &PrivateArtifactRef) -> Result<Vec<u8>, EstateError> {
        let path = self
            .values
            .get(artifact)
            .ok_or(EstateError::NotDispatched)?;
        fs::read(path).map_err(|_| EstateError::MalformedEvent)
    }
}

fn render_env(
    request: &ProcessRequest,
    artifacts: &BTreeMap<PrivateArtifactRef, PathBuf>,
) -> Result<BTreeMap<String, String>, ProcessAttempt> {
    request
        .environment
        .iter()
        .map(|(name, value)| {
            let rendered = match value {
                BindingValue::Literal(value) => value.clone(),
                BindingValue::ProjectRoot(_) => return Err(refusal("project env unsupported")),
                BindingValue::PrivateArtifact(value) => artifacts
                    .get(value)
                    .ok_or_else(|| refusal("artifact unavailable"))?
                    .to_string_lossy()
                    .into_owned(),
            };
            Ok((name.to_owned(), rendered))
        })
        .collect()
}

#[derive(Default)]
pub struct Gate;
impl EffectGate for Gate {
    fn perform<R>(
        &mut self,
        _: &EffectRequest,
        action: impl FnOnce() -> Result<R, EstateError>,
    ) -> Result<R, Error> {
        action().map_err(Into::into)
    }
}
pub struct Time(pub u64);
impl Clock for Time {
    fn now(&self) -> u64 {
        self.0
    }
}

fn refusal(detail: &str) -> ProcessAttempt {
    ProcessAttempt::NotDispatched {
        refusal: ProcessRefusal::Refused(detail.into()),
    }
}

fn bounded_output(stdout: &[u8], stderr: &[u8], budget: &ProcessBudget) -> (String, String, bool) {
    let cap = budget.max_output_bytes;
    let out_keep = stdout.len().min(cap);
    let err_keep = stderr.len().min(cap.saturating_sub(out_keep));
    (
        String::from_utf8_lossy(&stdout[..out_keep]).into_owned(),
        String::from_utf8_lossy(&stderr[..err_keep]).into_owned(),
        out_keep < stdout.len() || err_keep < stderr.len(),
    )
}

fn which(name: &str) -> Result<PathBuf, String> {
    let paths = std::env::var_os("PATH").ok_or("PATH is unavailable")?;
    std::env::split_paths(&paths)
        .map(|path| path.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| format!("{name} is unavailable"))
}

pub fn sym(value: &str) -> Result<Symbol, String> {
    Symbol::new(value).map_err(|e| e.to_string())
}

pub fn opaque_target(value: &str) -> Result<Symbol, String> {
    sym(&format!("host/{}", &hash_text(value)[..32]))
}

pub fn project_fingerprint(
    root: &Path,
    binding_digest: &str,
    operation: &str,
) -> Result<ProjectFingerprint, String> {
    let head = command_text(root, "git", &["rev-parse", "HEAD"])?;
    let state = git_state_digest(root)?;
    Ok(fingerprint_project(
        head.trim(),
        &[ProjectInput {
            name: "project-state",
            digest: hash32(state.as_bytes()),
        }],
        &[],
        hash32(binding_digest.as_bytes()),
        hash32(operation.as_bytes()),
    ))
}

pub fn git_state_digest(root: &Path) -> Result<String, String> {
    let head = command_text(root, "git", &["rev-parse", "HEAD"])?;
    let status = Command::new("git")
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !status.status.success() {
        return Err("git status failed".into());
    }
    Ok(hash_text(&format!(
        "{}:{}",
        head.trim(),
        hash_bytes(&status.stdout)
    )))
}

fn command_text(root: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("{program} {args:?} failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    fs::read(path)
        .map(|bytes| hash_bytes(&bytes))
        .map_err(|e| e.to_string())
}

pub fn hash_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| hash_bytes(&bytes))
        .map_err(|e| e.to_string())
}

pub fn hash_text(value: &str) -> String {
    hash_bytes(value.as_bytes())
}

fn hash_bytes(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn hash32(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}
