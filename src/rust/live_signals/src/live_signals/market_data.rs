use super::*;

#[derive(Clone, Debug)]
pub(super) enum MarketEvent {
    Price {
        timestamp: u64,
        price: f64,
        volatility_1m: f64,
    },
    Trade {
        timestamp: u64,
        price: f64,
        size: f64,
        is_buy: bool,
    },
    Book {
        timestamp: u64,
        best_bid: f64,
        best_ask: f64,
        bid_sizes: [f64; 7],
        ask_sizes: [f64; 7],
        bid_levels: [(f64, f64); 7],
        ask_levels: [(f64, f64); 7],
    },
}

#[derive(Debug)]
pub(super) struct SignalEvent {
    pub(super) symbol: String,
    pub(super) event: MarketEvent,
    pub(super) queued_at: Instant,
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn decode_event(bytes: &[u8]) -> Option<SignalEvent> {
    let symbol = std::str::from_utf8(bytes.get(8..40)?)
        .ok()?
        .trim_end_matches([' ', '\0'])
        .to_owned();
    if symbol.is_empty() {
        return None;
    }
    let template = u16::from_le_bytes(bytes.get(2..4)?.try_into().ok()?);
    let event = match template {
        4 if is_enriched_symbol_price(bytes) => MarketEvent::Price {
            timestamp: u64::from_le_bytes(bytes.get(48..56)?.try_into().ok()?),
            price: f64::from_le_bytes(bytes.get(40..48)?.try_into().ok()?),
            volatility_1m: f64::from_le_bytes(bytes.get(56..64)?.try_into().ok()?),
        },
        2 if bytes.len() >= 272 => {
            let bid_levels = std::array::from_fn(|index| {
                (
                    f64::from_le_bytes(
                        bytes[48 + index * 16..56 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                    f64::from_le_bytes(
                        bytes[56 + index * 16..64 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                )
            });
            let ask_levels = std::array::from_fn(|index| {
                (
                    f64::from_le_bytes(
                        bytes[160 + index * 16..168 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                    f64::from_le_bytes(
                        bytes[168 + index * 16..176 + index * 16]
                            .try_into()
                            .unwrap_or_default(),
                    ),
                )
            });
            MarketEvent::Book {
                timestamp: u64::from_le_bytes(bytes.get(40..48)?.try_into().ok()?),
                best_bid: bid_levels[0].0,
                best_ask: ask_levels[0].0,
                bid_sizes: bid_levels.map(|(_, size)| size),
                ask_sizes: ask_levels.map(|(_, size)| size),
                bid_levels,
                ask_levels,
            }
        }
        3 if bytes.len() >= 97 => MarketEvent::Trade {
            timestamp: u64::from_le_bytes(bytes.get(72..80)?.try_into().ok()?),
            price: f64::from_le_bytes(bytes.get(80..88)?.try_into().ok()?),
            size: f64::from_le_bytes(bytes.get(88..96)?.try_into().ok()?),
            is_buy: bytes[96] == 1,
        },
        _ => return None,
    };
    Some(SignalEvent {
        symbol,
        event,
        queued_at: Instant::now(),
    })
}

fn is_enriched_symbol_price(bytes: &[u8]) -> bool {
    bytes.len() >= 72
        && u16::from_le_bytes(bytes[0..2].try_into().unwrap_or_default()) == 64
        && u16::from_le_bytes(bytes[2..4].try_into().unwrap_or_default()) == 4
        && u16::from_le_bytes(bytes[4..6].try_into().unwrap_or_default()) == 1
        && u16::from_le_bytes(bytes[6..8].try_into().unwrap_or_default()) == 9
}

pub(super) fn book_value(timestamp: u64, bids: &[(f64, f64)], asks: &[(f64, f64)]) -> Value {
    let encode = |levels: &[(f64, f64)]| {
        levels
            .iter()
            .filter(|(price, size)| {
                price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0
            })
            .map(|(price, size)| json!({"price": price, "size": size}))
            .collect::<Vec<_>>()
    };
    json!({"bids": encode(bids), "asks": encode(asks), "updated_at_ms": timestamp})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_event(template: u16, length: usize) -> Vec<u8> {
        let mut bytes = vec![0; length];
        bytes[2..4].copy_from_slice(&template.to_le_bytes());
        bytes[8..16].copy_from_slice(b"BTC-USDT");
        bytes
    }

    fn put_f64(bytes: &mut [u8], offset: usize, value: f64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn now_ms_returns_epoch_milliseconds() {
        assert!(now_ms() > 1_700_000_000_000);
    }

    #[test]
    fn decodes_price_trade_and_book_templates() {
        let mut price = base_event(4, 72);
        price[0..2].copy_from_slice(&64_u16.to_le_bytes());
        price[4..6].copy_from_slice(&1_u16.to_le_bytes());
        price[6..8].copy_from_slice(&9_u16.to_le_bytes());
        put_f64(&mut price, 40, 42_000.5);
        price[48..56].copy_from_slice(&123_456_u64.to_le_bytes());
        put_f64(&mut price, 56, 0.25);
        let decoded = decode_event(&price).unwrap();
        assert_eq!(decoded.symbol, "BTC-USDT");
        assert!(matches!(
            decoded.event,
            MarketEvent::Price {
                timestamp: 123_456,
                price: 42_000.5,
                volatility_1m: 0.25
            }
        ));

        let mut trade = base_event(3, 97);
        trade[72..80].copy_from_slice(&654_321_u64.to_le_bytes());
        put_f64(&mut trade, 80, 42_001.0);
        put_f64(&mut trade, 88, 0.75);
        trade[96] = 1;
        assert!(matches!(
            decode_event(&trade).unwrap().event,
            MarketEvent::Trade {
                timestamp: 654_321,
                price: 42_001.0,
                size: 0.75,
                is_buy: true
            }
        ));

        let mut book = base_event(2, 272);
        book[40..48].copy_from_slice(&777_u64.to_le_bytes());
        put_f64(&mut book, 48, 41_999.0);
        put_f64(&mut book, 56, 2.0);
        put_f64(&mut book, 160, 42_001.0);
        put_f64(&mut book, 168, 3.0);
        assert!(matches!(
            decode_event(&book).unwrap().event,
            MarketEvent::Book {
                timestamp: 777,
                best_bid: 41_999.0,
                best_ask: 42_001.0,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unknown_truncated_and_symbol_less_events() {
        assert!(decode_event(&base_event(99, 100)).is_none());
        assert!(decode_event(&base_event(1, 71)).is_none());
        assert!(decode_event(&[0; 20]).is_none());
        assert!(decode_event(&base_event(1, 72)).is_none());
        assert!(decode_event(&base_event(4, 71)).is_none());
        let mut no_symbol = vec![0; 72];
        no_symbol[2..4].copy_from_slice(&1_u16.to_le_bytes());
        assert!(decode_event(&no_symbol).is_none());
    }

    #[test]
    fn book_value_filters_invalid_levels_and_preserves_timestamp() {
        let value = book_value(
            900,
            &[(100.0, 1.0), (f64::NAN, 2.0), (99.0, 0.0)],
            &[(101.0, 3.0), (-1.0, 2.0)],
        );
        assert_eq!(value["updated_at_ms"], 900);
        assert_eq!(value["bids"].as_array().unwrap().len(), 1);
        assert_eq!(value["asks"].as_array().unwrap().len(), 1);
        assert_eq!(value["bids"][0]["price"], 100.0);
    }
}
