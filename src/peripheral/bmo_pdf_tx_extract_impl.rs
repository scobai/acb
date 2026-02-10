use std::io::{Read, Write};
use std::path::PathBuf;

use clap::Parser;

use super::broker::bmo;
use crate::app::outfmt::csv::CsvWriter;
use crate::app::outfmt::model::{AcbWriter, OutputType};
use crate::app::outfmt::text::TextWriter;
use crate::portfolio::render::RenderTable;
use crate::portfolio::{CsvTx, Currency};
use crate::util::rw::WriteHandle;
use crate::write_errln;
use crate::{peripheral::pdf, util::basic::SError};

/// A convenience script to extract transactions from PDFs downloaded from
/// BMO InvestorLine (bmo.com)
///
/// Instructions:
/// Go to your investorline account.
/// From Documents > eDocuments > Confirmations & Other documents download all buy
/// and sell trade confirmations.
/// Run this script, giving the names of all PDFs as arguments.
/// I strongly recommend also downloading the Transaction History > Download > CSV
/// for all transactions, and comparing the output of this script to that CSV.
/// The difference will be that the PDFs are the only (current) place to find
/// the commission amounts, which are needed for accurate ACB calculations.
#[derive(Parser, Debug)]
#[command(author, about, long_about)]
pub struct Args {
    /// BMO trade confirmation PDFs
    ///
    /// These can also be plain .txt files, and will not be interpreted as actual
    /// PDFs, but just the text emitted by a tool like pdf-text.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,

    /// Print pretty tables instead of CSV
    #[arg(short = 'p', long)]
    pub pretty: bool,

    /// Turn on some very verbose debug printing
    ///
    /// Does not affect tracing. Set TRACE variable for this.
    #[arg(long)]
    pub debug: bool,
}

struct ParsedTrades {
    pub trades: Vec<bmo::BmoTrade>,
}

fn parse_pdfs(files: &Vec<PathBuf>, debug: bool) -> Result<ParsedTrades, SError> {
    let mut trades: Vec<bmo::BmoTrade> = Vec::new();

    for (i, fpath) in files.iter().enumerate() {
        if i != 0 {
            if debug {
                // Line separator between entries
                eprintln!()
            }
        }
        if debug {
            eprintln!("Parsing {fpath:?}");
        }

        let pdf_text = if fpath.extension().unwrap_or_default().to_string_lossy()
            == "txt"
        {
            // This is mostly for testing. We can just read the pre-parsed pdf text
            tracing::trace!("Getting raw text from {:?}", fpath);
            let mut buf = String::new();
            std::fs::File::open(fpath)
                .map_err(|e| format!("Failed to open text file {fpath:?}: {e}"))?
                .read_to_string(&mut buf)
                .map_err(|e| format!("Failed to read text file {fpath:?}: {e}"))?;
            buf
        } else {
            pdf::get_all_pages_text_from_path(fpath)
                .map_err(|e| format!("Failed to read {fpath:?}: {e}"))?
                .join("\n")
        };

        let trade = bmo::parse_bmo_trade(&pdf_text, fpath)
            .map_err(|e| format!("Failed to parse {fpath:?}: {e}"))?;

        if debug {
            eprintln!("{trade:#?}");
        }
        trades.push(trade);
    }

    Ok(ParsedTrades { trades })
}

fn dump_extracted_data(
    parsed_trades: &ParsedTrades,
    pretty: bool,
    out_w: WriteHandle,
) {
    let mut printer: Box<dyn AcbWriter> = if pretty {
        Box::new(TextWriter::new(out_w.clone()))
    } else {
        Box::new(CsvWriter::new_to_writer(out_w.clone()))
    };

    let mut rt = RenderTable::default();
    rt.header.extend(
        vec![
            "security",
            "trade_date",
            "settlement_date",
            "action",
            "amount_per_share",
            "num_shares",
            "commission",
            "currency",
            "memo",
            "account",
            "account_type",
            "client_name",
            "gross_amount",
        ]
        .into_iter()
        .map(String::from),
    );

    for t in &parsed_trades.trades {
        rt.rows.push(vec![
            t.security.clone(),
            t.trade_date.to_string(),
            t.settlement_date.to_string(),
            t.action.to_string(),
            t.amount_per_share.to_string(),
            t.num_shares.to_string(),
            t.commission.to_string(),
            t.currency.to_string(),
            t.memo.clone(),
            t.account_number.clone(),
            t.account_type.clone(),
            t.client_name.clone(),
            t.gross_amount.to_string(),
        ]);
    }

    let _ = printer.print_render_table(OutputType::Raw, "trades", &rt).unwrap();
}

/// Constructs/extracts CsvTxs from the parsed BMO trades.
fn txs_from_trades(trades: &[bmo::BmoTrade]) -> Result<Vec<CsvTx>, SError> {
    let mut csv_txs = Vec::new();

    for (i, trade) in trades.iter().enumerate() {
        // Only accept CAD or USD from BMO trade confirmations
        if trade.currency != Currency::cad() && trade.currency != Currency::usd() {
            return Err(format!(
                "Unsupported currency for BMO extractor: {}",
                trade.currency
            ));
        }

        let csv_tx = CsvTx {
            security: Some(trade.security.clone()),
            trade_date: Some(trade.trade_date),
            settlement_date: Some(trade.settlement_date),
            action: Some(trade.action),
            shares: Some(trade.num_shares),
            amount_per_share: Some(trade.amount_per_share),
            total_amount: None,
            commission: Some(trade.commission),
            tx_currency: Some(trade.currency.clone()),
            tx_curr_to_local_exchange_rate: None,
            // See Self-Directed Commission & Fee Schedule January 2026:
            // Commissions on U.S. trades are charged in U.S. dollars
            // Set commission_currency explicitly to the TX currency.
            commission_currency: Some(trade.currency.clone()),
            commission_curr_to_local_exchange_rate: None,
            memo: Some(trade.memo.clone()),
            affiliate: None,
            specified_superficial_loss: None,
            // TODO handle stock splits.
            stock_split_ratio: None,
            read_index: i.try_into().unwrap(),
        };
        csv_txs.push(csv_tx);
    }

    csv_txs.sort();
    Ok(csv_txs)
}

fn render_txs_from_trades(
    trades: &[bmo::BmoTrade],
    pretty: bool,
    out_w: WriteHandle,
) -> Result<(), SError> {
    let txs = txs_from_trades(trades)?;

    let mut printer: Box<dyn AcbWriter> = if pretty {
        Box::new(TextWriter::new(out_w))
    } else {
        Box::new(CsvWriter::new_to_writer(out_w))
    };
    let table_name = if pretty { "BMO TXs" } else { "bmo_txs" };
    let csv_table = crate::portfolio::io::tx_csv::txs_to_csv_table(&txs);
    printer.print_render_table(
        crate::app::outfmt::model::OutputType::Raw,
        &table_name,
        &crate::portfolio::render::RenderTable::from(csv_table),
    )
}

pub fn run() -> Result<(), ()> {
    let args = Args::parse();
    run_with_args(
        args,
        WriteHandle::stdout_write_handle(),
        WriteHandle::stderr_write_handle(),
    )
}

pub fn run_with_args(
    mut args: Args,
    mut out_w: WriteHandle,
    mut err_w: WriteHandle,
) -> Result<(), ()> {
    if args.debug {
        crate::tracing::enable_trace_env(
            "acb::peripheral::bmo_pdf_tx_extract_impl=debug",
        );
    }
    crate::tracing::setup_tracing();

    // Sort the files, so that we can deterministically output them in the same
    // order. This affects tie-breaks when we have multiple TXs on the same day.
    args.files.sort();

    let parsed_trades = parse_pdfs(&args.files, args.debug)
        .map_err(|e| write_errln!(err_w, "{}", e))?;

    if parsed_trades.trades.is_empty() {
        write_errln!(err_w, "WARN: No trades entries");
    }

    // Show extracted data in debug mode
    if args.debug {
        dump_extracted_data(&parsed_trades, args.pretty, out_w.clone());
        let _ = writeln!(out_w, "");
    }

    render_txs_from_trades(&parsed_trades.trades, args.pretty, out_w)
        .map_err(|e| write_errln!(err_w, "Error: {e}"))?;

    Ok(())
}

// MARK: tests

#[cfg(test)]
mod tests {
    use crate::portfolio::TxAction;

    use super::*;

    #[test]
    fn test_txs_from_trades() {
        let trades = vec![
            bmo::BmoTrade {
                security: "XUS.TO".to_string(),
                trade_date: time::Date::from_calendar_date(
                    2026,
                    time::Month::January,
                    7,
                )
                .unwrap(),
                settlement_date: time::Date::from_calendar_date(
                    2026,
                    time::Month::January,
                    8,
                )
                .unwrap(),
                action: TxAction::Sell,
                amount_per_share: "14.01".parse().unwrap(),
                num_shares: "12345".parse().unwrap(),
                commission: "9.93".parse().unwrap(),
                currency: Currency::cad(),
                memo: "BMO Trade 2026-01-07".to_string(),
                account_number: "123-XXXXX123".to_string(),
                account_type: "CSH".to_string(),
                client_name: "MR JOHN DOE".to_string(),
                gross_amount: "172953.45".parse().unwrap(),
            },
            bmo::BmoTrade {
                security: "IVV".to_string(),
                trade_date: time::Date::from_calendar_date(
                    2026,
                    time::Month::January,
                    7,
                )
                .unwrap(),
                settlement_date: time::Date::from_calendar_date(
                    2026,
                    time::Month::January,
                    8,
                )
                .unwrap(),
                action: TxAction::Buy,
                amount_per_share: "14.01".parse().unwrap(),
                num_shares: "12345".parse().unwrap(),
                commission: "9.93".parse().unwrap(),
                currency: Currency::usd(),
                memo: "BMO Trade 2026-01-07".to_string(),
                account_number: "123-XXXXX123".to_string(),
                account_type: "CSH".to_string(),
                client_name: "MR JOHN DOE".to_string(),
                gross_amount: "172953.45".parse().unwrap(),
            },
        ];

        let txs = txs_from_trades(&trades).unwrap();
        assert_eq!(txs.len(), 2);

        for tx in &txs {
            assert_eq!(
                tx.trade_date,
                Some(
                    time::Date::from_calendar_date(2026, time::Month::January, 7)
                        .unwrap()
                )
            );
            assert_eq!(
                tx.settlement_date,
                Some(
                    time::Date::from_calendar_date(2026, time::Month::January, 8)
                        .unwrap()
                )
            );
            assert_eq!(tx.memo, Some("BMO Trade 2026-01-07".to_string()));
            assert_eq!(tx.commission, Some("9.93".parse().unwrap()));
            assert_eq!(tx.shares, Some("12345".parse().unwrap()));
            assert_eq!(tx.amount_per_share, Some("14.01".parse().unwrap()));
            assert_eq!(
                tx.commission_currency,
                Some(tx.tx_currency.clone().unwrap())
            );

            if tx.security.as_ref().unwrap() == "XUS.TO" {
                assert_eq!(tx.action, Some(TxAction::Sell));
                assert_eq!(tx.tx_currency, Some(Currency::cad()));
            } else if tx.security.as_ref().unwrap() == "IVV" {
                assert_eq!(tx.action, Some(TxAction::Buy));
                assert_eq!(tx.tx_currency, Some(Currency::usd()));
            } else {
                panic!("Unexpected security: {:?}", tx.security);
            }
        }
    }

    #[test]
    fn test_txs_from_trades_rejects_other_currency() {
        let trades = vec![bmo::BmoTrade {
            security: "7203".to_string(),
            trade_date: time::Date::from_calendar_date(
                2026,
                time::Month::January,
                7,
            )
            .unwrap(),
            settlement_date: time::Date::from_calendar_date(
                2026,
                time::Month::January,
                8,
            )
            .unwrap(),
            action: TxAction::Sell,
            amount_per_share: "14.01".parse().unwrap(),
            num_shares: "12345".parse().unwrap(),
            commission: "9.93".parse().unwrap(),
            currency: Currency::new("JPY"),
            memo: "BMO Trade 2026-01-07".to_string(),
            account_number: "123-XXXXX123".to_string(),
            account_type: "CSH".to_string(),
            client_name: "MR JOHN DOE".to_string(),
            gross_amount: "172953.45".parse().unwrap(),
        }];

        let res = txs_from_trades(&trades);
        match res {
            Err(e) => assert!(
                e.contains("Unsupported currency for BMO extractor"),
                "unexpected error: {}",
                e
            ),
            Ok(_) => panic!("Expected error for unsupported currency"),
        }
    }

    #[test]
    fn test_parse_pdfs_lopdf_pypdf_consistency() {
        use std::fs;

        let mut lopdf_files: Vec<_> =
            fs::read_dir("tests/data/bmo_scenarios/2026_sample/lopdf")
                .expect("Failed to read lopdf directory")
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("txt") {
                        Some(path)
                    } else {
                        None
                    }
                })
                .collect();
        lopdf_files.sort();

        let mut pypdf_files: Vec<_> =
            fs::read_dir("tests/data/bmo_scenarios/2026_sample/pypdf")
                .expect("Failed to read pypdf directory")
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) == Some("txt") {
                        Some(path)
                    } else {
                        None
                    }
                })
                .collect();
        pypdf_files.sort();

        // Verify we have the expected files
        assert_eq!(lopdf_files.len(), 3, "Expected 3 files in lopdf directory");
        assert_eq!(pypdf_files.len(), 3, "Expected 3 files in pypdf directory");

        // Parse files from both directories
        let lopdf_trades =
            parse_pdfs(&lopdf_files, false).expect("Failed to parse lopdf files");
        let pypdf_trades =
            parse_pdfs(&pypdf_files, false).expect("Failed to parse pypdf files");

        // Verify same number of trades
        assert_eq!(
            lopdf_trades.trades.len(),
            pypdf_trades.trades.len(),
            "lopdf and pypdf should produce same number of trades"
        );

        // Verify each trade matches between lopdf and pypdf
        for (i, (lop_trade, py_trade)) in
            lopdf_trades.trades.iter().zip(pypdf_trades.trades.iter()).enumerate()
        {
            assert_eq!(
                lop_trade.security, py_trade.security,
                "Trade {}: security mismatch",
                i
            );
            assert_eq!(
                lop_trade.trade_date, py_trade.trade_date,
                "Trade {}: trade_date mismatch",
                i
            );
            assert_eq!(
                lop_trade.settlement_date, py_trade.settlement_date,
                "Trade {}: settlement_date mismatch",
                i
            );
            assert_eq!(
                lop_trade.action, py_trade.action,
                "Trade {}: action mismatch",
                i
            );
            assert_eq!(
                lop_trade.amount_per_share, py_trade.amount_per_share,
                "Trade {}: amount_per_share mismatch",
                i
            );
            assert_eq!(
                lop_trade.num_shares, py_trade.num_shares,
                "Trade {}: num_shares mismatch",
                i
            );
            assert_eq!(
                lop_trade.commission, py_trade.commission,
                "Trade {}: commission mismatch",
                i
            );
            assert_eq!(
                lop_trade.currency, py_trade.currency,
                "Trade {}: currency mismatch",
                i
            );
            assert_eq!(lop_trade.memo, py_trade.memo, "Trade {}: memo mismatch", i);
            assert_eq!(
                lop_trade.account_number, py_trade.account_number,
                "Trade {}: account_number mismatch",
                i
            );
            assert_eq!(
                lop_trade.account_type, py_trade.account_type,
                "Trade {}: account_type mismatch",
                i
            );
            assert_eq!(
                lop_trade.client_name, py_trade.client_name,
                "Trade {}: client_name mismatch",
                i
            );
            assert_eq!(
                lop_trade.gross_amount, py_trade.gross_amount,
                "Trade {}: gross_amount mismatch",
                i
            );
        }
    }
}
