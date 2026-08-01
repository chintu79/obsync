use crate::index::state::FileState;

/// How a path with different content on both sides should be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideOutcome {
    /// Both sides edited within the window — leave untouched, surface to the user.
    Conflict,
    /// Local side is (clearly) newer — push local.
    LocalWins,
    /// Remote side is (clearly) newer — pull remote.
    RemoteWins,
}

/// Revisions are per-engine local counters incremented on any edit, so a file
/// that was ever edited on both devices has `revision > 0` on both sides forever.
/// A hash difference alone therefore does NOT prove a genuine conflict — one
/// side's content may simply be a later edit of the other's.
///
/// The authoritative signal is [`FileState::synced_hash`]: the content hash the
/// last sync agreed on. If one side still has exactly that content, it never
/// changed since the agreement, so the other side's version is simply newer
/// (pull/push, never a conflict). A conflict only exists when BOTH sides
/// changed since the agreement. When agreement info is missing (pre-migration
/// rows), fall back to comparing modification times — the newer side wins.
pub fn resolve_divergence(local: &FileState, remote: &FileState) -> SideOutcome {
    if local.content_hash == remote.content_hash {
        return SideOutcome::RemoteWins; // caller should treat as no-op
    }
    let local_unchanged = local.synced_hash == Some(local.content_hash);
    let remote_unchanged = remote.synced_hash == Some(remote.content_hash);
    match (local_unchanged, remote_unchanged) {
        (true, _) => SideOutcome::RemoteWins, // local never changed since agreement → take remote
        (_, true) => SideOutcome::LocalWins, // remote never changed since agreement → push local
        (false, false) => {
            if local.synced_hash.is_some() && remote.synced_hash.is_some() {
                SideOutcome::Conflict
            } else if local.modified_at >= remote.modified_at {
                SideOutcome::LocalWins
            } else {
                SideOutcome::RemoteWins
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::index::state::SyncState;

    fn make_state(path: &str, hash_byte: u8, rev: u64, synced: Option<u8>) -> FileState {
        let mut hash = [0u8; 32];
        hash[0] = hash_byte;
        let mut synced_hash = None;
        if let Some(b) = synced {
            let mut sh = [0u8; 32];
            sh[0] = b;
            synced_hash = Some(sh);
        }
        FileState {
            relative_path: PathBuf::from(path),
            content_hash: hash,
            size: 100,
            modified_at: rev as i64,
            revision: rev,
            sync_state: SyncState::Synced,
            synced_hash,
        }
    }

    #[test]
    fn test_resolve_sequential_edits_push() {
        // Agreed on H1. Local then edited to H2 (synced still H1); remote
        // still holds the agreed H1 → local's edit is a plain update, push it.
        let local = make_state("a.md", 2, 3, Some(1));
        let remote = make_state("a.md", 1, 2, Some(1));
        assert_eq!(resolve_divergence(&local, &remote), SideOutcome::LocalWins);
    }

    #[test]
    fn test_resolve_sequential_edits_pull() {
        // Agreed on H1. Remote then edited to H2; local still holds H1.
        let local = make_state("a.md", 1, 2, Some(1));
        let remote = make_state("a.md", 2, 3, Some(1));
        assert_eq!(resolve_divergence(&local, &remote), SideOutcome::RemoteWins);
    }

    #[test]
    fn test_resolve_genuine_conflict() {
        // Agreed on H0. Both sides then edited to different content.
        let local = make_state("a.md", 1, 3, Some(0));
        let remote = make_state("a.md", 2, 2, Some(0));
        assert_eq!(resolve_divergence(&local, &remote), SideOutcome::Conflict);
    }

    #[test]
    fn test_resolve_same_hash_is_noop() {
        let local = make_state("a.md", 1, 3, Some(1));
        let remote = make_state("a.md", 1, 2, Some(0));
        assert_eq!(resolve_divergence(&local, &remote), SideOutcome::RemoteWins);
    }

    #[test]
    fn test_resolve_no_agreement_falls_back_to_mtime() {
        // Pre-migration rows have no synced_hash → newer mtime wins.
        let mut local = make_state("a.md", 1, 3, None);
        let mut remote = make_state("a.md", 2, 1, None);
        local.modified_at = 23 * 3600 * 1000;
        remote.modified_at = 21 * 3600 * 1000;
        assert_eq!(resolve_divergence(&local, &remote), SideOutcome::LocalWins);
        assert_eq!(resolve_divergence(&remote, &local), SideOutcome::RemoteWins);
    }
}
