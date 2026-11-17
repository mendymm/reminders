//! In-memory [`Storage`], for tests.

use std::collections::HashMap;

use super::storage::{
    ProgressWrite, ResolutionWrite, Storage, StorageError, WriteSet, check_expect,
};
use crate::model::{Progress, Reminder, ReminderId, Resolution, ResolutionId};

#[derive(Debug, Default)]
pub struct MemoryStorage {
    reminders: HashMap<ReminderId, Reminder>,
    progress: HashMap<ReminderId, Progress>,
    resolutions: HashMap<ResolutionId, Resolution>,
}

impl Storage for MemoryStorage {
    fn reminder(&self, id: ReminderId) -> Result<Option<Reminder>, StorageError> {
        Ok(self.reminders.get(&id).cloned())
    }

    fn reminders(&self) -> Result<Vec<Reminder>, StorageError> {
        Ok(self.reminders.values().cloned().collect())
    }

    fn progress(&self, id: ReminderId) -> Result<Option<Progress>, StorageError> {
        Ok(self.progress.get(&id).cloned())
    }

    fn resolution(&self, id: ResolutionId) -> Result<Option<Resolution>, StorageError> {
        Ok(self.resolutions.get(&id).cloned())
    }

    fn resolutions(&self, id: ReminderId) -> Result<Vec<Resolution>, StorageError> {
        let mut found: Vec<Resolution> = self
            .resolutions
            .values()
            .filter(|resolution| resolution.reminder_id == id)
            .cloned()
            .collect();
        found.sort_by_key(|resolution| std::cmp::Reverse((resolution.resolved_at, resolution.id.0)));
        Ok(found)
    }

    fn commit(&mut self, writes: WriteSet) -> Result<(), StorageError> {
        if let Some((reminder, expect)) = &writes.reminder {
            check_expect(
                self.reminders.get(&reminder.id).map(|r| r.meta.revision),
                *expect,
            )?;
        }
        if let Some(ProgressWrite::Put(progress, expect)) = &writes.progress {
            check_expect(
                self.progress.get(&progress.reminder_id).map(|p| p.revision),
                *expect,
            )?;
        }
        match &writes.resolution {
            Some(ResolutionWrite::Insert(resolution))
                if self.resolutions.contains_key(&resolution.id) =>
            {
                return Err(StorageError::AlreadyExists);
            }
            Some(ResolutionWrite::Update(resolution))
                if !self.resolutions.contains_key(&resolution.id) =>
            {
                return Err(StorageError::Missing);
            }
            _ => {}
        }

        if let Some((reminder, _)) = writes.reminder {
            self.reminders.insert(reminder.id, reminder);
        }
        match writes.progress {
            Some(ProgressWrite::Put(progress, _)) => {
                self.progress.insert(progress.reminder_id, progress);
            }
            Some(ProgressWrite::Delete(id)) => {
                self.progress.remove(&id);
            }
            None => {}
        }
        if let Some(ResolutionWrite::Insert(resolution) | ResolutionWrite::Update(resolution)) =
            writes.resolution
        {
            self.resolutions.insert(resolution.id, resolution);
        }
        Ok(())
    }
}
