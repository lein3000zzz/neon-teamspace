//! A [`Runner`] for tests: it answers from what it was given, and keeps what it ran.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::anyhow;

use super::{Program, Runner};
use crate::output::Output;

/// A command the fake ran: its program, arguments and added environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ran {
    pub(crate) program: Program,
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<(String, String)>,
}

impl Ran {
    /// The program and its arguments, joined by spaces.
    pub(crate) fn line(&self) -> String {
        std::iter::once(self.program.name())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Answers [`Runner::output`] from `outputs`, by the program and its arguments joined by
/// spaces, and fails it for any other; fails [`Runner::run`] for a command that starts with a
/// key of `failures`. Every command run is kept, in order.
#[derive(Debug, Default)]
pub(crate) struct Fake {
    pub(crate) outputs: HashMap<String, String>,
    pub(crate) failures: Vec<(String, String)>,
    ran: Mutex<Vec<Ran>>,
}

impl Fake {
    /// Answers `line` with `output`.
    pub(crate) fn answer(&mut self, line: &str, output: &str) -> &mut Self {
        self.outputs.insert(line.to_owned(), output.to_owned());
        self
    }

    /// Fails each command run that starts with `prefix`, saying `error`.
    pub(crate) fn fail(&mut self, prefix: &str, error: &str) -> &mut Self {
        self.failures.push((prefix.to_owned(), error.to_owned()));
        self
    }

    /// The commands run, in order.
    pub(crate) fn ran(&self) -> Vec<Ran> {
        self.ran.lock().unwrap().clone()
    }

    /// The commands run, each as [`Ran::line`] has it.
    pub(crate) fn lines(&self) -> Vec<String> {
        self.ran().iter().map(Ran::line).collect()
    }
}

impl Runner for Fake {
    async fn output(
        &self,
        program: Program,
        args: &[String],
        env: &[(String, String)],
    ) -> anyhow::Result<Vec<u8>> {
        let ran = Ran {
            program,
            args: args.to_vec(),
            env: env.to_vec(),
        };
        let line = ran.line();
        self.ran.lock().unwrap().push(ran);
        self.outputs
            .get(&line)
            .map(|output| output.clone().into_bytes())
            .ok_or_else(|| anyhow!("{line}: no such object"))
    }

    async fn run(
        &self,
        program: Program,
        args: &[String],
        env: &[(String, String)],
        out: &dyn Output,
    ) -> anyhow::Result<()> {
        let ran = Ran {
            program,
            args: args.to_vec(),
            env: env.to_vec(),
        };
        let line = ran.line();
        self.ran.lock().unwrap().push(ran);
        out.line(&format!("ran {}", args.last().map_or("", String::as_str)));
        match self
            .failures
            .iter()
            .find(|(prefix, _)| line.starts_with(prefix.as_str()))
        {
            Some((_, error)) => Err(anyhow!("{error}")),
            None => Ok(()),
        }
    }
}
