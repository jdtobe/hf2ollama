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

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    StartImport(ImportRequest),
    Pause(JobId),
    Resume(JobId),
    Cancel(JobId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn job_event_roundtrips_through_channel() {
        let (tx, rx) = mpsc::channel();
        let ev = JobEvent::Progress {
            id: JobId(1),
            done: 5,
            total: 10,
        };
        tx.send(ev.clone()).unwrap();
        assert_eq!(rx.recv().unwrap(), ev);
    }
}
