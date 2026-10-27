//! Pure job bookkeeping for the downloads dock (no egui).

use hf2ollama_core::config::AutoDismiss;
use hf2ollama_core::events::{JobEvent, JobId, Stage};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Speed is averaged over progress reports from the last five seconds.
pub const RATE_WINDOW: Duration = Duration::from_secs(5);

/// Most log lines kept per row; older lines are dropped first.
pub const MAX_LOG_LINES: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStatus {
    Queued,
    Running,
    Paused,
    Finished,
    Failed,
    Cancelled,
}

impl RowStatus {
    /// Finished, failed and cancelled rows wait for dismissal.
    pub fn is_settled(self) -> bool {
        matches!(
            self,
            RowStatus::Finished | RowStatus::Failed | RowStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobRow {
    pub id: JobId,
    pub name: String,
    pub total: u64,
    pub done: u64,
    pub stage: Stage,
    pub status: RowStatus,
    pub error: Option<String>,
    pub log: Vec<String>,
    pub settled_at: Option<Instant>,
    pub paused: bool,
    /// Bytes per second over the rolling window, while downloading.
    rate: Option<f64>,
    /// `(when, bytes done)` reports inside the rolling window.
    samples: VecDeque<(Instant, u64)>,
}

impl JobRow {
    fn placeholder(id: JobId) -> JobRow {
        JobRow {
            id,
            name: String::new(),
            total: 0,
            done: 0,
            stage: Stage::Queued,
            status: RowStatus::Queued,
            error: None,
            log: Vec::new(),
            settled_at: None,
            paused: false,
            rate: None,
            samples: VecDeque::new(),
        }
    }

    /// Download progress as a fraction in 0..=1.
    pub fn fraction(&self) -> f32 {
        if self.status == RowStatus::Finished {
            return 1.0;
        }
        if self.total == 0 {
            return 0.0;
        }
        (self.done as f64 / self.total as f64).clamp(0.0, 1.0) as f32
    }

    /// Status column text, e.g. `Downloading 42%` or `Ready in Ollama`.
    pub fn status_text(&self) -> String {
        let pct = (self.fraction() * 100.0).floor() as u32;
        match self.status {
            RowStatus::Queued => "Queued".to_string(),
            RowStatus::Paused => format!("Paused {pct}%"),
            RowStatus::Finished => "Ready in Ollama".to_string(),
            RowStatus::Failed => "Failed".to_string(),
            RowStatus::Cancelled => "Cancelled".to_string(),
            RowStatus::Running => match self.stage {
                Stage::Queued => "Queued".to_string(),
                Stage::Downloading => match self.rate {
                    Some(rate) => format!("Downloading {pct}% · {}", format_rate(rate)),
                    None => format!("Downloading {pct}%"),
                },
                Stage::Verifying => "Verifying".to_string(),
                Stage::Uploading => "Uploading".to_string(),
                Stage::Creating => "Creating".to_string(),
                Stage::CleaningUp => "Cleaning up".to_string(),
                Stage::Done => "Finishing".to_string(),
            },
        }
    }

    fn settle(&mut self, status: RowStatus, now: Instant) {
        if self.status.is_settled() {
            return;
        }
        self.status = status;
        self.paused = false;
        self.settled_at = Some(now);
        self.clear_rate();
    }

    fn clear_rate(&mut self) {
        self.rate = None;
        self.samples.clear();
    }

    /// Adds one progress report and recomputes the rate over the last
    /// `RATE_WINDOW`. Reports that do not advance the byte count are ignored
    /// so a stalled transfer does not average in zero-length samples.
    fn record_rate(&mut self, now: Instant) {
        if self
            .samples
            .back()
            .is_some_and(|(_, done)| *done >= self.done)
        {
            return;
        }
        self.samples.push_back((now, self.done));
        while let Some((at, _)) = self.samples.front() {
            if now.saturating_duration_since(*at) > RATE_WINDOW {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        self.rate = match (self.samples.front(), self.samples.back()) {
            (Some((first_at, first_done)), Some((last_at, last_done)))
                if self.samples.len() >= 2 =>
            {
                let secs = last_at.saturating_duration_since(*first_at).as_secs_f64();
                (secs > 0.0).then(|| last_done.saturating_sub(*first_done) as f64 / secs)
            }
            _ => None,
        };
    }

    fn live_status(&self) -> RowStatus {
        if self.paused {
            RowStatus::Paused
        } else if self.stage == Stage::Queued {
            RowStatus::Queued
        } else {
            RowStatus::Running
        }
    }
}

/// Formats a transfer rate, e.g. `12.4 MB/s`.
pub fn format_rate(bytes_per_sec: f64) -> String {
    let v = bytes_per_sec.max(0.0);
    if v < 1_000.0 {
        format!("{v:.0} B/s")
    } else if v < 1_000_000.0 {
        format!("{:.1} KB/s", v / 1e3)
    } else if v < 1_000_000_000.0 {
        format!("{:.1} MB/s", v / 1e6)
    } else {
        format!("{:.2} GB/s", v / 1e9)
    }
}

/// `tiny` and `tiny:latest` name the same Ollama model.
pub fn normalize_name(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{name}:latest")
    }
}

#[derive(Debug, Default)]
pub struct JobsState {
    /// Newest first (highest id first).
    rows: Vec<JobRow>,
}

impl JobsState {
    pub fn new() -> JobsState {
        JobsState::default()
    }

    pub fn rows(&self) -> &[JobRow] {
        &self.rows
    }

    /// Finds the row, creating a placeholder for an id not seen yet.
    fn row_mut(&mut self, id: JobId) -> &mut JobRow {
        let pos = match self.rows.iter().position(|r| r.id.0 <= id.0) {
            Some(i) if self.rows[i].id == id => i,
            Some(i) => {
                self.rows.insert(i, JobRow::placeholder(id));
                i
            }
            None => {
                self.rows.push(JobRow::placeholder(id));
                self.rows.len() - 1
            }
        };
        &mut self.rows[pos]
    }

    /// Names a job and records the size of what it will download.
    pub fn accept(&mut self, id: JobId, name: String, total: u64) {
        let row = self.row_mut(id);
        row.name = name;
        row.total = row.total.max(total);
    }

    /// True while a queued, running or paused job imports under `name`
    /// (`tiny` and `tiny:latest` are the same model).
    pub fn has_unsettled_job_named(&self, name: &str) -> bool {
        let want = normalize_name(name);
        self.rows
            .iter()
            .any(|r| !r.status.is_settled() && normalize_name(&r.name) == want)
    }

    pub fn apply(&mut self, ev: &JobEvent, now: Instant) {
        match ev {
            JobEvent::Stage { id, stage } => {
                let row = self.row_mut(*id);
                if row.status.is_settled() {
                    return;
                }
                if row.stage != *stage {
                    row.clear_rate();
                }
                row.stage = *stage;
                row.status = row.live_status();
            }
            JobEvent::Progress { id, done, total } => {
                let row = self.row_mut(*id);
                if row.status.is_settled() {
                    return;
                }
                row.total = row.total.max(*total);
                row.done = row.done.max(*done);
                if row.status == RowStatus::Running
                    && row.stage == Stage::Downloading
                    && !row.paused
                {
                    row.record_rate(now);
                }
            }
            JobEvent::Log { id, line } => {
                let row = self.row_mut(*id);
                row.log.push(line.clone());
                if row.log.len() > MAX_LOG_LINES {
                    let extra = row.log.len() - MAX_LOG_LINES;
                    row.log.drain(..extra);
                }
            }
            JobEvent::Paused { id } => {
                let row = self.row_mut(*id);
                if row.status.is_settled() {
                    return;
                }
                row.paused = true;
                row.status = RowStatus::Paused;
                row.clear_rate();
            }
            JobEvent::Resumed { id } => {
                let row = self.row_mut(*id);
                if row.status.is_settled() {
                    return;
                }
                row.paused = false;
                row.status = row.live_status();
                row.clear_rate();
            }
            JobEvent::Finished { id } => {
                let row = self.row_mut(*id);
                if !row.status.is_settled() {
                    row.stage = Stage::Done;
                }
                row.settle(RowStatus::Finished, now);
            }
            JobEvent::Failed { id, error } => {
                let row = self.row_mut(*id);
                if !row.status.is_settled() {
                    row.error = Some(error.clone());
                }
                row.settle(RowStatus::Failed, now);
            }
            JobEvent::Cancelled { id } => {
                self.row_mut(*id).settle(RowStatus::Cancelled, now);
            }
        }
    }

    pub fn dismiss(&mut self, id: JobId) {
        self.rows.retain(|r| r.id != id);
    }

    pub fn clear_finished(&mut self) {
        self.rows.retain(|r| !r.status.is_settled());
    }

    pub fn has_settled(&self) -> bool {
        self.rows.iter().any(|r| r.status.is_settled())
    }

    /// Ids of queued, running and paused jobs.
    pub fn unsettled_ids(&self) -> Vec<JobId> {
        self.rows
            .iter()
            .filter(|r| !r.status.is_settled())
            .map(|r| r.id)
            .collect()
    }

    fn active_count(&self) -> usize {
        self.rows.iter().filter(|r| !r.status.is_settled()).count()
    }

    /// Removes settled rows older than `auto`; returns when the next settled
    /// row will be due, or `None` when nothing is waiting.
    pub fn tick(&mut self, now: Instant, auto: AutoDismiss) -> Option<Instant> {
        let limit = auto.duration()?;
        self.rows.retain(|r| match r.settled_at {
            Some(at) if r.status.is_settled() => now.saturating_duration_since(at) < limit,
            _ => true,
        });
        self.rows
            .iter()
            .filter(|r| r.status.is_settled())
            .filter_map(|r| r.settled_at)
            .map(|at| at + limit)
            .min()
    }

    pub fn header_text(&self) -> String {
        match (self.active_count(), self.rows.is_empty()) {
            (0, true) => "No active jobs".to_string(),
            (0, false) => "All done".to_string(),
            (n, _) => format!("{n} active"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn id(n: u64) -> JobId {
        JobId(n)
    }

    fn t0() -> Instant {
        Instant::now()
    }

    fn stage(n: u64, stage: Stage) -> JobEvent {
        JobEvent::Stage { id: id(n), stage }
    }

    fn progress(n: u64, done: u64, total: u64) -> JobEvent {
        JobEvent::Progress {
            id: id(n),
            done,
            total,
        }
    }

    fn log(n: u64, line: &str) -> JobEvent {
        JobEvent::Log {
            id: id(n),
            line: line.into(),
        }
    }

    fn row(s: &JobsState, n: u64) -> &JobRow {
        s.rows().iter().find(|r| r.id == id(n)).expect("row exists")
    }

    /// One accepted job in the Downloading stage.
    fn running(s: &mut JobsState, n: u64, now: Instant) {
        s.accept(id(n), format!("model-{n}:q4"), 1000);
        s.apply(&stage(n, Stage::Queued), now);
        s.apply(&stage(n, Stage::Downloading), now);
    }

    #[test]
    fn progress_updates_row() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&progress(1, 420, 1000), now);
        let r = row(&s, 1);
        assert_eq!((r.done, r.total), (420, 1000));
        assert_eq!(r.status, RowStatus::Running);
        assert_eq!(r.stage, Stage::Downloading);
        assert!((r.fraction() - 0.42).abs() < 1e-6);
    }

    #[test]
    fn finished_row_stays_until_dismissed() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&JobEvent::Finished { id: id(1) }, now);
        let r = row(&s, 1);
        assert_eq!(r.status, RowStatus::Finished);
        assert_eq!(r.settled_at, Some(now));
        assert_eq!(r.fraction(), 1.0);
        s.apply(&log(1, "late line"), now + Duration::from_secs(1));
        assert_eq!(s.rows().len(), 1);
        s.dismiss(id(1));
        assert!(s.rows().is_empty());
    }

    #[test]
    fn cancelled_row_stays_with_log() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&log(1, "downloading"), now);
        s.apply(&log(1, "cancelled by user"), now);
        s.apply(&JobEvent::Cancelled { id: id(1) }, now);
        let r = row(&s, 1);
        assert_eq!(r.status, RowStatus::Cancelled);
        assert_eq!(r.log, vec!["downloading", "cancelled by user"]);
        assert_eq!(s.rows().len(), 1);
    }

    #[test]
    fn failed_row_stays_with_error_in_log() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&log(1, "failed: Ollama rejected the model"), now);
        s.apply(
            &JobEvent::Failed {
                id: id(1),
                error: "Ollama rejected the model".into(),
            },
            now,
        );
        let r = row(&s, 1);
        assert_eq!(r.status, RowStatus::Failed);
        assert_eq!(r.error.as_deref(), Some("Ollama rejected the model"));
        assert!(r.log.iter().any(|l| l.contains("Ollama rejected")));
        assert!(r.settled_at.is_some());
    }

    #[test]
    fn dismiss_removes_only_that_row() {
        let now = t0();
        let mut s = JobsState::new();
        for n in 1..=3 {
            running(&mut s, n, now);
        }
        s.apply(&JobEvent::Finished { id: id(2) }, now);
        s.dismiss(id(2));
        let ids: Vec<u64> = s.rows().iter().map(|r| r.id.0).collect();
        assert_eq!(ids, vec![3, 1]);
    }

    #[test]
    fn clear_finished_keeps_running() {
        let now = t0();
        let mut s = JobsState::new();
        for n in 1..=4 {
            running(&mut s, n, now);
        }
        s.apply(&JobEvent::Finished { id: id(1) }, now);
        s.apply(&JobEvent::Cancelled { id: id(2) }, now);
        s.apply(
            &JobEvent::Failed {
                id: id(3),
                error: "x".into(),
            },
            now,
        );
        s.apply(&JobEvent::Paused { id: id(4) }, now);
        s.clear_finished();
        let ids: Vec<u64> = s.rows().iter().map(|r| r.id.0).collect();
        assert_eq!(ids, vec![4]);
        assert_eq!(row(&s, 4).status, RowStatus::Paused);
    }

    #[test]
    fn header_text_variants() {
        let now = t0();
        let mut s = JobsState::new();
        assert_eq!(s.header_text(), "No active jobs");
        running(&mut s, 1, now);
        assert_eq!(s.header_text(), "1 active");
        running(&mut s, 2, now);
        s.apply(&JobEvent::Paused { id: id(2) }, now);
        assert_eq!(s.header_text(), "2 active");
        s.apply(&JobEvent::Finished { id: id(1) }, now);
        assert_eq!(s.header_text(), "1 active");
        s.apply(&JobEvent::Cancelled { id: id(2) }, now);
        assert_eq!(s.header_text(), "All done");
        s.clear_finished();
        assert_eq!(s.header_text(), "No active jobs");
    }

    #[test]
    fn log_lines_accumulate_in_order() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        for l in ["a", "b", "c"] {
            s.apply(&log(1, l), now);
        }
        assert_eq!(row(&s, 1).log, vec!["a", "b", "c"]);
    }

    #[test]
    fn log_is_capped_at_2000_lines() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        for i in 0..2005 {
            s.apply(&log(1, &format!("line {i}")), now);
        }
        let r = row(&s, 1);
        assert_eq!(r.log.len(), 2000);
        assert_eq!(r.log[0], "line 5");
        assert_eq!(r.log[1999], "line 2004");
    }

    #[test]
    fn events_before_job_accepted_create_placeholder_row() {
        let now = t0();
        let mut s = JobsState::new();
        s.apply(&stage(7, Stage::Queued), now);
        s.apply(&log(7, "queued"), now);
        assert_eq!(s.rows().len(), 1);
        let r = row(&s, 7);
        assert_eq!(r.name, "");
        assert_eq!(r.status, RowStatus::Queued);
        assert_eq!(r.log, vec!["queued"]);
    }

    #[test]
    fn job_accepted_fills_name_and_total() {
        let now = t0();
        let mut s = JobsState::new();
        s.apply(&stage(7, Stage::Queued), now);
        s.accept(id(7), "qwen:q4".into(), 4_680_000_000);
        assert_eq!(s.rows().len(), 1);
        let r = row(&s, 7);
        assert_eq!(r.name, "qwen:q4");
        assert_eq!(r.total, 4_680_000_000);
        assert_eq!(r.log, Vec::<String>::new());
    }

    #[test]
    fn progress_regression_is_ignored() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&progress(1, 600, 1000), now);
        s.apply(&progress(1, 200, 1000), now);
        assert_eq!(row(&s, 1).done, 600);
    }

    #[test]
    fn pause_for_settled_row_is_ignored() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&JobEvent::Finished { id: id(1) }, now);
        s.apply(&JobEvent::Paused { id: id(1) }, now);
        let r = row(&s, 1);
        assert_eq!(r.status, RowStatus::Finished);
        assert!(!r.paused);
    }

    #[test]
    fn pause_and_resume_toggle_the_row() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&JobEvent::Paused { id: id(1) }, now);
        assert_eq!(row(&s, 1).status, RowStatus::Paused);
        assert!(row(&s, 1).paused);
        s.apply(&JobEvent::Resumed { id: id(1) }, now);
        assert_eq!(row(&s, 1).status, RowStatus::Running);
        assert!(!row(&s, 1).paused);
    }

    #[test]
    fn rows_are_newest_first() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 2, now);
        running(&mut s, 5, now);
        // An event for an older, unseen id lands in id order.
        s.apply(&stage(3, Stage::Queued), now);
        let ids: Vec<u64> = s.rows().iter().map(|r| r.id.0).collect();
        assert_eq!(ids, vec![5, 3, 2]);
    }

    // ---- auto-dismiss ----

    fn settled_pair(s: &mut JobsState, now: Instant) {
        running(s, 1, now);
        running(s, 2, now);
        running(s, 3, now);
        s.apply(&JobEvent::Finished { id: id(1) }, now);
        s.apply(&JobEvent::Cancelled { id: id(2) }, now);
    }

    #[test]
    fn auto_dismiss_after_one_minute_removes_settled_rows_only() {
        let now = t0();
        let mut s = JobsState::new();
        settled_pair(&mut s, now);
        s.tick(now + Duration::from_secs(59), AutoDismiss::OneMinute);
        assert_eq!(s.rows().len(), 3, "not yet due");
        s.tick(now + Duration::from_secs(60), AutoDismiss::OneMinute);
        let ids: Vec<u64> = s.rows().iter().map(|r| r.id.0).collect();
        assert_eq!(ids, vec![3], "the running row is never auto-dismissed");
    }

    #[test]
    fn auto_dismiss_never_keeps_rows() {
        let now = t0();
        let mut s = JobsState::new();
        settled_pair(&mut s, now);
        let next = s.tick(now + Duration::from_secs(86_400), AutoDismiss::Never);
        assert_eq!(s.rows().len(), 3);
        assert_eq!(next, None);
    }

    #[test]
    fn changing_auto_dismiss_applies_to_already_settled_rows() {
        let now = t0();
        let mut s = JobsState::new();
        settled_pair(&mut s, now);
        let later = now + Duration::from_secs(400);
        s.tick(later, AutoDismiss::Never);
        assert_eq!(s.rows().len(), 3);
        s.tick(later, AutoDismiss::FiveMinutes);
        let ids: Vec<u64> = s.rows().iter().map(|r| r.id.0).collect();
        assert_eq!(ids, vec![3], "settled 400 s ago, limit is now 300 s");
    }

    #[test]
    fn tick_returns_next_deadline_only_when_settled_rows_wait() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        assert_eq!(s.tick(now, AutoDismiss::OneMinute), None, "nothing settled");
        s.apply(
            &JobEvent::Finished { id: id(1) },
            now + Duration::from_secs(10),
        );
        running(&mut s, 2, now);
        s.apply(
            &JobEvent::Cancelled { id: id(2) },
            now + Duration::from_secs(30),
        );
        let next = s.tick(now + Duration::from_secs(31), AutoDismiss::OneMinute);
        assert_eq!(next, Some(now + Duration::from_secs(70)), "earliest first");
        assert_eq!(
            s.tick(now + Duration::from_secs(31), AutoDismiss::Never),
            None
        );
        s.tick(now + Duration::from_secs(95), AutoDismiss::OneMinute);
        assert!(s.rows().is_empty());
        assert_eq!(
            s.tick(now + Duration::from_secs(95), AutoDismiss::OneMinute),
            None
        );
    }

    #[test]
    fn download_speed_uses_the_last_five_seconds() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&progress(1, 10_000_000, 100_000_000), now);
        assert_eq!(row(&s, 1).status_text(), "Downloading 10%");
        s.apply(
            &progress(1, 20_000_000, 100_000_000),
            now + Duration::from_secs(1),
        );
        assert_eq!(row(&s, 1).status_text(), "Downloading 20% · 10.0 MB/s");
        // A gap longer than the window drops the old samples instead of
        // carrying a stale rate forward.
        s.apply(
            &progress(1, 30_000_000, 100_000_000),
            now + Duration::from_secs(7),
        );
        assert_eq!(row(&s, 1).status_text(), "Downloading 30%");
        s.apply(
            &progress(1, 40_000_000, 100_000_000),
            now + Duration::from_secs(8),
        );
        assert_eq!(row(&s, 1).status_text(), "Downloading 40% · 10.0 MB/s");
    }

    #[test]
    fn pause_and_resume_clear_the_speed() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&progress(1, 10_000_000, 100_000_000), now);
        s.apply(
            &progress(1, 20_000_000, 100_000_000),
            now + Duration::from_secs(1),
        );
        assert!(row(&s, 1).status_text().contains("MB/s"));
        s.apply(
            &JobEvent::Paused { id: id(1) },
            now + Duration::from_secs(2),
        );
        assert_eq!(row(&s, 1).status_text(), "Paused 20%");
        s.apply(
            &JobEvent::Resumed { id: id(1) },
            now + Duration::from_secs(3),
        );
        assert_eq!(row(&s, 1).status_text(), "Downloading 20%");
        s.apply(
            &progress(1, 30_000_000, 100_000_000),
            now + Duration::from_secs(3),
        );
        assert_eq!(row(&s, 1).status_text(), "Downloading 30%");
        s.apply(
            &progress(1, 40_000_000, 100_000_000),
            now + Duration::from_secs(4),
        );
        assert_eq!(row(&s, 1).status_text(), "Downloading 40% · 10.0 MB/s");
    }

    #[test]
    fn speed_is_only_shown_while_downloading() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&progress(1, 10_000_000, 100_000_000), now);
        s.apply(
            &progress(1, 20_000_000, 100_000_000),
            now + Duration::from_secs(1),
        );
        s.apply(&stage(1, Stage::Verifying), now + Duration::from_secs(2));
        assert_eq!(row(&s, 1).status_text(), "Verifying");
        s.apply(&stage(1, Stage::Downloading), now + Duration::from_secs(3));
        assert_eq!(row(&s, 1).status_text(), "Downloading 20%");
    }

    #[test]
    fn rate_is_formatted_with_readable_units() {
        assert_eq!(format_rate(0.0), "0 B/s");
        assert_eq!(format_rate(999.0), "999 B/s");
        assert_eq!(format_rate(1_500.0), "1.5 KB/s");
        assert_eq!(format_rate(850_000.0), "850.0 KB/s");
        assert_eq!(format_rate(12_400_000.0), "12.4 MB/s");
        assert_eq!(format_rate(1_234_000_000.0), "1.23 GB/s");
    }

    #[test]
    fn status_text_covers_every_state() {
        let now = t0();
        let mut s = JobsState::new();
        running(&mut s, 1, now);
        s.apply(&progress(1, 420, 1000), now);
        assert_eq!(row(&s, 1).status_text(), "Downloading 42%");
        s.apply(&JobEvent::Paused { id: id(1) }, now);
        assert_eq!(row(&s, 1).status_text(), "Paused 42%");
        s.apply(&JobEvent::Resumed { id: id(1) }, now);
        for (st, want) in [
            (Stage::Verifying, "Verifying"),
            (Stage::Uploading, "Uploading"),
            (Stage::Creating, "Creating"),
            (Stage::CleaningUp, "Cleaning up"),
        ] {
            s.apply(&stage(1, st), now);
            assert_eq!(row(&s, 1).status_text(), want);
        }
        s.apply(&JobEvent::Finished { id: id(1) }, now);
        assert_eq!(row(&s, 1).status_text(), "Ready in Ollama");
        s.apply(&stage(2, Stage::Queued), now);
        assert_eq!(row(&s, 2).status_text(), "Queued");
        s.apply(
            &JobEvent::Failed {
                id: id(2),
                error: "x".into(),
            },
            now,
        );
        assert_eq!(row(&s, 2).status_text(), "Failed");
        s.apply(&stage(3, Stage::Queued), now);
        s.apply(&JobEvent::Cancelled { id: id(3) }, now);
        assert_eq!(row(&s, 3).status_text(), "Cancelled");
    }
}
