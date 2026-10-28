fn parse_decimal(value: &str) -> Result<u128> {
    ensure!(!value.is_empty(), "decimal value cannot be empty");
    let mut whole = 0_u128;
    let mut fraction = 0_u128;
    let mut whole_digits = 0;
    let mut fraction_digits = 0;
    let mut decimal_seen = false;

    for byte in value.bytes() {
        if byte == b'.' && !decimal_seen {
            decimal_seen = true;
            continue;
        }

        ensure!(byte.is_ascii_digit(), "invalid decimal value {value}");
        let digit = u128::from(byte - b'0');
        if decimal_seen {
            fraction_digits += 1;
            ensure!(
                fraction_digits <= 18,
                "decimal value {value} has more than 18 fractional digits"
            );
            fraction = fraction
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .ok_or_else(|| anyhow!("decimal value {value} is too large"))?;
        } else {
            whole_digits += 1;
            whole = whole
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .ok_or_else(|| anyhow!("decimal value {value} is too large"))?;
        }
    }

    ensure!(whole_digits > 0, "invalid decimal value {value}");
    for _ in fraction_digits..18 {
        fraction *= 10;
    }
    whole
        .checked_mul(PRICE_SCALE)
        .and_then(|whole_units| whole_units.checked_add(fraction))
        .ok_or_else(|| anyhow!("decimal value {value} is too large"))
}
