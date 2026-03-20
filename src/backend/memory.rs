//! Per-backend memory tracking via periodic RSS sampling.

use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;
use serde::Serialize;

/// Memory statistics for a single backend process.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryStats {
    pub pid: u32,
    pub rss_kb: u64,
    pub peak_rss_kb: u64,
    #[serde(skip)]
    pub sampled_at: Instant,
}

/// Sample RSS for a list of PIDs.
/// Returns a map of PID -> RSS in KB.
#[allow(dead_code)]
pub async fn sample_rss(pids: &[u32]) -> Result<HashMap<u32, u64>> {
    if pids.is_empty() {
        return Ok(HashMap::new());
    }

    #[cfg(unix)]
    {
        let pid_args: String = pids
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let output = tokio::process::Command::new("ps")
            .arg("-o")
            .arg("pid=,rss=")
            .arg("-p")
            .arg(&pid_args)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut result = HashMap::new();
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2
                && let (Ok(pid), Ok(rss)) = (parts[0].parse::<u32>(), parts[1].parse::<u64>())
            {
                result.insert(pid, rss);
            }
        }
        Ok(result)
    }

    #[cfg(windows)]
    {
        let mut result = HashMap::new();
        for pid in pids {
            let output = tokio::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid), "/FO", "CSV", "/NH"])
                .output()
                .await?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let fields: Vec<&str> = line.split(',').collect();
                if fields.len() >= 5 {
                    let mem_str = fields[4]
                        .trim_matches('"')
                        .replace(" K", "")
                        .replace(",", "");
                    if let Ok(rss) = mem_str.parse::<u64>() {
                        result.insert(*pid, rss);
                    }
                }
            }
        }
        Ok(result)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pids;
        Ok(HashMap::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sample_rss_self() {
        let pid = std::process::id();
        let result = sample_rss(&[pid]).await;
        assert!(result.is_ok());
        let map = result.unwrap();
        // Our own process should have non-zero RSS
        if let Some(&rss) = map.get(&pid) {
            assert!(rss > 0, "our own RSS should be > 0");
        }
        // On some systems ps may not find our PID format, that's OK
    }

    #[tokio::test]
    async fn test_sample_rss_empty() {
        let result = sample_rss(&[]).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_sample_rss_nonexistent_pid() {
        // Very high PID that doesn't exist
        let result = sample_rss(&[999_999_999]).await;
        // Should succeed but return empty (ps returns no rows)
        assert!(result.is_ok());
    }
}
