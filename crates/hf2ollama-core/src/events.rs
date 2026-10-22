use crate::model::ImportRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Queued,
    Downloading,
    Verifying,
    Uploading,
    Creating,
    CleaningUp,
    Done,
}

#[derive(Debug, Clone)]
pub enum JobEvent {
    Stage { id: JobId, stage: Stage },
    Progress { id: JobId, done: u64, total: u64 },
    Log { id: JobId, line: String },
    Paused { id: JobId },
    Resumed { id: JobId },
    Finished { id: JobId },
    Failed { id: JobId, error: String },
    Cancelled { id: JobId },
}

// Placeholder so the test below fails on an assertion path, not a build error.
impl PartialEq for JobEvent {
    fn eq(&self, _other: &Self) -> bool {
        todo!()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    StartImport(ImportRequest),
    Pause(JobId),
    Resume(JobId),
    Cancel(JobId),
}
