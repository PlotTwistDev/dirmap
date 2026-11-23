//! The database: D1 in the Worker, SQLite in the tests. Both speak the same SQL.

use filescape_licence::Ts;
use filescape_licence::model::{Activation, Licence, Validity};
use serde_json::Value;

/// A bound parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum V {
    Null,
    Int(i64),
    Text(String),
}

impl From<i64> for V {
    fn from(v: i64) -> V {
        V::Int(v)
    }
}
impl From<u32> for V {
    fn from(v: u32) -> V {
        V::Int(v.into())
    }
}
impl From<bool> for V {
    fn from(v: bool) -> V {
        V::Int(v.into())
    }
}
impl From<&str> for V {
    fn from(v: &str) -> V {
        V::Text(v.into())
    }
}
impl From<String> for V {
    fn from(v: String) -> V {
        V::Text(v)
    }
}
impl From<&String> for V {
    fn from(v: &String) -> V {
        V::Text(v.clone())
    }
}
impl<T: Into<V>> From<Option<T>> for V {
    fn from(v: Option<T>) -> V {
        v.map_or(V::Null, Into::into)
    }
}

/// One statement for [`Db::batch`].
pub type Stmt = (&'static str, Vec<V>);

/// A result row, by column name.
pub type Row = serde_json::Map<String, Value>;

pub trait Db {
    async fn all(&self, sql: &str, args: Vec<V>) -> Result<Vec<Row>, String>;
    /// Runs the statements in one transaction (all or nothing) and returns how many rows each
    /// changed.
    async fn batch(&self, stmts: Vec<Stmt>) -> Result<Vec<usize>, String>;

    async fn first(&self, sql: &str, args: Vec<V>) -> Result<Option<Row>, String> {
        Ok(self.all(sql, args).await?.into_iter().next())
    }

    async fn run(&self, sql: &'static str, args: Vec<V>) -> Result<usize, String> {
        Ok(self.batch(vec![(sql, args)]).await?.first().copied().unwrap_or(0))
    }
}

/// Column readers. D1 hands numbers back as JSON numbers (sometimes floats).
pub fn int(r: &Row, k: &str) -> Option<i64> {
    match r.get(k)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn text(r: &Row, k: &str) -> Option<String> {
    match r.get(k)? {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        v => Some(v.to_string()),
    }
}

pub fn req_text(r: &Row, k: &str) -> Result<String, String> {
    text(r, k).ok_or_else(|| format!("missing column {k}"))
}

pub const LICENCE_COLS: &str = "id, serial_hash, serial_hint, serial_enc, kind, validity, owner, email, purpose, issued_at, first_activated_at, expires_at, max_activations, activations_used, revoked_at, revoke_reason, order_ref, refund_flag, emailed_at";
pub const ACTIVATION_COLS: &str = "activation_id, licence_id, device_hash, app_version, created_at, counted, ended_at, voided_at, void_reason, last_seen_at";

/// A licence row, with what the core's [`Licence`] doesn't hold.
#[derive(Debug, Clone)]
pub struct LicenceRow {
    pub lic: Licence,
    pub serial_enc: Option<String>,
    pub refund_flag: Option<String>,
    pub emailed_at: Option<Ts>,
}

pub fn licence(r: &Row) -> Result<LicenceRow, String> {
    let validity: Validity = serde_json::from_str(&req_text(r, "validity")?).map_err(|e| e.to_string())?;
    Ok(LicenceRow {
        lic: Licence {
            id: req_text(r, "id")?,
            serial_hash: req_text(r, "serial_hash")?,
            serial_hint: req_text(r, "serial_hint")?,
            validity,
            owner: text(r, "owner"),
            email: text(r, "email"),
            purpose: text(r, "purpose"),
            issued_at: int(r, "issued_at").unwrap_or(0),
            first_activated_at: int(r, "first_activated_at"),
            expires_at: int(r, "expires_at"),
            max_activations: int(r, "max_activations").unwrap_or(0) as u32,
            activations_used: int(r, "activations_used").unwrap_or(0) as u32,
            revoked_at: int(r, "revoked_at"),
            revoke_reason: text(r, "revoke_reason"),
            order_ref: text(r, "order_ref"),
            events: Vec::new(),
        },
        serial_enc: text(r, "serial_enc"),
        refund_flag: text(r, "refund_flag"),
        emailed_at: int(r, "emailed_at"),
    })
}

/// An activation row, with when the app last checked in.
#[derive(Debug, Clone)]
pub struct ActivationRow {
    pub a: Activation,
    pub last_seen_at: Option<Ts>,
}

pub fn activation(r: &Row) -> Result<ActivationRow, String> {
    let voided = int(r, "voided_at").map(|t| (t, text(r, "void_reason").unwrap_or_default()));
    Ok(ActivationRow {
        a: Activation {
            activation_id: req_text(r, "activation_id")?,
            licence_id: req_text(r, "licence_id")?,
            device_hash: req_text(r, "device_hash")?,
            app_version: text(r, "app_version").unwrap_or_default(),
            created_at: int(r, "created_at").unwrap_or(0),
            counted: int(r, "counted").unwrap_or(1) != 0,
            ended_at: int(r, "ended_at"),
            voided,
        },
        last_seen_at: int(r, "last_seen_at"),
    })
}
