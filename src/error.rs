//! Erreurs typées : chaque échec porte un type et, quand c'est utile, un conseil à l'agent
//! (et surtout ce qu'il ne faut pas réessayer).

use serde_json::{json, Map, Value};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorType {
    WriteDisabled,
    DeleteDisabled,
    TypeNotAllowed,
    ServiceNotAllowed,
    ResourceNotAllowed,
    NotConfigured,
    TestModeEgressBlocked,
    InvalidInput,
    AuthFailed,
    Forbidden,
    NotFound,
    BadRequest,
    ValidationFailed,
    Conflict,
    UpstreamError,
    BadResponse,
    TlsError,
    Unreachable,
    Timeout,
    ConfigError,
    UserCancelled,
    ConfirmationRequired,
    InternalError,
}

impl ErrorType {
    pub fn as_str(self) -> &'static str {
        use ErrorType::*;
        match self {
            WriteDisabled => "write_disabled",
            DeleteDisabled => "delete_disabled",
            TypeNotAllowed => "type_not_allowed",
            ServiceNotAllowed => "service_not_allowed",
            ResourceNotAllowed => "resource_not_allowed",
            NotConfigured => "not_configured",
            TestModeEgressBlocked => "TEST_MODE_EGRESS_BLOCKED",
            InvalidInput => "invalid_input",
            AuthFailed => "auth_failed",
            Forbidden => "forbidden",
            NotFound => "not_found",
            BadRequest => "bad_request",
            ValidationFailed => "validation_failed",
            Conflict => "conflict",
            UpstreamError => "upstream_error",
            BadResponse => "bad_response",
            TlsError => "tls_error",
            Unreachable => "unreachable",
            Timeout => "timeout",
            ConfigError => "config_error",
            UserCancelled => "user_cancelled",
            ConfirmationRequired => "confirmation_required",
            InternalError => "internal_error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ArrError {
    pub kind: ErrorType,
    pub message: String,
    pub hint: Option<String>,
    pub status: Option<u16>,
    pub details: Option<Value>,
}

pub type Result<T> = std::result::Result<T, ArrError>;

impl ArrError {
    pub fn new(kind: ErrorType, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: None,
            status: None,
            details: None,
        }
    }
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    pub fn status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }
    pub fn details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
    pub fn is(&self, kind: ErrorType) -> bool {
        self.kind == kind
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("error".into(), json!(self.message));
        m.insert("error_type".into(), json!(self.kind.as_str()));
        if let Some(s) = self.status {
            m.insert("http_status".into(), json!(s));
        }
        if let Some(h) = self.hint.as_ref().filter(|h| !h.is_empty()) {
            m.insert("hint".into(), json!(h));
        }
        if let Some(d) = &self.details {
            m.insert("details".into(), d.clone());
        }
        Value::Object(m)
    }
}

impl fmt::Display for ArrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ArrError {}

/// Raccourci : `err!(InvalidInput, "...")`.
#[macro_export]
macro_rules! err {
    ($kind:ident, $($arg:tt)*) => {
        $crate::error::ArrError::new($crate::error::ErrorType::$kind, format!($($arg)*))
    };
}
