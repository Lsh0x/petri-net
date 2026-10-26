//! CloudFormation-style dependency graphs compiled into numeric Petri nets.
//!
//! Every edge has its own internal place. Completing a task produces one token
//! per dependent plus a persistent completion marker. Joining consumes one token
//! from each dependency, so shared prerequisites and crossed joins are supported.
use crate::engine::{
    self, Arc, Authenticator, Definition, RunId, Snapshot, Storage, Token, Transition,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc as Shared;

pub type TaskId = u32;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_TASKS: usize = 1024;
const MAX_PLACES: usize = 4096;

/// Parsing, graph validation and execution errors. Symbolic names are diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Parse(String),
    UnsupportedVersion(u32),
    EmptyWorkflow,
    EmptyName,
    DuplicateId(TaskId),
    UnknownDependency { task: String, dependency: String },
    DuplicateDependency { task: String, dependency: String },
    Cycle,
    Limit,
    UnknownTask(TaskId),
    InvalidTicket,
    Engine(engine::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<engine::Error> for Error {
    fn from(value: engine::Error) -> Self {
        Self::Engine(value)
    }
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Task {
    id: TaskId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(
        default,
        deserialize_with = "dependencies",
        skip_serializing_if = "Vec::is_empty"
    )]
    depends_on: Vec<String>,
}
fn dependencies<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Input {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Input::deserialize(d)? {
        Input::One(value) => vec![value],
        Input::Many(values) => values,
    })
}

// BTreeMap's default deserializer silently replaces duplicate keys. Reject them
// at the boundary for both YAML and JSON rather than validating a lossy map.
fn tasks<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<BTreeMap<String, Task>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, Task>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique task names")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut tasks = BTreeMap::new();
            while let Some((name, task)) = map.next_entry::<String, Task>()? {
                if tasks.len() >= MAX_TASKS {
                    return Err(serde::de::Error::custom("too many tasks"));
                }
                if tasks.insert(name.clone(), task).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "duplicate task name: {name}"
                    )));
                }
            }
            Ok(tasks)
        }
    }
    d.deserialize_map(Visitor)
}

/// Validated DAG. Deserialize only through `from_yaml` or `from_json`.
/// Task names are symbolic references; IDs used by the execution API are numeric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Workflow {
    version: u32,
    tasks: BTreeMap<String, Task>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct Input {
    version: u32,
    #[serde(deserialize_with = "tasks")]
    tasks: BTreeMap<String, Task>,
}
impl Workflow {
    pub fn from_yaml(input: &str) -> Result<Self> {
        check_size(input)?;
        let input: Input =
            serde_saphyr::from_str(input).map_err(|e| Error::Parse(e.to_string()))?;
        Self::validate(input)
    }
    pub fn from_json(input: &str) -> Result<Self> {
        check_size(input)?;
        let input: Input = serde_json::from_str(input).map_err(|e| Error::Parse(e.to_string()))?;
        Self::validate(input)
    }
    pub fn to_yaml(&self) -> Result<String> {
        let result = serde_saphyr::to_string(self).map_err(|e| Error::Parse(e.to_string()))?;
        check_size(&result)?;
        Ok(result)
    }
    pub fn to_json(&self) -> Result<String> {
        let result = serde_json::to_string(self).map_err(|e| Error::Parse(e.to_string()))?;
        check_size(&result)?;
        Ok(result)
    }
    pub fn task_id(&self, name: &str) -> Option<TaskId> {
        self.tasks.get(name).map(|t| t.id)
    }
    pub fn task_name(&self, id: TaskId) -> Option<&str> {
        self.tasks
            .iter()
            .find(|(_, t)| t.id == id)
            .map(|(name, _)| name.as_str())
    }
    pub fn description(&self, id: TaskId) -> Option<&str> {
        self.tasks
            .values()
            .find(|t| t.id == id)
            .and_then(|t| t.description.as_deref())
    }
    fn validate(mut input: Input) -> Result<Self> {
        if input.version != 1 {
            return Err(Error::UnsupportedVersion(input.version));
        }
        if input.tasks.is_empty() {
            return Err(Error::EmptyWorkflow);
        }
        if input.tasks.len() > MAX_TASKS {
            return Err(Error::Limit);
        }
        let mut ids = BTreeSet::new();
        let mut remaining = BTreeMap::new();
        let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut edges = 0;
        for (name, task) in &input.tasks {
            if name.trim().is_empty() {
                return Err(Error::EmptyName);
            }
            if !ids.insert(task.id) {
                return Err(Error::DuplicateId(task.id));
            }
            let mut seen = BTreeSet::new();
            for dependency in &task.depends_on {
                if !input.tasks.contains_key(dependency) {
                    return Err(Error::UnknownDependency {
                        task: name.clone(),
                        dependency: dependency.clone(),
                    });
                }
                if !seen.insert(dependency) {
                    return Err(Error::DuplicateDependency {
                        task: name.clone(),
                        dependency: dependency.clone(),
                    });
                }
                children
                    .entry(dependency.clone())
                    .or_default()
                    .push(name.clone());
                edges += 1;
            }
            remaining.insert(name.clone(), task.depends_on.len());
        }
        let mut queue: VecDeque<_> = remaining
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(name, _)| name.clone())
            .collect();
        let roots = queue.len();
        // Includes the reserved initial place, root inputs, edge places and markers.
        if 1 + roots + edges + input.tasks.len() > MAX_PLACES {
            return Err(Error::Limit);
        }
        let mut visited = 0;
        while let Some(name) = queue.pop_front() {
            visited += 1;
            for child in children.get(&name).into_iter().flatten() {
                let count = remaining.get_mut(child).expect("known child");
                *count -= 1;
                if *count == 0 {
                    queue.push_back(child.clone());
                }
            }
        }
        if visited != input.tasks.len() {
            return Err(Error::Cycle);
        }
        for task in input.tasks.values_mut() {
            task.depends_on.sort();
        }
        Ok(Self {
            version: input.version,
            tasks: input.tasks,
        })
    }
    fn compile(&self) -> Result<Compiled> {
        let by_id: BTreeMap<_, _> = self.tasks.values().map(|task| (task.id, task)).collect();
        let mut edges = BTreeSet::new();
        for task in by_id.values() {
            for dependency in &task.depends_on {
                edges.insert((self.tasks[dependency].id, task.id));
            }
        }
        let mut next = 1u32;
        let mut root_places = BTreeMap::new();
        for task in by_id.values().filter(|t| t.depends_on.is_empty()) {
            root_places.insert(task.id, next);
            next += 1;
        }
        let mut edge_places = BTreeMap::new();
        for edge in edges {
            edge_places.insert(edge, next);
            next += 1;
        }
        let mut compiled = BTreeMap::new();
        let mut transitions = Vec::new();
        for task in by_id.values() {
            let mut dependencies: Vec<_> = task
                .depends_on
                .iter()
                .map(|name| self.tasks[name].id)
                .collect();
            dependencies.sort_unstable();
            let inputs: Vec<_> = if dependencies.is_empty() {
                vec![root_places[&task.id]]
            } else {
                dependencies
                    .iter()
                    .map(|id| edge_places[&(*id, task.id)])
                    .collect()
            };
            let mut outputs: Vec<_> = edge_places
                .iter()
                .filter(|((from, _), _)| *from == task.id)
                .map(|(_, place)| *place)
                .collect();
            let marker = next;
            next += 1;
            outputs.push(marker);
            transitions.push(Transition::new(
                task.id,
                inputs.iter().map(|p| Arc::new(*p, 1)).collect(),
                outputs.iter().map(|p| Arc::new(*p, 1)).collect(),
            ));
            compiled.insert(
                task.id,
                CompiledTask {
                    dependencies,
                    inputs,
                    outputs,
                    marker,
                },
            );
        }
        let definition =
            Definition::new((0..next).collect(), 0, transitions)?.for_dependency_graph();
        Ok(Compiled {
            definition,
            tasks: compiled,
            root_places,
        })
    }
}
fn check_size(input: &str) -> Result<()> {
    if input.len() > MAX_BYTES {
        Err(Error::Limit)
    } else {
        Ok(())
    }
}

struct CompiledTask {
    dependencies: Vec<TaskId>,
    inputs: Vec<u32>,
    outputs: Vec<u32>,
    marker: u32,
}
struct Compiled {
    definition: Definition,
    tasks: BTreeMap<TaskId, CompiledTask>,
    root_places: BTreeMap<TaskId, u32>,
}

/// Portable task activation. Parsing is not authorization: call `granted` or
/// `executed` before trusting its payloads. Embedded tokens are authenticated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskToken {
    run: RunId,
    task: TaskId,
    dependencies: Vec<TaskId>,
    inputs: Vec<Token>,
}
impl TaskToken {
    pub fn run_id(&self) -> RunId {
        self.run
    }
    pub fn task_id(&self) -> TaskId {
        self.task
    }
    pub fn dependencies(&self) -> &[TaskId] {
        &self.dependencies
    }
    /// Root data, supplied to `start`; non-root tasks return None.
    pub fn initial_data(&self) -> Option<&Value> {
        if self.dependencies.is_empty() {
            self.inputs.first().map(Token::data)
        } else {
            None
        }
    }
    /// Result produced by a particular direct prerequisite.
    pub fn input(&self, task: TaskId) -> Option<&Value> {
        self.dependencies
            .iter()
            .position(|id| *id == task)
            .and_then(|i| self.inputs.get(i).map(Token::data))
    }
    pub fn to_json(&self) -> Result<String> {
        let json = serde_json::to_string(self).map_err(|e| Error::Parse(e.to_string()))?;
        check_size(&json)?;
        Ok(json)
    }
    pub fn from_json(input: &str) -> Result<Self> {
        check_size(input)?;
        serde_json::from_str(input).map_err(|e| Error::Parse(e.to_string()))
    }
}

/// Authenticated completion receipt. It is not a task activation token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    task: TaskId,
    token: Token,
}
impl Completion {
    pub fn task_id(&self) -> TaskId {
        self.task
    }
    pub fn run_id(&self) -> RunId {
        self.token.run_id()
    }
    pub fn result(&self) -> &Value {
        self.token.data()
    }
}

/// One consistent view of an execution. Ready tickets are advisory, not reserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub revision: u64,
    pub completed: Vec<TaskId>,
    pub ready: Vec<TaskToken>,
    pub finished: bool,
}

/// High-level executor: no numeric internal places/arcs are exposed to callers.
/// A validated DAG gives each task at most one completion per execution.
pub struct Petri<S: Storage> {
    workflow: Workflow,
    compiled: Compiled,
    engine: engine::Petri<S>,
}
impl<S: Storage> Petri<S> {
    pub fn new(
        workflow: Workflow,
        storage: Shared<S>,
        auth: Shared<Authenticator>,
    ) -> Result<Self> {
        let compiled = workflow.compile()?;
        let engine = engine::Petri::new(compiled.definition.clone(), storage, auth)?;
        Ok(Self {
            workflow,
            compiled,
            engine,
        })
    }
    pub fn workflow(&self) -> &Workflow {
        &self.workflow
    }
    /// Creates all root activations in one atomic insert. No partial bootstrap.
    pub async fn start(&self, run: RunId, data: Value) -> Result<Vec<TaskToken>> {
        let entries = self
            .compiled
            .root_places
            .values()
            .map(|place| (*place, data.clone()))
            .collect();
        let tokens = self.engine.start_marking(run, entries).await?;
        let mut roots = Vec::new();
        for (task, place) in &self.compiled.root_places {
            roots.push(TaskToken {
                run,
                task: *task,
                dependencies: vec![],
                inputs: tokens
                    .iter()
                    .filter(|t| t.place() == *place)
                    .cloned()
                    .collect(),
            });
        }
        Ok(roots)
    }
    pub async fn ready(&self, run: RunId) -> Result<Vec<TaskToken>> {
        Ok(self.status(run).await?.ready)
    }
    pub async fn status(&self, run: RunId) -> Result<Status> {
        let snapshot = self.engine.snapshot(run).await?;
        let mut completed = Vec::new();
        let mut ready = Vec::new();
        for (id, task) in &self.compiled.tasks {
            if snapshot
                .tokens()
                .iter()
                .any(|token| token.place() == task.marker)
            {
                completed.push(*id);
                continue;
            }
            let inputs: Option<Vec<_>> = task
                .inputs
                .iter()
                .map(|place| {
                    snapshot
                        .tokens()
                        .iter()
                        .find(|t| t.place() == *place)
                        .cloned()
                })
                .collect();
            if let Some(inputs) = inputs {
                ready.push(TaskToken {
                    run,
                    task: *id,
                    dependencies: task.dependencies.clone(),
                    inputs,
                });
            }
        }
        Ok(Status {
            revision: snapshot.revision(),
            finished: completed.len() == self.compiled.tasks.len(),
            completed,
            ready,
        })
    }
    fn check_ticket(&self, ticket: &TaskToken) -> Result<&CompiledTask> {
        let task = self
            .compiled
            .tasks
            .get(&ticket.task)
            .ok_or(Error::UnknownTask(ticket.task))?;
        if ticket.dependencies != task.dependencies
            || ticket.inputs.len() != task.inputs.len()
            || ticket
                .inputs
                .iter()
                .zip(&task.inputs)
                .any(|(t, place)| t.place() != *place || t.run_id() != ticket.run)
        {
            return Err(Error::InvalidTicket);
        }
        Ok(task)
    }
    pub async fn granted(&self, ticket: &TaskToken) -> Result<()> {
        self.check_ticket(ticket)?;
        self.engine
            .granted(ticket.run, ticket.task, &ticket.inputs)
            .await?;
        Ok(())
    }
    /// Records declared completion and atomically produces all dependent inputs.
    /// Caller output is copied to every outgoing edge and the completion receipt.
    pub async fn executed(&self, ticket: TaskToken, result: Value) -> Result<Completion> {
        let task = self.check_ticket(&ticket)?;
        let outputs = self
            .engine
            .execute(
                ticket.run,
                ticket.task,
                &ticket.inputs,
                vec![result; task.outputs.len()],
            )
            .await?;
        let token = outputs
            .into_iter()
            .find(|t| t.place() == task.marker)
            .expect("compiled completion marker");
        Ok(Completion {
            task: ticket.task,
            token,
        })
    }
    pub fn verify_completion(&self, completion: &Completion) -> Result<()> {
        let task = self
            .compiled
            .tasks
            .get(&completion.task)
            .ok_or(Error::UnknownTask(completion.task))?;
        self.engine.verify_token(&completion.token)?;
        if completion.token.place() != task.marker {
            return Err(Error::InvalidTicket);
        }
        Ok(())
    }
    pub async fn snapshot(&self, run: RunId) -> Result<Snapshot> {
        Ok(self.engine.snapshot(run).await?)
    }
    pub async fn restore(&self, snapshot: Snapshot) -> Result<()> {
        Ok(self.engine.restore(snapshot).await?)
    }
    #[cfg(feature = "tiercache")]
    pub async fn archive(&self, run: RunId, cache: &tiercache::Cache) -> Result<Snapshot> {
        Ok(self.engine.archive(run, cache).await?)
    }
    #[cfg(feature = "tiercache")]
    pub async fn read_archive(
        &self,
        run: RunId,
        revision: u64,
        cache: &tiercache::Cache,
    ) -> Result<Option<Snapshot>> {
        Ok(self.engine.read_archive(run, revision, cache).await?)
    }
}
