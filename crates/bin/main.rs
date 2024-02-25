#![allow(dead_code, unused)]
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use std::{path, sync::OnceLock};

use binan_spot::market::klines::KlineInterval;
use binan_spot::trade::order::{Side, TimeInForce};
use quant_api::message::{KlineApiRequest, NewOrderApiRequest, TickerApiRequest};
use quant_config::ConfigBuilder;
use quant_indicator::{data_item::ToDataItem, macd::MacdOutputBuilder};
use quant_model::kline::Kline;
use quant_util::time::{DurationInterval, GetDuration, UtcTimeTool};

use clap::Parser;
use rust_decimal_macros::dec;

use manager::QuantState;
use tokio::sync::Mutex;

mod api;
mod manager;

static STATE: OnceLock<QuantState> = OnceLock::new();

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    config: path::PathBuf,
}

#[actix::main]
async fn main() -> Result<(), quant_core::Error> {
    tokio::task::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for event");

        tracing::info!("Ctrl-C received, stop system");

        STATE.get().unwrap().stop().await;

        tracing::info!("Shutdown now");

        std::process::exit(0);
    });

    let args: Cli = Cli::parse();
    let config = ConfigBuilder::build(args.config)?;

    let _manager = STATE.get_or_init(|| {
        let mut m = QuantState::from_config(config.to_owned()).expect("Failed to create manager");
        let _ = m.init();
        m
    });

    run().await
}

async fn run() -> Result<(), quant_core::Error> {
    let symbol = "BTCUSDT";
    let total = dec!(50.0);
    let mut trades = VecDeque::new();

    let manager = STATE.get().unwrap();
    let price_slot = Arc::new(Mutex::new(dec!(1.0)));

    let price = price_slot.clone();
    tokio::task::spawn(async move {
        loop {
            let origin_price = manager
                .get_ticker(TickerApiRequest {
                    symbol: symbol.to_owned(),
                })
                .await
                .unwrap()
                .price();
            *price.lock().await = origin_price;
        }
    });

    'out: loop {
        let (start, end) = UtcTimeTool.get_duration(DurationInterval::Minutes1, 250);
        match manager
            .get_kline(KlineApiRequest {
                symbol: symbol.to_owned(),
                interval: KlineInterval::Minutes5,
                start_time: start,
                end_time: end,
                limit: 50,
            })
            .await
        {
            Ok(res) => {
                let signal = handle_klines_with_macd(&res);
                tracing::debug!(
                    "Signal: {}",
                    signal.map(|s| s.to_string()).unwrap_or("Nil".to_string())
                );

                let origin_price = *price_slot.lock().await;
                tracing::debug!("Ticker of {}: {}", symbol, origin_price);

                match signal {
                    Some(Side::Buy) => {
                        let price = origin_price + dec!(1.0);
                        let stop_price = price * dec!(1.01);
                        let quantity = total / price;

                        let res = manager
                            .new_order(NewOrderApiRequest {
                                symbol: symbol.to_owned(),
                                side: Side::Buy,
                                r#type: "LIMIT".to_owned(),
                                time_in_force: TimeInForce::Gtc,
                                quantity,
                                price,
                                stop_price,
                            })
                            .await?;

                        tracing::info!("Order res: {}", res);
                        trades.push_back((price, quantity));
                    }
                    Some(Side::Sell) if !trades.is_empty() => {
                        let (buy_price, buy_quantity) = trades.pop_front().unwrap();
                        let price = origin_price - dec!(1.0);
                        let stop_price = price * dec!(0.99);

                        let res = manager
                            .new_order(NewOrderApiRequest {
                                symbol: symbol.to_owned(),
                                side: Side::Sell,
                                r#type: "LIMIT".to_owned(),
                                time_in_force: TimeInForce::Gtc,
                                quantity: buy_quantity,
                                price,
                                stop_price,
                            })
                            .await?;

                        tracing::info!("Order res: {}", res);
                        let profit = (price - buy_price) * buy_quantity;
                        tracing::info!("Profit: {}", profit);
                    }
                    _ => {}
                }
            }
            Err(e) => {
                tracing::warn!("{}", e);
                break 'out;
            }
        }

        tokio::time::sleep(Duration::from_secs(60 * 5)).await;
    }

    Ok(())
}

fn handle_klines_with_macd(klines: &[Kline]) -> Option<Side> {
    let item = klines
        .iter()
        .filter_map(|k| k.to_data_item().ok())
        .collect::<Vec<_>>();
    let macd = MacdOutputBuilder::compute(&item).build();

    let fast = macd.iter().map(|m| m.macd).collect::<Vec<_>>();
    let slow = macd.iter().map(|m| m.signal).collect::<Vec<_>>();
    let bar = macd.iter().map(|m| m.histogram).collect::<Vec<_>>();

    let fast_point = fast.last().copied().unwrap_or_default();
    let slow_point = slow.last().copied().unwrap_or_default();
    let bar_point = bar.last().copied().unwrap_or_default();
    let (last_flag, current_flag) = match bar[..] {
        [.., a, b, c] => (b - a, c - b),
        _ => panic!("array shorter than 3"),
    };

    if last_flag * current_flag < 0.0 {
        if fast_point < 0.0
            && slow_point < 0.0
            && bar_point < 0.0
            && last_flag <= 0.0
            && current_flag >= 0.0
        {
            // 当出现 MACD 极小值点且快慢线均小于0
            return Some(Side::Buy);
        }
        if fast_point > 0.0
            && slow_point > 0.0
            && bar_point > 0.0
            && last_flag >= 0.0
            && current_flag <= 0.0
        {
            // 当出现 MACD 极大值点且快慢线均大于0
            return Some(Side::Sell);
        }
    }

    None
}
