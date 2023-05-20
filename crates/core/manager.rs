#![allow(dead_code)]

use binan_spot::market::klines::KlineInterval;
use quant_api::res::{account_info, kline, ticker_price};
use quant_db::recorder::Recorder;
use quant_util::{
    log,
    time::{LocalTimeTool, TimeConverter, TimeZoneConverter},
};

use crate::{api::Api, time};

pub struct Manager {
    api: Api,
    recorder: Recorder,
}

unsafe impl Send for Manager {}

unsafe impl Sync for Manager {}

impl Default for Manager {
    fn default() -> Self {
        Self {
            api: Api::default_with_proxy(),
            recorder: Recorder::default(),
        }
    }
}

impl Manager {
    pub fn init(&self) -> Result<(), Box<dyn std::error::Error>> {
        log::Logger::setup_logger()?;

        self.recorder.init();

        Ok(())
    }

    pub fn recorder(&self) -> &Recorder {
        &self.recorder
    }

    pub async fn get_account_snapshot(&self) -> String {
        self.api.get_account_snapshot().await
    }

    pub async fn get_account_info(&self) -> account_info::AccountInfoRes {
        self.api.get_account_info().await
    }

    pub async fn get_ticker_price(&self, symbol: &str) -> ticker_price::TickerPriceRes {
        let (date_time, unix_time) = time::DateTime::get_local_current();
        let ticker_price = self.api.get_ticker_price(symbol).await;

        self.recorder.record_ticker_price_data(
            &["name", "price", "unix_time", "date_time"],
            (
                &ticker_price.symbol,
                &ticker_price.price,
                &unix_time,
                &date_time,
            ),
        );

        ticker_price
    }

    pub async fn get_kline(
        &self,
        symbol: &str,
        interval: KlineInterval,
        start_time: u64,
        end_time: u64,
    ) -> Vec<kline::KlineRes> {
        let klines = self
            .api
            .get_kline(symbol, interval, start_time, end_time)
            .await;
        for i in &klines {
            let open_date_time = LocalTimeTool::convert_to_date_time(
                TimeZoneConverter::convert_utc_to_local(i.open_time),
            )
            .unwrap();
            let close_date_time = LocalTimeTool::convert_to_date_time(
                TimeZoneConverter::convert_utc_to_local(i.close_time),
            )
            .unwrap();
            self.recorder.record_kline_data(
                &[
                    "name",
                    "open_price",
                    "high_price",
                    "low_price",
                    "close_price",
                    "volume",
                    "quote_asset_volume",
                    "open_date_time",
                    "close_date_time",
                    "open_unix_time",
                    "close_unix_time",
                ],
                (
                    &symbol,
                    &i.open_price,
                    &i.high_price,
                    &i.low_price,
                    &i.close_price,
                    &i.volume,
                    &i.quote_asset_volume,
                    &open_date_time,
                    &close_date_time,
                    &i.open_time,
                    &i.close_time,
                ),
            );
        }

        klines
    }
}
