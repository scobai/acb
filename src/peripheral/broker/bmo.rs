use std::path::Path;

use lazy_static::lazy_static;
use rust_decimal::Decimal;
use time::Date;

use super::BrokerTx;
use crate::util::symbol_alias::SymbolAliasResolver;
use crate::{
    portfolio::TxAction,
    util::{basic::SError, decimal::parse_large_decimal},
};

const BMO_ACCOUNT_BROKER_NAME: &str = "BMO InvestorLine";

pub fn new_account(account_number: String, account_type: String) -> super::Account {
    super::Account {
        broker_name: BMO_ACCOUNT_BROKER_NAME,
        account_type,
        account_num: account_number,
    }
}

struct Searcher {
    pattern: String,
    re: regex::Regex,
}

impl Searcher {
    pub fn new(pattern: &str) -> Self {
        let pat = pattern.to_string();
        let re = regex::RegexBuilder::new(pattern)
            .dot_matches_new_line(true)
            .build()
            .expect("invalid regex pattern");
        Searcher { pattern: pat, re }
    }

    pub fn captures<'t>(&self, text: &'t str) -> Option<regex::Captures<'t>> {
        self.re.captures(text)
    }

    pub fn get_from(&self, text: &str, group: usize) -> Result<String, SError> {
        match self.re.captures(text) {
            Some(m) => {
                m.get(group).map(|c| c.as_str().to_string()).ok_or_else(|| {
                    format!(
                        "Could not get group {} from pattern '{}'",
                        group, self.pattern
                    )
                })
            }
            None => Err(format!("Could not find pattern '{}'", self.pattern)),
        }
    }

    pub fn get1_from(&self, text: &str) -> Result<String, SError> {
        self.get_from(text, 1)
    }

    // Convenience alias
    pub fn str1(&self, text: &str) -> Result<String, SError> {
        self.get1_from(text)
    }

    pub fn get1_dec_from(&self, text: &str) -> Result<Decimal, SError> {
        let val_str = self.get_from(text, 1)?;
        parse_large_decimal(&val_str).map_err(|e| e.to_string())
    }

    // Convenience alias
    pub fn dec1(&self, text: &str) -> Result<Decimal, SError> {
        self.get1_dec_from(text)
    }
}

fn srch(pattern: &str) -> Searcher {
    Searcher::new(pattern)
}

lazy_static! {
    // Capture month name (full or short), day, year (case-insensitive)
    static ref RE_DATE_MONTHNAME: Searcher = srch(r"(?i)(JAN(?:UARY)?|FEB(?:RUARY)?|MAR(?:CH)?|APR(?:IL)?|MAY|JUN(?:E)?|JUL(?:Y)?|AUG(?:UST)?|SEP(?:T(?:EMBER)?)?|OCT(?:OBER)?|NOV(?:EMBER)?|DEC(?:EMBER)?)\s+(\d{1,2}),\s*(\d{2,4})");
    // Shared monetary value pattern: supports plain integers and decimals,
    // comma-grouped thousands, and leading-dot decimals. Examples:
    // "1234.56", "1,234.56", "1234", ".95", "0.95".
    static ref MONEY_VALUE: Searcher = srch(r"(?:\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?|\.\d+)");
}

fn parse_bmo_date(text: &str) -> Result<Date, SError> {
    // Normalize whitespace for month-name matching
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");

    // Try month-name style first (e.g. "Jan 7, 2026" or "January 7, 2026")
    if let Some(caps) = RE_DATE_MONTHNAME.captures(&normalized) {
        let mon_str = caps
            .get(1)
            .ok_or_else(|| "Missing month".to_string())?
            .as_str()
            .to_uppercase();
        let day_str = caps.get(2).ok_or_else(|| "Missing day".to_string())?.as_str();
        let year_str =
            caps.get(3).ok_or_else(|| "Missing year".to_string())?.as_str();

        let day: u8 = day_str
            .parse()
            .map_err(|e| format!("Invalid day '{}': {}", day_str, e))?;
        let mut year: i32 = year_str
            .parse()
            .map_err(|e| format!("Invalid year '{}': {}", year_str, e))?;
        // If year is two-digit, assume 2000s
        if year < 100 {
            year += 2000
        }

        let month = match mon_str.as_str() {
            "JAN" | "JANUARY" => time::Month::January,
            "FEB" | "FEBRUARY" => time::Month::February,
            "MAR" | "MARCH" => time::Month::March,
            "APR" | "APRIL" => time::Month::April,
            "MAY" => time::Month::May,
            "JUN" | "JUNE" => time::Month::June,
            "JUL" | "JULY" => time::Month::July,
            "AUG" | "AUGUST" => time::Month::August,
            "SEP" | "SEPT" | "SEPTEMBER" => time::Month::September,
            "OCT" | "OCTOBER" => time::Month::October,
            "NOV" | "NOVEMBER" => time::Month::November,
            "DEC" | "DECEMBER" => time::Month::December,
            other => return Err(format!("Unknown month '{}'", other)),
        };

        return Ok(Date::from_calendar_date(year, month, day).map_err(|e| {
            format!("Invalid date {}-{}-{}: {}", year, mon_str, day, e)
        })?);
    }

    Err(format!("Could not parse date: {}", normalized))
}

#[derive(Debug, Clone)]
pub struct BmoTrade {
    pub security: String,
    pub trade_date: Date,
    pub settlement_date: Date,
    pub action: TxAction,
    pub amount_per_share: Decimal,
    pub num_shares: Decimal,
    pub commission: Decimal,
    pub currency: crate::portfolio::Currency,
    pub memo: String,
    pub account_number: String,
    pub account_type: String,
    pub client_name: String,
    pub gross_amount: Decimal,
    pub order_number: String,
}

/// Parse a single BMO trade confirmation from text.
pub fn parse_bmo_trade(text: &str, _filename: &Path) -> Result<BmoTrade, SError> {
    // Extract trade date - look for "DATE" followed by a month-name date;
    // reuse the static month-name date regex.
    let month_pat = RE_DATE_MONTHNAME.pattern.trim_start_matches("(?i)");
    let date_pattern = format!(r"(?i)DATE\s+({})", month_pat);
    let trade_date_str = srch(&date_pattern).str1(text)?;
    let trade_date = parse_bmo_date(&trade_date_str)?;

    // Extract settlement date - "SETTLEMENT DATE <date>"
    let settlement_pattern = format!(r"(?i)SETTLEMENT\s+DATE\s+({})", month_pat);
    let settlement_date_str = srch(&settlement_pattern).str1(text)?;
    let settlement_date = parse_bmo_date(&settlement_date_str)?;

    // Extract transaction type (BUY or SELL)
    let tx_type_str =
        srch(r"(?i)TRANSACTION\s+TYPE\s+(\w+)").str1(text)?.to_uppercase();
    let action = match tx_type_str.as_str() {
        "SOLD" | "SELL" => TxAction::Sell,
        "BUY" | "BOUGHT" => TxAction::Buy,
        _ => return Err(format!("Unknown transaction type: {}", tx_type_str)),
    };

    // Extract quantity and security name in one regex
    // Look for the Quantity section, which has a header row followed by the actual qty/security/price
    // The actual pattern is like: "Quantity ... [headers]\n12,345 GLOBAL ... @ [price]"
    let re_qty_sec = regex::RegexBuilder::new(
        r"(?i)Quantity.+?(\d{1,3}(?:,\d{3})*(?:\.\d+)?)\s+(.+?)@",
    )
    .dot_matches_new_line(true)
    .build()
    .unwrap();

    let qty_caps = re_qty_sec
        .captures(text)
        .ok_or("Could not find Quantity section with qty and security name")?;

    let qty_str =
        qty_caps.get(1).ok_or("Could not extract quantity group")?.as_str();
    let num_shares = parse_large_decimal(qty_str)
        .map_err(|e| format!("Could not parse quantity '{}': {}", qty_str, e))?;

    // Extract security name from the same capture.
    // Note: The security name may span multiple lines in the PDF (e.g., "GLOBAL X US DLR CURRENCY ETF UNIT CL A"),
    // but we only capture the first line to extract here ("GLOBAL X US DLR CURRENCY").
    let full_qty_line =
        qty_caps.get(2).ok_or("Could not extract security name group")?.as_str();

    // Extract only the first line of the security name and normalize whitespace
    // (removes double-spaces, tabs, newlines, etc., replacing with single spaces)
    let raw_security = full_qty_line
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if raw_security.is_empty() {
        return Err("Could not extract security name".to_string());
    }

    // Try to extract SECURITY NO. code (e.g., CUSIP) from the document
    let security_code = srch(r"(?i)SECURITY\s+NO\.?\s*([A-Z0-9]+)").str1(text).ok();

    // If the security_code isn't found we'll return an error. As far as I know it is
    // always present.
    // If we can't find an alias for the code, we'll just use the raw security name as the symbol,
    // but we'll include the code in the memo for reference.
    let symbol_alias_resolver = SymbolAliasResolver::new();
    let (security, orig_symbol_note) = if let Some(code) = security_code {
        if let Some(alias) = symbol_alias_resolver.resolve(code.as_str()) {
            let memo_suffix = if let Some(aka_value) = alias.aka {
                // The AKA value is something like DLR.TO vs DLR.U.TO
                format!("; {} AKA {} - {}", code, aka_value, raw_security)
            } else {
                // NO AKA value, just include the code and raw security in the memo
                format!("; {} - {}", code, raw_security)
            };
            (alias.canonical.to_string(), Some(memo_suffix))
        } else {
            (raw_security.clone(), Some(format!("; {}", code)))
        }
    } else {
        return Err(
            "Could not extract security code (SECURITY NO.) from document"
                .to_string(),
        );
    };

    // Extract unit price from "@" section using shared money pattern
    let money_pat = &MONEY_VALUE.pattern;
    let price_pattern = format!(r"@\s*({})", money_pat);
    let price_str = srch(&price_pattern).str1(text)?;
    let amount_per_share = parse_large_decimal(&price_str)
        .map_err(|e| format!("Could not parse price: {}", e))?;

    // Extract currency: require explicit marker for CAD or USD.
    // BMO PDFs often show prices like "@ 14.0100C$" or "@ 10.1400U$".
    let currency_pattern = format!(r"@\s*{}\s*([A-Za-z\$UuCc]+)", money_pat);
    let mut currency_str = srch(&currency_pattern).str1(text)?;
    currency_str = currency_str.trim().to_uppercase();
    let currency = match currency_str.as_str() {
        "C$" | "C" | "CAD" => crate::portfolio::Currency::cad(),
        "U$" | "U" | "USD" => crate::portfolio::Currency::usd(),
        other => {
            return Err(format!(
                "Unable to determine currency from BMO PDF: '{}'",
                other
            ))
        }
    };

    // Extract commission using shared money pattern
    // NOTE: BMO does not always show commission with a leading number before the decimal.
    // If commission is not found, default to 0.00
    let commission_pat = format!(r"(?i)COMMISSION\s+({})", money_pat);
    let commission = srch(&commission_pat).dec1(text).unwrap_or_else(|_| Decimal::ZERO);

    // Extract account number - look for "ACCOUNT NO. TYPE" followed by account number
    // Note: The number and type may appear on the same line as the header or on the next line
    // The account number may use a special dash character (−) or regular hyphen (-)
    let account_number = srch(
        r"(?i)ACCOUNT\s+NO[.\s]*(?:TYPE)?[.\s]*([0-9a-zA-Z]+[−\-][0-9a-zA-Z]+)",
    )
    .str1(text)
    .unwrap_or_else(|_| String::from("UNKNOWN"))
    .replace('−', "-"); // Normalize special dash to regular hyphen

    // Extract account type - look for the type following the account number
    // Note: The type may appear on the same line as the account number or on a following line
    let account_type =
        srch(r"(?i)ACCOUNT\s+NO[.\s]*(?:TYPE)?[.\s]*[0-9a-zA-Z]+[−\-][0-9a-zA-Z]+\s+(\w+)")
            .str1(text)
            .unwrap_or_else(|_| String::from("CSH"));

    // Extract client name - try labeled format first (e.g., "CLIENT NAME MR JOHN DOE"),
    // then fall back to extracting from the top of the document (some PDFs have name on
    // line 3 without a label before the address)
    let client_name = srch(r"(?i)CLIENT\s+NAME\s+(.+?)(?:\s+ACCOUNT|$)")
        .str1(text)
        .or_else(|_| {
            // Fall back: try extracting from top of document (line 3, before address)
            // Look for pattern like "MR john doe" at the start
            srch(r"(?m)^\s*([A-Z]{2,3}\s+[a-zA-Z]+(?:\s+[a-zA-Z]+)*)\s*$")
                .str1(text)
        })?
        .trim()
        .split('\n')
        .next()
        .unwrap_or("")
        .trim()
        .to_uppercase(); // Normalize to uppercase for consistency

    // Extract gross amount using shared money pattern
    let gross_pat = format!(r"(?i)GROSS\s+AMOUNT\s+({})", money_pat);
    let gross_amount = srch(&gross_pat).dec1(text)?;

    // Extract order number (e.g., "ORDER NO. 999123")
    let order_number = srch(r"(?i)ORDER\s+NO\.\s+(\d+)")
        .str1(text)?;

    Ok(BmoTrade {
        security,
        trade_date,
        settlement_date,
        action,
        amount_per_share,
        num_shares,
        commission,
        currency,
        memo: format!("BMO Trade {}", orig_symbol_note.as_deref().unwrap_or("")),
        account_number,
        account_type,
        client_name,
        gross_amount,
        order_number,
    })
}

/// Convert a BmoTrade to a BrokerTx
pub fn bmo_trade_to_broker_tx(
    trade: &BmoTrade,
    row_num: u32,
    filename: Option<String>,
) -> BrokerTx {
    BrokerTx {
        security: trade.security.clone(),
        trade_date: trade.trade_date,
        settlement_date: trade.settlement_date,
        trade_date_and_time: trade.trade_date.to_string(),
        settlement_date_and_time: trade.settlement_date.to_string(),
        action: trade.action,
        amount_per_share: trade.amount_per_share,
        num_shares: trade.num_shares,
        commission: trade.commission,
        currency: trade.currency.clone(),
        memo: trade.memo.clone(),
        exchange_rate: None,
        affiliate: crate::portfolio::Affiliate::default(),
        row_num,
        account: new_account(
            trade.account_number.clone(),
            trade.account_type.clone(),
        ),
        sort_tiebreak: None,
        filename,
    }
}

// MARK: tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::Currency;
    use rust_decimal_macros::dec;

    fn read_sample(path: &str) -> String {
        std::fs::read_to_string(path).expect("failed to read sample file")
    }

    #[test]
    fn test_parse_buy_confirmation_lopdf_pypdf() {
        let lopdf_text = read_sample(
            "tests/data/bmo_scenarios/2026_sample/lopdf/bought_confirmation_1.txt",
        );
        let pypdf_text = read_sample(
            "tests/data/bmo_scenarios/2026_sample/pypdf/bought_confirmation_1.txt",
        );

        let lop =
            parse_bmo_trade(&lopdf_text, &std::path::PathBuf::from("lopdf.txt"))
                .unwrap();
        let py =
            parse_bmo_trade(&pypdf_text, &std::path::PathBuf::from("pypdf.txt"))
                .unwrap();

        // Both should be BUY and USD
        assert_eq!(lop.action, TxAction::Buy);
        assert_eq!(py.action, TxAction::Buy);
        assert_eq!(lop.currency, Currency::usd());
        assert_eq!(py.currency, Currency::usd());

        assert_eq!(lop.amount_per_share, dec!(10.1400));
        assert_eq!(py.amount_per_share, dec!(10.1400));

        assert_eq!(lop.num_shares, Decimal::from(12345u32));
        assert_eq!(py.num_shares, Decimal::from(12345u32));

        assert_eq!(lop.commission, dec!(9.95));
        assert_eq!(py.commission, dec!(9.95));

        // Security should use canonical symbol
        assert_eq!(lop.security, "DLR.TO");
        assert_eq!(py.security, "DLR.TO");

        let expected_trade_date =
            time::Date::from_calendar_date(2026, time::Month::January, 7).unwrap();
        let expected_settle =
            time::Date::from_calendar_date(2026, time::Month::January, 8).unwrap();
        assert_eq!(lop.trade_date, expected_trade_date);
        assert_eq!(py.trade_date, expected_trade_date);
        assert_eq!(lop.settlement_date, expected_settle);
        assert_eq!(py.settlement_date, expected_settle);

        assert_eq!(
            lop.memo,
            "BMO Trade ; G036247 AKA DLR.U.TO - GLOBAL X US DLR CURRENCY"
        );
        assert_eq!(
            py.memo,
            "BMO Trade ; G036247 AKA DLR.U.TO - GLOBAL X US DLR CURRENCY"
        );

        assert_eq!(lop.account_number, "123-XXXXX123");
        assert_eq!(py.account_number, "123-XXXXX123");
        assert_eq!(lop.account_type, "CSH");
        assert_eq!(py.account_type, "CSH");
        assert_eq!(lop.client_name, "MR JOHN DOE");
        assert_eq!(py.client_name, "MR JOHN DOE");

        let expected_gross = dec!(125178.30); // 10.1400$ * 12345 units
        assert_eq!(lop.gross_amount, expected_gross);
        assert_eq!(py.gross_amount, expected_gross);

        assert_eq!(lop.order_number, "987611");
        assert_eq!(py.order_number, "987611");
    }

    #[test]
    fn test_parse_sell_confirmation_1_lopdf_pypdf() {
        let lopdf_text = read_sample(
            "tests/data/bmo_scenarios/2026_sample/lopdf/sold_confirmation_1.txt",
        );
        let pypdf_text = read_sample(
            "tests/data/bmo_scenarios/2026_sample/pypdf/sold_confirmation_1.txt",
        );

        let lop = parse_bmo_trade(
            &lopdf_text,
            &std::path::PathBuf::from("lopdf_conf.txt"),
        )
        .unwrap();
        let py = parse_bmo_trade(
            &pypdf_text,
            &std::path::PathBuf::from("pypdf_conf.txt"),
        )
        .unwrap();

        // Both should be SELL and CAD
        assert_eq!(lop.action, TxAction::Sell);
        assert_eq!(py.action, TxAction::Sell);
        assert_eq!(lop.currency, Currency::cad());
        assert_eq!(py.currency, Currency::cad());

        assert_eq!(lop.amount_per_share, dec!(14.0100));
        assert_eq!(py.amount_per_share, dec!(14.0100));

        assert_eq!(lop.num_shares, Decimal::from(20u32));
        assert_eq!(py.num_shares, Decimal::from(20u32));

        assert_eq!(lop.commission, dec!(0.02));
        assert_eq!(py.commission, dec!(0.02));

        // Security should use canonical symbol
        assert_eq!(lop.security, "DLR.TO");
        assert_eq!(py.security, "DLR.TO");

        let expected_trade_date_s =
            time::Date::from_calendar_date(2026, time::Month::January, 7).unwrap();
        let expected_settle_s =
            time::Date::from_calendar_date(2026, time::Month::January, 8).unwrap();
        assert_eq!(lop.trade_date, expected_trade_date_s);
        assert_eq!(py.trade_date, expected_trade_date_s);
        assert_eq!(lop.settlement_date, expected_settle_s);
        assert_eq!(py.settlement_date, expected_settle_s);

        assert_eq!(
            lop.memo,
            "BMO Trade ; G036247 AKA DLR.U.TO - GLOBAL X US DLR CURRENCY"
        );
        assert_eq!(
            py.memo,
            "BMO Trade ; G036247 AKA DLR.U.TO - GLOBAL X US DLR CURRENCY"
        );

        assert_eq!(lop.account_number, "123-XXXXX123");
        assert_eq!(py.account_number, "123-XXXXX123");
        assert_eq!(lop.account_type, "CSH");
        assert_eq!(py.account_type, "CSH");
        assert_eq!(lop.client_name, "MR JOHN DOE");
        assert_eq!(py.client_name, "MR JOHN DOE");

        let expected_gross = dec!(14.0100) * Decimal::from(20u32);
        assert_eq!(lop.gross_amount, expected_gross);
        assert_eq!(py.gross_amount, expected_gross);

        assert_eq!(lop.order_number, "987612");
        assert_eq!(py.order_number, "987612");
    }

    #[test]
    fn test_parse_sell_confirmation_2_lopdf_pypdf() {
        let lopdf_text = read_sample(
            "tests/data/bmo_scenarios/2026_sample/lopdf/sold_confirmation_2.txt",
        );
        let pypdf_text = read_sample(
            "tests/data/bmo_scenarios/2026_sample/pypdf/sold_confirmation_2.txt",
        );

        let lop = parse_bmo_trade(
            &lopdf_text,
            &std::path::PathBuf::from("lopdf_conf2.txt"),
        )
        .unwrap();
        let py = parse_bmo_trade(
            &pypdf_text,
            &std::path::PathBuf::from("pypdf_conf2.txt"),
        )
        .unwrap();

        // Both should be SELL and CAD
        assert_eq!(lop.action, TxAction::Sell);
        assert_eq!(py.action, TxAction::Sell);
        assert_eq!(lop.currency, Currency::cad());
        assert_eq!(py.currency, Currency::cad());

        assert_eq!(lop.amount_per_share, dec!(14.01));
        assert_eq!(py.amount_per_share, dec!(14.01));

        assert_eq!(lop.num_shares, Decimal::from(12325u32));
        assert_eq!(py.num_shares, Decimal::from(12325u32));

        assert_eq!(lop.commission, dec!(9.93));
        assert_eq!(py.commission, dec!(9.93));

        // Security should use canonical symbol
        assert_eq!(lop.security, "DLR.TO");
        assert_eq!(py.security, "DLR.TO");

        let expected_trade_date =
            time::Date::from_calendar_date(2026, time::Month::January, 7).unwrap();
        let expected_settle =
            time::Date::from_calendar_date(2026, time::Month::January, 8).unwrap();
        assert_eq!(lop.trade_date, expected_trade_date);
        assert_eq!(py.trade_date, expected_trade_date);
        assert_eq!(lop.settlement_date, expected_settle);
        assert_eq!(py.settlement_date, expected_settle);

        assert_eq!(
            lop.memo,
            "BMO Trade ; G036247 AKA DLR.U.TO - GLOBAL X US DLR CURRENCY"
        );
        assert_eq!(
            py.memo,
            "BMO Trade ; G036247 AKA DLR.U.TO - GLOBAL X US DLR CURRENCY"
        );

        assert_eq!(lop.account_number, "123-XXXXX123");
        assert_eq!(py.account_number, "123-XXXXX123");
        assert_eq!(lop.account_type, "CSH");
        assert_eq!(py.account_type, "CSH");
        assert_eq!(lop.client_name, "MR JOHN DOE");
        assert_eq!(py.client_name, "MR JOHN DOE");

        let expected_gross = dec!(14.01) * Decimal::from(12325u32);
        assert_eq!(lop.gross_amount, expected_gross);
        assert_eq!(py.gross_amount, expected_gross);

        assert_eq!(lop.order_number, "987613");
        assert_eq!(py.order_number, "987613");
    }

    #[test]
    fn test_parse_buy_nocode_lopdf_pypdf() {
        let lopdf_text =
            read_sample("tests/data/bmo_scenarios/2026_sample/lopdf/buy_nocode.txt");
        let pypdf_text =
            read_sample("tests/data/bmo_scenarios/2026_sample/pypdf/buy_nocode.txt");

        let lop =
            parse_bmo_trade(&lopdf_text, &std::path::PathBuf::from("lopdf.txt"))
                .unwrap();
        let py =
            parse_bmo_trade(&pypdf_text, &std::path::PathBuf::from("pypdf.txt"))
                .unwrap();

        // SECURITY NO. is not a resolvable alias, so fallback to security string from file
        let expected_security = "GLOBAL X US DLR CURRENCY";
        assert_eq!(lop.security, expected_security);
        assert_eq!(py.security, expected_security);

        assert_eq!(lop.memo, "BMO Trade ; G999999");
        assert_eq!(py.memo, "BMO Trade ; G999999");

        assert_eq!(lop.order_number, "987611");
        assert_eq!(py.order_number, "987611");
    }

    #[test]
    fn test_re_date_monthname_variants() {
        // Full month
        let t1 = "JANUARY 7, 2026";
        let c1 = RE_DATE_MONTHNAME.captures(t1).expect("should capture");
        assert_eq!(c1.get(1).unwrap().as_str().to_uppercase(), "JANUARY");
        assert_eq!(c1.get(2).unwrap().as_str(), "7");
        assert_eq!(c1.get(3).unwrap().as_str(), "2026");

        // Short month, mixed case
        let t2 = "Jan 07, 26";
        let c2 = RE_DATE_MONTHNAME.captures(t2).expect("should capture short month");
        assert_eq!(c2.get(1).unwrap().as_str().to_uppercase(), "JAN");
        assert_eq!(c2.get(2).unwrap().as_str(), "07");
        assert_eq!(c2.get(3).unwrap().as_str(), "26");

        // Another short month
        let t3 = "Sep 9, 2025";
        let c3 = RE_DATE_MONTHNAME.captures(t3).expect("should capture Sep");
        assert_eq!(c3.get(1).unwrap().as_str().to_uppercase(), "SEP");
        assert_eq!(c3.get(2).unwrap().as_str(), "9");
        assert_eq!(c3.get(3).unwrap().as_str(), "2025");
    }

    #[test]
    fn test_money_value_variants() {
        let samples = vec![
            "1,234.56",
            "1234.56",
            "1234",
            "1",
            "10",
            ".95",
            "0.95",
            "1,234",
            "12,345,678.90",
        ];
        for s in samples {
            let m = MONEY_VALUE.captures(s).expect(&format!("should match {}", s));
            assert_eq!(m.get(0).unwrap().as_str(), s);
        }
    }

    #[test]
    fn test_parse_buy_vee_nocommission_lopdf_pypdf() {
        let lopdf_text = read_sample(
            "tests/data/bmo_scenarios/2022_sample/lopdf/buy_vee_nocommission.txt",
        );
        let pypdf_text = read_sample(
            "tests/data/bmo_scenarios/2022_sample/pypdf/buy_vee_nocommission.txt",
        );

        let lop =
            parse_bmo_trade(&lopdf_text, &std::path::PathBuf::from("lopdf.txt"))
                .unwrap();
        let py =
            parse_bmo_trade(&pypdf_text, &std::path::PathBuf::from("pypdf.txt"))
                .unwrap();

        // Both should be BUY and CAD
        assert_eq!(lop.action, TxAction::Buy);
        assert_eq!(py.action, TxAction::Buy);
        assert_eq!(lop.currency, Currency::cad());
        assert_eq!(py.currency, Currency::cad());

        assert_eq!(lop.amount_per_share, dec!(32.6300));
        assert_eq!(py.amount_per_share, dec!(32.6300));

        assert_eq!(lop.num_shares, Decimal::from(50u32));
        assert_eq!(py.num_shares, Decimal::from(50u32));

        assert_eq!(lop.commission, Decimal::ZERO);
        assert_eq!(py.commission, Decimal::ZERO);

        // Security should use canonical symbol
        assert_eq!(lop.security, "VEE.TO");
        assert_eq!(py.security, "VEE.TO");

        let expected_trade_date =
            time::Date::from_calendar_date(2022, time::Month::December, 25).unwrap();
        let expected_settle =
            time::Date::from_calendar_date(2022, time::Month::December, 27).unwrap();
        assert_eq!(lop.trade_date, expected_trade_date);
        assert_eq!(py.trade_date, expected_trade_date);
        assert_eq!(lop.settlement_date, expected_settle);
        assert_eq!(py.settlement_date, expected_settle);

        assert_eq!(
            lop.memo,
            "BMO Trade ; V009796 - VANGUARDFTSE EMERGING MKTS"
        );
        assert_eq!(
            py.memo,
            "BMO Trade ; V009796 - VANGUARDFTSE EMERGING MKTS"
        );

        assert_eq!(lop.account_number, "999-9999999");
        assert_eq!(py.account_number, "999-9999999");
        assert_eq!(lop.account_type, "CSH");
        assert_eq!(py.account_type, "CSH");
        assert_eq!(lop.client_name, "MR JOHN DOE");
        assert_eq!(py.client_name, "MR JOHN DOE");

        let expected_gross = dec!(32.6300) * Decimal::from(50u32);
        assert_eq!(lop.gross_amount, expected_gross);
        assert_eq!(py.gross_amount, expected_gross);

        assert_eq!(lop.order_number, "999123");
        assert_eq!(py.order_number, "999123");
    }

    #[test]
    fn test_parse_buy_hisa_lopdf_pypdf() {
        let lopdf_text = read_sample(
            "tests/data/bmo_scenarios/2023_hisa/lopdf/lopdf_buy_hisa.txt",
        );
        let pypdf_text = read_sample(
            "tests/data/bmo_scenarios/2023_hisa/pypdf/pypdf_buy_hisa.txt",
        );

        let lop =
            parse_bmo_trade(&lopdf_text, &std::path::PathBuf::from("lopdf.txt"))
                .unwrap();
        let py =
            parse_bmo_trade(&pypdf_text, &std::path::PathBuf::from("pypdf.txt"))
                .unwrap();

        // Both should be BUY and CAD
        assert_eq!(lop.action, TxAction::Buy);
        assert_eq!(py.action, TxAction::Buy);
        assert_eq!(lop.currency, Currency::cad());
        assert_eq!(py.currency, Currency::cad());

        assert_eq!(lop.amount_per_share, dec!(1.0000));
        assert_eq!(py.amount_per_share, dec!(1.0000));

        assert_eq!(lop.num_shares, Decimal::from(9000u32));
        assert_eq!(py.num_shares, Decimal::from(9000u32));

        assert_eq!(lop.commission, Decimal::ZERO);
        assert_eq!(py.commission, Decimal::ZERO);

        // Security should use canonical symbol from alias
        assert_eq!(lop.security, "BMT CAD HISA");
        assert_eq!(py.security, "BMT CAD HISA");

        let expected_trade_date =
            time::Date::from_calendar_date(2023, time::Month::March, 15).unwrap();
        let expected_settle =
            time::Date::from_calendar_date(2023, time::Month::March, 15).unwrap();
        assert_eq!(lop.trade_date, expected_trade_date);
        assert_eq!(py.trade_date, expected_trade_date);
        assert_eq!(lop.settlement_date, expected_settle);
        assert_eq!(py.settlement_date, expected_settle);

        assert_eq!(
            lop.memo,
            "BMO Trade ; B074340 AKA BMT104/BMT109 - BANK OF MONTREAL CAD HISA"
        );
        assert_eq!(
            py.memo,
            "BMO Trade ; B074340 AKA BMT104/BMT109 - BANK OF MONTREAL CAD HISA"
        );

        assert_eq!(lop.account_number, "999-9999999");
        assert_eq!(py.account_number, "999-9999999");
        assert_eq!(lop.account_type, "CSH");
        assert_eq!(py.account_type, "CSH");
        assert_eq!(lop.client_name, "MR JOHN DOE");
        assert_eq!(py.client_name, "MR JOHN DOE");

        let expected_gross = dec!(1.0000) * Decimal::from(9000u32);
        assert_eq!(lop.gross_amount, expected_gross);
        assert_eq!(py.gross_amount, expected_gross);

        assert_eq!(lop.order_number, "999999");
        assert_eq!(py.order_number, "999999");
    }
}
