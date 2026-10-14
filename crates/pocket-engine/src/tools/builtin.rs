//! ENG-07's built-in tools: calculator, datetime, and a registry callers
//! plug a knowledge-base search into (ENG-11's `search_knowledge_base`
//! isn't implemented yet, so it's injected rather than hard-coded here).
use std::collections::HashMap;

pub type ToolHandler = Box<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

#[derive(Default)]
pub struct ToolRegistry {
    handlers: HashMap<String, ToolHandler>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: impl Into<String>, handler: impl Fn(&str) -> Result<String, String> + Send + Sync + 'static) {
        self.handlers.insert(name.into(), Box::new(handler));
    }

    pub fn call(&self, name: &str, arguments_json: &str) -> Result<String, String> {
        match self.handlers.get(name) {
            Some(handler) => handler(arguments_json),
            None => Err(format!("unknown tool: {name}")),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    /// `calculator` and `datetime` (clocked by `now_ms`, so tests control
    /// the result instead of racing the real wall clock).
    pub fn with_builtins(now_ms: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        let mut registry = Self::new();
        registry.register("calculator", calculator);
        registry.register("datetime", move |args| datetime(args, now_ms()));
        registry
    }
}

pub fn calculator(arguments_json: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(arguments_json).map_err(|e| e.to_string())?;
    let expression = value.get("expression").and_then(|e| e.as_str()).ok_or("missing 'expression' argument")?;
    meval::eval_str(expression).map(|r| r.to_string()).map_err(|e| e.to_string())
}

fn datetime(_arguments_json: &str, now_ms: i64) -> Result<String, String> {
    Ok(iso8601_utc(now_ms))
}

/// Howard Hinnant's `civil_from_days` - no calendar dependency needed for
/// one UTC timestamp format.
fn iso8601_utc(epoch_ms: i64) -> String {
    let total_secs = epoch_ms.div_euclid(1000);
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator_evaluates_a_simple_expression() {
        let result = calculator(r#"{"expression": "2 + 2 * 3"}"#).unwrap();
        assert_eq!(result, "8");
    }

    #[test]
    fn calculator_errors_on_missing_expression() {
        assert!(calculator("{}").is_err());
    }

    #[test]
    fn calculator_errors_on_invalid_expression() {
        assert!(calculator(r#"{"expression": "not math"}"#).is_err());
    }

    #[test]
    fn iso8601_formats_the_unix_epoch() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn iso8601_formats_a_known_fixed_timestamp() {
        // 2024-01-15T12:30:45Z
        assert_eq!(iso8601_utc(1_705_321_845_000), "2024-01-15T12:30:45Z");
    }

    #[test]
    fn registry_dispatches_to_registered_handlers_and_errors_on_unknown_names() {
        let mut registry = ToolRegistry::new();
        registry.register("echo", |args| Ok(args.to_string()));
        assert_eq!(registry.call("echo", "hi").unwrap(), "hi");
        assert!(registry.call("missing", "{}").is_err());
    }

    #[test]
    fn with_builtins_wires_a_fixed_clock_into_datetime() {
        let registry = ToolRegistry::with_builtins(|| 0);
        assert_eq!(registry.call("datetime", "{}").unwrap(), "1970-01-01T00:00:00Z");
        assert_eq!(registry.call("calculator", r#"{"expression": "10/2"}"#).unwrap(), "5");
    }
}
