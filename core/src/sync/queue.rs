use std::collections::VecDeque;

use crate::sync::delta::SyncOperation;

#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub id: u64,
    pub operation: SyncOperation,
    pub created_at: i64,
    pub retries: u32,
}

pub struct SyncQueue {
    entries: VecDeque<QueueEntry>,
    next_id: u64,
}

impl SyncQueue {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            next_id: 1,
        }
    }

    pub fn push(&mut self, op: SyncOperation) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let now = chrono::Utc::now().timestamp_millis();
        self.entries.push_back(QueueEntry {
            id,
            operation: op,
            created_at: now,
            retries: 0,
        });
        id
    }

    pub fn pop_front(&mut self) -> Option<QueueEntry> {
        self.entries.pop_front()
    }

    pub fn peek_front(&self) -> Option<&QueueEntry> {
        self.entries.front()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn increment_retry(&mut self, id: u64) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.retries += 1;
            entry.retries < 10
        } else {
            false
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn prioritize_small_files(&mut self) {
        let mut vec: Vec<QueueEntry> = self.entries.drain(..).collect();
        vec.sort_by_key(|e| match &e.operation {
            SyncOperation::Create { size, .. } | SyncOperation::Update { size, .. } => *size,
            _ => 0,
        });
        self.entries = vec.into();
    }
}

impl Default for SyncQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_create() -> SyncOperation {
        SyncOperation::Create {
            path: "small.md".into(),
            content_hash: [0u8; 32],
            size: 100,
            modified_at: 1,
        }
    }

    fn large_create() -> SyncOperation {
        SyncOperation::Create {
            path: "large.bin".into(),
            content_hash: [0u8; 32],
            size: 100_000_000,
            modified_at: 1,
        }
    }

    #[test]
    fn test_queue_push_pop() {
        let mut q = SyncQueue::new();
        assert!(q.is_empty());
        let id = q.push(small_create());
        assert!(!q.is_empty());
        assert_eq!(q.len(), 1);
        let entry = q.pop_front().unwrap();
        assert_eq!(entry.id, id);
        assert!(q.is_empty());
    }

    #[test]
    fn test_queue_retry_limit() {
        let mut q = SyncQueue::new();
        let id = q.push(small_create());
        for _ in 0..9 {
            assert!(q.increment_retry(id));
        }
        assert!(!q.increment_retry(id)); // 10th retry fails
    }

    #[test]
    fn test_prioritize_small_files() {
        let mut q = SyncQueue::new();
        q.push(large_create());
        q.push(small_create());
        q.prioritize_small_files();
        let first = q.pop_front().unwrap();
        match first.operation {
            SyncOperation::Create { size, .. } => assert_eq!(size, 100),
            _ => panic!("expected create"),
        }
    }

    #[test]
    fn test_queue_remove() {
        let mut q = SyncQueue::new();
        let id = q.push(small_create());
        assert_eq!(q.len(), 1);
        q.remove(id);
        assert!(q.is_empty());
    }
}
