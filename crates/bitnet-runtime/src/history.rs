use bitnet_core::{now_secs, AppError, ErrorCode, HistoryEvent, Statistics};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct History {
    db: Connection,
}
impl History {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        let db = Connection::open(path).map_err(db_error)?;
        db.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(db_error)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA journal_size_limit=1048576; CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY, at INTEGER NOT NULL, kind TEXT NOT NULL, message TEXT NOT NULL, account TEXT, duration_ms INTEGER); CREATE INDEX IF NOT EXISTS events_at ON events(at);").map_err(db_error)?;
        let has_details = {
            let mut query = db.prepare("PRAGMA table_info(events)").map_err(db_error)?;
            let names = query
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(db_error)?;
            names
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?
                .iter()
                .any(|name| name == "details")
        };
        if !has_details {
            db.execute_batch("ALTER TABLE events ADD COLUMN details TEXT;")
                .map_err(db_error)?;
        }
        Ok(Self { db })
    }
    pub fn record(
        &self,
        kind: &str,
        message: &str,
        account: Option<&str>,
        duration_ms: Option<u64>,
    ) -> Result<(), AppError> {
        self.db
            .execute(
                "INSERT INTO events(at,kind,message,account,duration_ms) VALUES(?1,?2,?3,?4,?5)",
                params![now_secs(), kind, message, account, duration_ms],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn record_details(
        &self,
        kind: &str,
        message: &str,
        account: Option<&str>,
        duration_ms: Option<u64>,
        details: &serde_json::Value,
    ) -> Result<(), AppError> {
        self.db.execute("INSERT INTO events(at,kind,message,account,duration_ms,details) VALUES(?1,?2,?3,?4,?5,?6)",params![now_secs(),kind,message,account,duration_ms,details.to_string()]).map_err(db_error)?;
        Ok(())
    }
    pub fn prune(&self, days: u32) -> Result<(), AppError> {
        let before = now_secs().saturating_sub(u64::from(days) * 86400);
        self.db.execute("DELETE FROM events WHERE at<?1 OR id < (SELECT COALESCE(MAX(id),0)-20000 FROM events)",[before]).map_err(db_error)?;
        Ok(())
    }
    pub fn recent(&self, limit: u32) -> Result<Vec<HistoryEvent>, AppError> {
        let mut statement=self.db.prepare("SELECT id,at,kind,message,account,duration_ms,details FROM events ORDER BY id DESC LIMIT ?1").map_err(db_error)?;
        let rows = statement
            .query_map([limit.clamp(1, 1000)], |r| {
                Ok(HistoryEvent {
                    id: r.get(0)?,
                    at: r.get(1)?,
                    kind: r.get(2)?,
                    message: r.get(3)?,
                    account: r.get(4)?,
                    duration_ms: r.get(5)?,
                    details: r
                        .get::<_, Option<String>>(6)?
                        .and_then(|text| serde_json::from_str(&text).ok()),
                })
            })
            .map_err(db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
    }
    pub fn statistics(&self) -> Result<Statistics, AppError> {
        self.db.query_row("SELECT COUNT(*), COALESCE(SUM(kind='login_success'),0), COALESCE(SUM(kind='login_failure'),0), COALESCE(SUM(kind='reconnect'),0), MAX(CASE WHEN kind='login_success' THEN at END), AVG(CASE WHEN kind='reconnect' THEN duration_ms END),COALESCE(SUM(CASE WHEN kind='online_segment' THEN duration_ms ELSE 0 END),0),COALESCE(MAX(CASE WHEN kind='online_segment' THEN duration_ms ELSE 0 END),0),COALESCE(SUM(kind='session_lost'),0),COALESCE(SUM(kind='health_degraded'),0) FROM events",[],|r|Ok(Statistics{ events:r.get(0)?,login_successes:r.get(1)?,login_failures:r.get(2)?,reconnects:r.get(3)?,last_success_at:r.get(4)?,average_recovery_ms:r.get::<_,Option<f64>>(5)?.map(|n|n as u64),observed_online_seconds:r.get::<_,u64>(6)?/1000,longest_observed_online_seconds:r.get::<_,u64>(7)?/1000,session_interruptions:r.get(8)?,health_anomalies:r.get(9)?})).map_err(db_error)
    }
}
fn db_error(_: rusqlite::Error) -> AppError {
    AppError::new(ErrorCode::Unavailable, "历史数据库不可用，原数据已保留")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_only_recorded_outcomes() {
        let history = History::open(Path::new(":memory:")).unwrap();
        history
            .record("login_success", "已认证", None, None)
            .unwrap();
        history
            .record("login_failure", "密码错误", None, None)
            .unwrap();
        let stats = history.statistics().unwrap();
        assert_eq!(stats.login_successes, 1);
        assert_eq!(stats.login_failures, 1);
        assert_eq!(history.recent(5).unwrap().len(), 2);
    }
}
