//! The ledger on Neon Postgres, over its SQL-over-HTTPS endpoint — the same
//! one `@neondatabase/serverless` speaks: one POST per statement, JSON in,
//! JSON out, no driver, no connection to keep alive.
//!
//! Two tables. `ledger` is append-only and its serial id is the order.
//! `snapshot` holds one row and only ever moves forward.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gemini::{arr, Value};
use world::ledger::Entry;

use crate::Ledger;

pub struct Neon {
    endpoint: String,
    conn: String,
    agent: ureq::Agent,
    ready: AtomicBool,
}

impl Neon {
    /// From `DATABASE_URL` (Vercel's Neon integration sets it).
    pub fn from_env() -> Option<Neon> {
        let conn = std::env::var("DATABASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        Neon::new(conn.trim().to_string())
    }

    pub fn new(conn: String) -> Option<Neon> {
        // postgres://user:pass@host/db?sslmode=require → https://host/sql
        let after_at = conn.split_once('@')?.1;
        let host = after_at.split(['/', '?']).next()?;
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Some(Neon {
            endpoint: format!("https://{host}/sql"),
            conn,
            agent: config.into(),
            ready: AtomicBool::new(false),
        })
    }

    /// Run one statement; rows come back as objects of text.
    fn sql(&self, query: &str, params: Vec<Value>) -> Result<Vec<Value>, String> {
        let body = gemini::obj! {"query" => query, "params" => params}.to_string();
        let mut resp = self
            .agent
            .post(&self.endpoint)
            .header("Neon-Connection-String", &self.conn)
            .header("Neon-Raw-Text-Output", "true")
            .header("Neon-Array-Mode", "false")
            .header("Content-Type", "application/json")
            .send(body.as_str())
            .map_err(|e| format!("neon: {e}"))?;
        let code = resp.status().as_u16();
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("neon: {e}"))?;
        let v = Value::parse(&text)
            .map_err(|e| format!("neon: {e}: {}", text.chars().take(200).collect::<String>()))?;
        if code >= 400 {
            return Err(format!(
                "neon {code}: {}",
                v.get("message").as_str().unwrap_or(&text)
            ));
        }
        Ok(v.get("rows").as_arr().to_vec())
    }

    fn ensure_tables(&self) -> Result<(), String> {
        if self.ready.load(Ordering::Relaxed) {
            return Ok(());
        }
        self.sql("create table if not exists ledger (id bigserial primary key, at_ms bigint not null, entry text not null)", vec![])?;
        self.sql("create table if not exists snapshot (k int primary key, last_id bigint not null, realm text not null)", vec![])?;
        self.ready.store(true, Ordering::Relaxed);
        Ok(())
    }
}

fn u64_of(v: &Value) -> Result<u64, String> {
    match v {
        Value::Num(n) => Ok(*n as u64),
        Value::Str(s) => s.parse().map_err(|_| format!("neon: not a number: {s}")),
        other => Err(format!("neon: not a number: {other}")),
    }
}

impl Ledger for Neon {
    fn load(&self) -> Result<(Option<(u64, Value)>, Vec<(u64, Entry)>), String> {
        self.ensure_tables()?;
        let snap = self.sql("select last_id, realm from snapshot where k = 1", vec![])?;
        let snap = match snap.first() {
            Some(row) => Some((
                u64_of(row.get("last_id"))?,
                Value::parse(row.get("realm").as_str().unwrap_or("{}"))
                    .map_err(|e| e.to_string())?,
            )),
            None => None,
        };
        let from = snap.as_ref().map(|(id, _)| *id).unwrap_or(0);
        let rows = self.sql(
            "select id, entry from ledger where id > $1 order by id",
            vec![Value::from(from)],
        )?;
        let mut tail = Vec::with_capacity(rows.len());
        for row in rows {
            let id = u64_of(row.get("id"))?;
            let entry = Entry::from_json(
                &Value::parse(row.get("entry").as_str().unwrap_or("{}"))
                    .map_err(|e| e.to_string())?,
            )?;
            tail.push((id, entry));
        }
        Ok((snap, tail))
    }

    fn append(&self, e: &Entry) -> Result<u64, String> {
        self.ensure_tables()?;
        let rows = self.sql(
            "insert into ledger (at_ms, entry) values ($1, $2) returning id",
            vec![Value::from(e.at_ms), Value::from(e.to_json().to_string())],
        )?;
        u64_of(
            rows.first()
                .ok_or("neon: insert returned nothing")?
                .get("id"),
        )
    }

    fn snapshot(&self, last_id: u64, realm: &Value) -> Result<(), String> {
        self.ensure_tables()?;
        self.sql(
            "insert into snapshot (k, last_id, realm) values (1, $1, $2) \
             on conflict (k) do update set last_id = excluded.last_id, realm = excluded.realm \
             where snapshot.last_id < excluded.last_id",
            vec![Value::from(last_id), Value::from(realm.to_string())],
        )?;
        Ok(())
    }

    fn all(&self) -> Result<Vec<Entry>, String> {
        self.ensure_tables()?;
        let rows = self.sql("select entry from ledger order by id", vec![])?;
        rows.iter()
            .map(|row| {
                Entry::from_json(
                    &Value::parse(row.get("entry").as_str().unwrap_or("{}"))
                        .map_err(|e| e.to_string())?,
                )
            })
            .collect()
    }
}

#[allow(dead_code)]
fn _params_are_json_arrays() -> Value {
    arr![1, "two"]
}
