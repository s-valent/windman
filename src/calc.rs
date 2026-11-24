use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const RATES_URL: &str = "https://open.er-api.com/v6/latest/USD";
const MAX_AGE_SECONDS: u64 = 24 * 60 * 60;

static RATES: LazyLock<Mutex<HashMap<String, f64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static VERSION: AtomicU64 = AtomicU64::new(0);
static FETCHING: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize)]
struct Response {
    result: String,
    rates: HashMap<String, f64>,
}

#[derive(Serialize, Deserialize)]
struct Cache {
    fetched: u64,
    rates: HashMap<String, f64>,
}

pub struct Suggestion {
    pub title: String,
    pub subtitle: String,
    pub copy: String,
}

pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

pub fn ensure_rates() {
    if FETCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        let cached = load_cache();
        let fresh = cached
            .as_ref()
            .is_some_and(|cache| now().saturating_sub(cache.fetched) < MAX_AGE_SECONDS);
        if let Some(cache) = cached {
            install(cache.rates);
        }
        if !fresh && let Some(rates) = fetch_rates() {
            save_cache(&rates);
            install(rates);
        }
        FETCHING.store(false, Ordering::Relaxed);
    });
}

pub fn convert(query: &str) -> Option<Suggestion> {
    let (amount, from, to) = parse_conversion(query)?;
    let from_rate = rate(from)?;
    let to_rate = rate(to)?;
    let converted = amount * to_rate / from_rate;
    let rate = to_rate / from_rate;
    Some(Suggestion {
        title: format!(
            "{} {from} = {} {to}",
            format_value(amount),
            format_value(converted)
        ),
        subtitle: format!("1 {from} = {} {to} · copy result", format_value(rate)),
        copy: format_value(converted),
    })
}

pub fn calculate(query: &str) -> Option<Suggestion> {
    let expression = query.trim();
    if expression.len() > 80
        || !expression
            .chars()
            .any(|character| "+-*/^%()".contains(character))
        || [" in ", " to ", "->", "→"]
            .iter()
            .any(|separator| expression.to_lowercase().contains(separator))
    {
        return None;
    }
    let value = meval::eval_str(expression).ok()?;
    if !value.is_finite() {
        return None;
    }
    let copy = format_value(value);
    Some(Suggestion {
        title: format!("{expression} = {copy}"),
        subtitle: "Calculator · copy result".into(),
        copy,
    })
}

fn parse_conversion(query: &str) -> Option<(f64, &'static str, &'static str)> {
    let normalized = query.trim().to_lowercase().replace(',', "");
    let (amount_part, target_part) = [" in ", " to ", "->", "→", "=>"]
        .iter()
        .find_map(|separator| normalized.split_once(separator))?;
    let amount = extract_amount(amount_part)?;
    let from = extract_currency(amount_part)?;
    let to = extract_currency(target_part)?;
    Some((amount, from, to))
}

fn extract_amount(text: &str) -> Option<f64> {
    let start = text.find(|character: char| {
        character.is_ascii_digit() || character == '-' || character == '+'
    })?;
    let number = text[start..]
        .chars()
        .take_while(|character| {
            character.is_ascii_digit()
                || *character == '.'
                || *character == '-'
                || *character == '+'
        })
        .collect::<String>();
    number.parse().ok()
}

fn extract_currency(text: &str) -> Option<&'static str> {
    text.split(|character: char| {
        character.is_ascii_digit()
            || character == '.'
            || character == '-'
            || character == '+'
            || character.is_whitespace()
    })
    .filter(|token| !token.is_empty())
    .find_map(|token| {
        currency(
            token.trim_matches(|c: char| c.is_ascii_punctuation() && !"$€£¥₽₴₸₹₩₺".contains(c)),
        )
    })
}

fn currency(token: &str) -> Option<&'static str> {
    let token = token.to_lowercase();
    let singular = token.trim_end_matches('s');
    match singular {
        "$" | "usd" | "usdollar" | "dollar" => Some("USD"),
        "€" | "eur" | "euro" => Some("EUR"),
        "£" | "gbp" | "pound" => Some("GBP"),
        "¥" | "jpy" | "yen" => Some("JPY"),
        "₽" | "rub" | "ruble" | "rouble" => Some("RUB"),
        "₴" | "uah" | "hryvnia" => Some("UAH"),
        "₸" | "kzt" | "tenge" => Some("KZT"),
        "₹" | "inr" | "rupee" => Some("INR"),
        "₩" | "krw" | "won" => Some("KRW"),
        "₺" | "try" | "lira" => Some("TRY"),
        "cny" | "rmb" | "yuan" => Some("CNY"),
        "chf" | "franc" => Some("CHF"),
        "cad" => Some("CAD"),
        "aud" => Some("AUD"),
        "nzd" => Some("NZD"),
        "sek" => Some("SEK"),
        "nok" => Some("NOK"),
        "dkk" => Some("DKK"),
        "pln" => Some("PLN"),
        "czk" => Some("CZK"),
        "huf" => Some("HUF"),
        "ron" => Some("RON"),
        "bgn" => Some("BGN"),
        "ils" => Some("ILS"),
        "aed" => Some("AED"),
        "thb" => Some("THB"),
        "sgd" => Some("SGD"),
        "hkd" => Some("HKD"),
        "mxn" => Some("MXN"),
        "brl" => Some("BRL"),
        "zar" => Some("ZAR"),
        "btc" => Some("BTC"),
        _ => None,
    }
}
fn format_value(value: f64) -> String {
    let decimals = if value.abs() >= 0.01 { 2 } else { 6 };
    let formatted = format!("{value:.decimals$}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn rate(currency: &str) -> Option<f64> {
    let rates = RATES.lock().ok()?;
    let value = *rates.get(currency)?;
    (value > 0.).then_some(value)
}

fn install(mut rates: HashMap<String, f64>) {
    rates.insert("USD".into(), 1.);
    if let Ok(mut global) = RATES.lock() {
        *global = rates;
        VERSION.fetch_add(1, Ordering::Relaxed);
    }
}

fn fetch_rates() -> Option<HashMap<String, f64>> {
    let output = Command::new("curl")
        .args(["-fsSL", "--max-time", "10", RATES_URL])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let response: Response = serde_json::from_slice(&output.stdout).ok()?;
    (response.result == "success").then_some(response.rates)
}

fn cache_path() -> Option<PathBuf> {
    crate::data_dir().map(|directory| directory.join("rates.json"))
}

fn load_cache() -> Option<Cache> {
    serde_json::from_str(&std::fs::read_to_string(cache_path()?).ok()?).ok()
}

fn save_cache(rates: &HashMap<String, f64>) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(directory) = path.parent() {
        let _ = std::fs::create_dir_all(directory);
    }
    if let Ok(cache) = serde_json::to_string(&Cache {
        fetched: now(),
        rates: rates.clone(),
    }) {
        let _ = std::fs::write(path, cache);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{calculate, format_value, parse_conversion};

    #[test]
    fn parses_currency_queries() {
        assert_eq!(parse_conversion("10 usd in eur"), Some((10., "USD", "EUR")));
        assert_eq!(parse_conversion("10$ in rub"), Some((10., "USD", "RUB")));
        assert_eq!(parse_conversion("€25 to usd"), Some((25., "EUR", "USD")));
        assert_eq!(
            parse_conversion("10 dollars in euros"),
            Some((10., "USD", "EUR"))
        );
    }

    #[test]
    fn calculates_expressions() {
        assert_eq!(calculate("2+2*3").unwrap().copy, "8");
        assert_eq!(calculate("(10 + 5) / 3").unwrap().copy, "5");
        assert!(calculate("echo 1").is_none());
    }

    #[test]
    fn formats_values() {
        assert_eq!(format_value(14.), "14");
        assert_eq!(format_value(0.9284), "0.93");
        assert_eq!(format_value(0.001234), "0.001234");
    }
}
