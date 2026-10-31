use super::{engine, types::*};
use std::{
    io::{BufRead, Write},
    path::Path,
};
/// Offline deterministic replay: no service startup or order submission.
pub fn run(input: &Path, output: &Path) -> Result<(), String> {
    let reader = std::io::BufReader::new(std::fs::File::open(input).map_err(|e| e.to_string())?);
    let mut book = Book::initialized();
    for s in book.sleeves.values_mut() {
        s.config.enabled = true;
    }
    for (line_number, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let observation: Observation =
            serde_json::from_str(&line).map_err(|e| format!("Line {}: {e}", line_number + 1))?;
        let mut sleeve = book
            .sleeves
            .get(observation.data.id())
            .ok_or("Unknown strategy")?
            .clone();
        engine::apply(&mut sleeve, &observation, true)
            .map_err(|e| format!("Line {}: {e}", line_number + 1))?;
        book.sleeves.insert(sleeve.id.clone(), sleeve);
    }
    let bytes = serde_json::to_vec_pretty(&book).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    for s in book.sleeves.values() {
        println!(
            "{}: equity {:.2} USD, realized {:.2}, fees {:.2}, drawdown {:.2}%, {} open positions",
            s.id,
            s.equity_usd,
            s.realized_pnl_usd,
            s.fees_usd,
            s.max_drawdown_percent,
            s.positions.len()
        );
    }
    Ok(())
}
