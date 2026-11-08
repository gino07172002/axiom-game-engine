use crate::{Diagnostic, Request};
use anyhow::{ensure, Result};
use serde_json::Value;
use std::collections::BTreeMap;
pub fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c)),
        "request_id must be 1..128 ASCII letters, digits or -_.:"
    );
    Ok(())
}
/// Explicit session receipts are retained until the host exits; never silently evict an ID.
#[derive(Default)]
pub struct SessionReceipts {
    entries: BTreeMap<String, (String, Value)>,
    bytes: usize,
}
impl SessionReceipts {
    pub fn lookup(&self, id: &str, session: &str, request: &Request) -> Result<Option<Value>> {
        validate_id(id)?;
        let payload = serde_json::to_string(&(session, request))?;
        if let Some((old, response)) = self.entries.get(id) {
            if old != &payload {
                return Err(Diagnostic::error(
                    "idempotency_conflict",
                    "/request_id",
                    "request ID already has a different payload/session",
                ));
            }
            return Ok(Some(response.clone()));
        }
        ensure!(
            self.entries.len() < 4096 && self.bytes + payload.len() + 4_194_304 <= 67_108_864,
            "runtime receipt budget exhausted; restart with a new session"
        );
        Ok(None)
    }
    pub fn insert(
        &mut self,
        id: String,
        session: &str,
        request: &Request,
        response: Value,
    ) -> Result<()> {
        let payload = serde_json::to_string(&(session, request))?;
        let size = serde_json::to_vec(&response)?.len();
        ensure!(
            size <= 4_194_304,
            "runtime receipt response exceeds reserved budget"
        );
        self.bytes += payload.len() + size;
        self.entries.insert(id, (payload, response));
        Ok(())
    }
}
