use crate::vwap::{NUM_BINS, OrderAction, OrderManager, VwapConfig};
use axum::{Json, Router, extract::{Path, State}, http::StatusCode, routing::{get, post}};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
type Managers = Arc<Mutex<HashMap<String, Arc<Mutex<OrderManager>>>>>;
#[derive(Deserialize)] struct Create { parent_qty:f64, curve:Vec<f64>, expected:Vec<f64>, max_participation_rate:f64, min_trade_threshold:f64, bin_duration_secs:f64 }
#[derive(Deserialize)] struct Market { trade_qty:f64, trade_price:f64, elapsed_bin_secs:f64, target_bin:usize }
#[derive(Deserialize)] struct Fill { fill_price:f64, filled_qty:f64 }
#[derive(Serialize)] #[serde(tag="action",rename_all="snake_case")] enum Action { NewOrder{qty:f64}, ModifyOrder{new_qty:f64}, CancelOrder }
#[derive(Serialize)] struct StateDto { current_bin:usize, executed_qty:f64, remaining_qty:f64, action:Option<Action>, market_vwap:f64, execution_vwap:f64, slippage:f64 }
async fn manager(s:&Managers,id:&str)->Result<Arc<Mutex<OrderManager>>,(StatusCode,String)>{s.lock().await.get(id).cloned().ok_or((StatusCode::NOT_FOUND,"Unknown VWAP manager".into()))}
async fn create(State(s):State<Managers>,Json(r):Json<Create>)->Result<Json<String>,(StatusCode,String)>{
 let curve:[f64;NUM_BINS]=r.curve.try_into().map_err(|_|(StatusCode::BAD_REQUEST,"curve must have 78 weights".to_owned()))?; let expected:[f64;NUM_BINS]=r.expected.try_into().map_err(|_|(StatusCode::BAD_REQUEST,"expected must have 78 volumes".to_owned()))?; let m=OrderManager::new(VwapConfig{parent_qty:r.parent_qty,historical_vwap_curve:curve,max_participation_rate:r.max_participation_rate},expected,r.min_trade_threshold,r.bin_duration_secs).map_err(|e|(StatusCode::BAD_REQUEST,e.to_owned()))?;
 let id=format!("vwap-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
 s.lock().await.insert(id.clone(),Arc::new(Mutex::new(m)));Ok(Json(id))
}
async fn market(State(s):State<Managers>,Path(id):Path<String>,Json(r):Json<Market>)->Result<Json<StateDto>,(StatusCode,String)>{
 let m=manager(&s,&id).await?;let mut m=m.lock().await;
 while m.state().current_bin<r.target_bin.min(NUM_BINS){m.on_bin_transition();}
 let action=m.on_market_trade(r.trade_qty,r.trade_price,r.elapsed_bin_secs).map(|a|match a{OrderAction::NewOrder{qty}=>Action::NewOrder{qty},OrderAction::ModifyOrder{new_qty}=>Action::ModifyOrder{new_qty},OrderAction::CancelOrder=>Action::CancelOrder});
 let x=m.state();let b=m.benchmark();Ok(Json(StateDto{current_bin:x.current_bin,executed_qty:x.executed_qty,remaining_qty:x.remaining_qty,action,market_vwap:b.market_vwap(),execution_vwap:b.execution_vwap(),slippage:b.slippage()}))
}
async fn get_state(State(s):State<Managers>,Path(id):Path<String>)->Result<Json<StateDto>,(StatusCode,String)>{let m=manager(&s,&id).await?;let m=m.lock().await;let x=m.state();let b=m.benchmark();Ok(Json(StateDto{current_bin:x.current_bin,executed_qty:x.executed_qty,remaining_qty:x.remaining_qty,action:None,market_vwap:b.market_vwap(),execution_vwap:b.execution_vwap(),slippage:b.slippage()}))}
async fn fill(State(s):State<Managers>,Path(id):Path<String>,Json(r):Json<Fill>)->Result<StatusCode,(StatusCode,String)>{manager(&s,&id).await?.lock().await.on_execution_fill(r.fill_price,r.filled_qty);Ok(StatusCode::NO_CONTENT)}
async fn complete(State(s):State<Managers>,Path(id):Path<String>)->Result<StatusCode,(StatusCode,String)>{manager(&s,&id).await?.lock().await.on_order_complete();Ok(StatusCode::NO_CONTENT)}
async fn remove(State(s):State<Managers>,Path(id):Path<String>)->StatusCode{if s.lock().await.remove(&id).is_some(){StatusCode::NO_CONTENT}else{StatusCode::NOT_FOUND}}
pub fn router()->Router{let s:Managers=Arc::new(Mutex::new(HashMap::new()));Router::new().route("/internal/vwap/managers",post(create)).route("/internal/vwap/managers/{id}/market",post(market)).route("/internal/vwap/managers/{id}",get(get_state).delete(remove)).route("/internal/vwap/managers/{id}/fill",post(fill)).route("/internal/vwap/managers/{id}/complete",post(complete)).with_state(s)}
