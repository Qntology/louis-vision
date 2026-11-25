//! Lock-free inference execution (`Prepare -> Infer -> Commit`) keeping state mutex hold < 2 ms.

use std::sync::Mutex;

use anyhow::{bail, Result};

const ATTEMPTS: usize = 3;

#[derive(Debug, Clone, PartialEq)]
pub enum Commit<T> {
    Done(T),
    Stale,
}

#[derive(Debug, Clone)]
pub struct StagedSnapshot<I> {
    pub rev: u64,
    pub input: I,
}

#[derive(Debug, Clone)]
pub struct StagedEngine<S> {
    pub rev: u64,
    pub state: S,
}

impl<S> StagedEngine<S> {
    pub fn new(state: S) -> Self {
        Self { rev: 0, state }
    }
}

pub trait EngineAccess<S> {
    fn engine<R>(&self, f: impl FnOnce(&mut StagedEngine<S>) -> R) -> R;
    fn infer<R>(&self, f: impl FnOnce() -> R) -> R {
        f()
    }
}

impl<S> EngineAccess<S> for Mutex<StagedEngine<S>> {
    fn engine<R>(&self, f: impl FnOnce(&mut StagedEngine<S>) -> R) -> R {
        f(&mut self.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

/// Executes `prepare -> infer (no engine lock) -> commit (CAS rev)` with up to `ATTEMPTS` retries.
pub fn run_staged<S, I, O, T>(
    access: &impl EngineAccess<S>,
    mut prepare: impl FnMut(&mut StagedEngine<S>) -> Result<StagedSnapshot<I>>,
    mut infer_fn: impl FnMut(&I) -> Result<O>,
    mut commit_fn: impl FnMut(&mut StagedEngine<S>, u64, O) -> Result<Commit<T>>,
) -> Result<T> {
    for _ in 0..ATTEMPTS {
        let snap = access.engine(|e| prepare(e))?;
        let out = access.infer(|| infer_fn(&snap.input))?;
        match access.engine(|e| -> Result<Commit<T>> {
            if e.rev != snap.rev {
                Ok(Commit::Stale)
            } else {
                let res = commit_fn(e, snap.rev, out)?;
                if matches!(res, Commit::Done(_)) {
                    e.rev += 1;
                }
                Ok(res)
            }
        })? {
            Commit::Done(val) => return Ok(val),
            Commit::Stale => continue,
        }
    }
    bail!("staged commit conflicted repeatedly across {ATTEMPTS} attempts")
}
