use flate2::read::GzDecoder;
use insights::signals::{SignalEvaluation, SignalResearchFeatures, StoredSignalRecord};
use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};
use std::{collections::HashMap, env, fs::{File, OpenOptions}, io::{BufWriter, Read, Write}, path::PathBuf, thread, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

const FEATURES: i64 = 4;
const EVALUATIONS: i64 = 2;

#[derive(Clone)]
struct Candidate { symbol: String, timestamp: u64, horizon: u64, btc_return_5m: f64, side: &'static str }
#[derive(Default)]
struct Summary { count: u64, positive: u64, net_sum: f64, gross_sum: f64 }

fn now_ms() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
fn decode(payload: &[u8]) -> anyhow::Result<StoredSignalRecord> {
    let mut decoder = GzDecoder::new(payload);
    let mut bytes = Vec::new(); decoder.read_to_end(&mut bytes)?;
    Ok(bincode::deserialize(&bytes)?)
}
fn append(writer: &mut BufWriter<File>, event: Value) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *writer, &event)?; writer.write_all(b"\n")?; writer.flush()?; Ok(())
}
fn read_new(db: &Connection, kind: i64, after: i64) -> anyhow::Result<Vec<(i64, Vec<u8>)>> {
    let mut stmt = db.prepare("SELECT id,payload FROM signal_records WHERE kind=?1 AND id>?2 ORDER BY id LIMIT 20000")?;
    let rows = stmt.query_map(params![kind, after], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}
fn log_summary(summaries: &HashMap<u64, Summary>) {
    let mut hs = summaries.keys().copied().collect::<Vec<_>>(); hs.sort_unstable();
    for h in hs { let s=&summaries[&h]; if s.count>0 { eprintln!("PAPER_SUMMARY horizon_seconds={h} resolved={} positive_net={} mean_gross_percent={:.6} mean_net_percent={:.6}",s.count,s.positive,s.gross_sum/s.count as f64,s.net_sum/s.count as f64); } }
}

fn main() -> anyhow::Result<()> {
    let mut args=env::args().skip(1);
    let db_path=PathBuf::from(args.next().expect("usage: cross_asset_paper_test <signals.sqlite> <events.jsonl> [duration_seconds] [poll_seconds]"));
    let output=PathBuf::from(args.next().expect("missing events.jsonl path"));
    let seconds=args.next().and_then(|v|v.parse().ok()).unwrap_or(259200_u64);
    let poll=args.next().and_then(|v|v.parse().ok()).unwrap_or(2_u64).max(1);
    if !db_path.exists(){anyhow::bail!("signal database not found: {}",db_path.display());}
    if let Some(parent)=output.parent(){std::fs::create_dir_all(parent)?;}
    let db=Connection::open_with_flags(&db_path,OpenFlags::SQLITE_OPEN_READ_ONLY)?; db.busy_timeout(Duration::from_secs(5))?;
    let mut writer=BufWriter::new(OpenOptions::new().create(true).append(true).open(&output)?);
    let (mut feature_id,mut evaluation_id)=(0_i64,0_i64);
    let mut last_sample=HashMap::<(String,u64),u64>::new();
    let mut pending=HashMap::<(String,u64,u64),Candidate>::new();
    let mut summaries=HashMap::<u64,Summary>::new();
    let start=Instant::now(); let mut next_summary=Instant::now()+Duration::from_secs(300);
    eprintln!("PAPER_START db={} output={} duration_seconds={} horizons=300,900 strategy=fade_btc_5m_return execution=none",db_path.display(),output.display(),seconds);
    while start.elapsed()<Duration::from_secs(seconds) {
        for (id,payload) in read_new(&db,FEATURES,feature_id)? {
            feature_id=id;
            if let StoredSignalRecord::ResearchFeatures(f)=decode(&payload)? { on_feature(&f,&mut last_sample,&mut pending,&mut writer)?; }
        }
        for (id,payload) in read_new(&db,EVALUATIONS,evaluation_id)? {
            evaluation_id=id;
            if let StoredSignalRecord::Evaluation(e)=decode(&payload)? { on_evaluation(&e,&mut pending,&mut summaries,&mut writer)?; }
        }
        if Instant::now()>=next_summary {log_summary(&summaries);next_summary=Instant::now()+Duration::from_secs(300);}
        thread::sleep(Duration::from_secs(poll));
    }
    log_summary(&summaries);
    append(&mut writer,json!({"event":"run_complete","finished_at_ms":now_ms(),"elapsed_seconds":start.elapsed().as_secs(),"unresolved_candidates":pending.len()}))?;
    eprintln!("PAPER_STOP elapsed_seconds={} unresolved_candidates={}",start.elapsed().as_secs(),pending.len());
    Ok(())
}

fn on_feature(f:&SignalResearchFeatures,last:&mut HashMap<(String,u64),u64>,pending:&mut HashMap<(String,u64,u64),Candidate>,w:&mut BufWriter<File>)->anyhow::Result<()> {
    if f.symbol.starts_with("BTC-")||f.symbol.starts_with("ETH-"){return Ok(());}
    let Some(r)=f.btc_return_5m_percent.filter(|v|v.is_finite()&&v.abs()>1e-10) else{return Ok(());};
    for h in [300_u64,900_u64] {
        let sample=(f.symbol.clone(),h);
        if f.timestamp.saturating_sub(*last.get(&sample).unwrap_or(&0))<h*1000{continue;}
        last.insert(sample,f.timestamp);
        let c=Candidate{symbol:f.symbol.clone(),timestamp:f.timestamp,horizon:h,btc_return_5m:r,side:if r>0.0{"short"}else{"long"}};
        append(w,json!({"event":"candidate","symbol":c.symbol,"signal_timestamp_ms":c.timestamp,"horizon_seconds":c.horizon,"btc_return_5m_percent":c.btc_return_5m,"paper_side":c.side,"strategy":"fade_btc_5m_return","execution":"none_paper_only"}))?;
        pending.insert((c.symbol.clone(),c.timestamp,c.horizon),c);
    }
    Ok(())
}
fn on_evaluation(e:&SignalEvaluation,pending:&mut HashMap<(String,u64,u64),Candidate>,summaries:&mut HashMap<u64,Summary>,w:&mut BufWriter<File>)->anyhow::Result<()> {
    if !matches!(e.horizon_seconds,300|900){return Ok(());}
    let key=(e.symbol.clone(),e.signal_timestamp,e.horizon_seconds);
    let Some(c)=pending.remove(&key) else{return Ok(());};
    let (gross,net)=if c.side=="long"{(e.long_gross_percent,e.long_net_percent)}else{(e.short_gross_percent,e.short_net_percent)};
    append(w,json!({"event":"outcome","symbol":c.symbol,"signal_timestamp_ms":c.timestamp,"evaluated_timestamp_ms":e.evaluated_timestamp,"horizon_seconds":c.horizon,"btc_return_5m_percent":c.btc_return_5m,"paper_side":c.side,"gross_return_percent":gross,"net_return_percent":net,"recorded_execution_cost_percent":gross-net.unwrap_or(gross),"execution":"none_paper_only"}))?;
    if let Some(n)=net.filter(|v|v.is_finite()){let s=summaries.entry(c.horizon).or_default();s.count+=1;s.positive+=u64::from(n>0.0);s.net_sum+=n;s.gross_sum+=gross;}
    Ok(())
}
